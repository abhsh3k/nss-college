//! Marks entry: the roster-first grid for filing a course's marks, a CSV
//! import for whole batches of results, and the management list for rows
//! already filed under a semester.
//!
//! Pick a programme, semester and course, see the class with whatever is
//! already filed, type a number per student and save — or upload a CSV of
//! `key, course_code, obtained, max` rows (keyed by PRN for the university
//! sheet, by admission number otherwise). Both ways write the same `marks`
//! rows through `services::marks`, so a sheet saved here reads back on the
//! student's Results page at once and anything filed earlier comes back
//! prefilled in the grid.

use std::collections::{HashMap, HashSet};

use askama::Template;
use axum::{
    extract::{Multipart, Path, Query, State},
    http::header,
    response::{IntoResponse, Redirect, Response},
    Form,
};
use serde::Deserialize;
use tower_sessions::Session;

use super::{parse_i32, parse_i64, safe_back, TokenForm};
use crate::{
    auth::{csrf, require_reauth, Manager},
    error::AppError,
    sections,
    services::{
        academics::{self, ProgrammeOption},
        exams::{self, ExamCourseOption, PushKind},
        marks::{self, CorrectionOutcome, FiledRow, HistoryRow, PendingCorrection},
        notifications,
        result_import,
        users,
    },
    shell::{self, Shell},
    state::AppState,
};

/// A path in the workspace this manager is working in: `/admin` for the IT
/// administrator, `/hod` for a head of department, who signs in on the public
/// listener where `/admin` is not served at all.
fn ws(manager: &Manager, path: &str) -> String {
    format!("{}{path}", sections::base_for(&manager.user))
}

const SEMESTERS: [i32; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

/// The coursework assessments a sheet can be keyed on (the `exam` assessment
/// belongs to the university and internal kinds, which set it themselves).
const COURSEWORKS: [&str; 4] = ["internal", "assignment", "practical", "external"];

const KINDS: [&str; 3] = ["coursework", "internal", "university"];

/// A mark can never exceed NUMERIC(6,2)'s ceiling.
const MAX_MARKS_CEILING: f64 = 9999.99;

// ---------- Selection carried across the page ----------

#[derive(Deserialize, Default)]
pub struct PageQuery {
    programme: Option<String>,
    semester: Option<String>,
    course: Option<String>,
    kind: Option<String>,
    assessment: Option<String>,
    exam_name: Option<String>,
    max: Option<String>,
}

/// What the sheet is editing: where it files, and the marks key every input
/// on the grid writes to.
#[derive(Clone)]
pub struct Sel {
    pub programme: i64,
    pub semester: i32,
    pub course: i64,
    /// "coursework", "internal" or "university".
    pub kind: String,
    /// Coursework only: internal, assignment, practical or external.
    pub assessment: String,
    /// Internal only: the exam's own name.
    pub exam_name: String,
    /// Raw text for the max-marks field, so a half-typed value survives a
    /// failed save.
    pub max: String,
}

impl Sel {
    fn from_query(q: &PageQuery) -> Self {
        Sel {
            programme: parse_i64(q.programme.as_deref().unwrap_or("")).unwrap_or(0),
            semester: parse_i32(q.semester.as_deref().unwrap_or("")).unwrap_or(1),
            course: parse_i64(q.course.as_deref().unwrap_or("")).unwrap_or(0),
            kind: clean_kind(q.kind.as_deref().unwrap_or("")),
            assessment: clean_assessment(q.assessment.as_deref().unwrap_or("")),
            exam_name: q.exam_name.clone().unwrap_or_default().trim().to_string(),
            max: q.max.clone().unwrap_or_default().trim().to_string(),
        }
    }

    /// Same normalisation from a posted save, where every field is a bare
    /// string in the multipart-free urlencoded body.
    fn from_fields<'a>(field: impl Fn(&str) -> &'a str) -> Self {
        Sel {
            programme: parse_i64(field("programme")).unwrap_or(0),
            semester: parse_i32(field("semester")).unwrap_or(0),
            course: parse_i64(field("course")).unwrap_or(0),
            kind: clean_kind(field("kind")),
            assessment: clean_assessment(field("assessment")),
            exam_name: field("exam_name").trim().to_string(),
            max: field("max").trim().to_string(),
        }
    }

    /// The marks key this sheet writes: (assessment, exam_kind, exam_name).
    /// An unnamed internal exam falls back to the same default a pasted or
    /// uploaded row uses, so the two entry paths meet on the same row.
    fn sheet_key(&self) -> (&str, &'static str, String) {
        match self.kind.as_str() {
            "internal" => {
                let name = if self.exam_name.is_empty() {
                    "Internal exam".to_string()
                } else {
                    self.exam_name.clone()
                };
                ("exam", "internal", name)
            }
            "university" => ("exam", "university", String::new()),
            _ => (self.assessment.as_str(), "coursework", String::new()),
        }
    }

    /// The default max-marks shown when nothing is typed yet.
    fn max_or_default(&self) -> &str {
        if self.max.is_empty() {
            "100"
        } else {
            &self.max
        }
    }

    fn max_value(&self) -> Result<f64, String> {
        let raw = self.max_or_default();
        match raw.parse::<f64>() {
            Ok(v) if v > 0.0 && v <= MAX_MARKS_CEILING => Ok(v),
            _ => Err(format!(
                "Max marks must be a number above 0 and at most {MAX_MARKS_CEILING}."
            )),
        }
    }

    /// The query string that brings the browser back to this exact sheet, in
    /// the workspace `base` the reader is working in.
    fn back_url(&self, base: &str) -> String {
        let (assessment, _, exam_name) = self.sheet_key();
        format!(
            "{base}/marks?programme={}&semester={}&course={}&kind={}&assessment={}&exam_name={}&max={}",
            self.programme,
            self.semester,
            self.course,
            qenc(&self.kind),
            qenc(assessment),
            qenc(&exam_name),
            qenc(self.max_or_default()),
        )
    }
}

fn clean_kind(raw: &str) -> String {
    let raw = raw.trim();
    if KINDS.contains(&raw) {
        raw.to_string()
    } else {
        "coursework".to_string()
    }
}

fn clean_assessment(raw: &str) -> String {
    let raw = raw.trim().to_ascii_lowercase();
    if COURSEWORKS.contains(&raw.as_str()) {
        raw
    } else {
        "internal".to_string()
    }
}

/// Percent-encode for a query string, so an exam name with spaces or `&`
/// still comes back as one parameter (and `Redirect::to` gets a valid URI).
fn qenc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---------- Page ----------

/// One row of the grid: the student, and what the input should show (the
/// value just typed when a save came back with problems, else the filed one).
pub struct GridRow {
    pub student_id: i64,
    pub name: String,
    pub admission_no: String,
    pub prn: String,
    pub enrolled: bool,
    pub value: String,
    /// "", "Published" or "Draft" — whether a row already exists for this key.
    pub status: &'static str,
}

#[derive(Template)]
#[template(path = "admin/marks.html")]
pub struct MarksTemplate {
    shell: Shell,
    programmes: Vec<ProgrammeOption>,
    semester_opts: Vec<(i32, bool)>,
    courses: Vec<ExamCourseOption>,
    sel: Sel,
    rows: Vec<GridRow>,
    filed: Vec<FiledRow>,
    filed_more: bool,
    /// Corrections awaiting a second manager's decision.
    pending: Vec<PendingCorrection>,
    /// Recently applied corrections, newest first.
    history: Vec<HistoryRow>,
    problems: Vec<String>,
    notice: Option<String>,
}

#[allow(clippy::too_many_arguments)]
async fn render(
    s: &AppState,
    session: &Session,
    manager: &Manager,
    mut sel: Sel,
    entered: &HashMap<i64, String>,
    problems: Vec<String>,
    notice: Option<String>,
) -> Result<MarksTemplate, AppError> {
    // An HOD only ever sees their own department's programmes; a programme
    // they may not manage collapses the selection, as on the exams page.
    if sel.programme > 0
        && !academics::may_manage_programme(&s.db, sel.programme, manager.department()).await?
    {
        sel.programme = 0;
    }

    let mut courses = Vec::new();
    let mut rows = Vec::new();
    let mut filed = Vec::new();
    let mut filed_more = false;
    let mut pending = Vec::new();
    let mut history = Vec::new();

    if sel.programme > 0 {
        // Only courses taught in the semester on the sheet.
        courses = exams::course_options_for_semester(&s.db, sel.programme, sel.semester).await?;
        // Default to the first course so the grid is usable immediately.
        if !courses.iter().any(|c| c.id == sel.course) {
            sel.course = courses.first().map(|c| c.id).unwrap_or(0);
        }
        if sel.course > 0 {
            let (assessment, exam_kind, exam_name) = sel.sheet_key();
            let roster = marks::roster(
                &s.db,
                sel.programme,
                sel.semester,
                sel.course,
                assessment,
                exam_kind,
                &exam_name,
            )
            .await?;
            rows = roster
                .into_iter()
                .map(|r| {
                    let status = match r.published {
                        Some(true) => "Published",
                        Some(false) => "Draft",
                        None => "",
                    };
                    let value = entered
                        .get(&r.student_id)
                        .cloned()
                        .or_else(|| r.obtained.map(fmt_mark))
                        .unwrap_or_default();
                    GridRow {
                        student_id: r.student_id,
                        name: r.name,
                        admission_no: r.admission_no,
                        prn: r.prn,
                        enrolled: r.enrolled,
                        value,
                        status,
                    }
                })
                .collect();
        }
        let (list, more) = marks::filed(&s.db, sel.programme, sel.semester, 500).await?;
        filed = list;
        filed_more = more;
        pending = marks::pending_corrections(&s.db, sel.programme).await?;
        history = marks::correction_history(&s.db, sel.programme, 20).await?;
    }

    Ok(MarksTemplate {
        shell: Shell::build(&manager.user, session, &s.db).await?,
        programmes: academics::programme_options_for(&s.db, manager.department()).await?,
        semester_opts: SEMESTERS.iter().map(|n| (*n, *n == sel.semester)).collect(),
        courses,
        sel,
        rows,
        filed,
        filed_more,
        pending,
        history,
        problems,
        notice,
    })
}

/// How a mark reads in an input: whole numbers without decimals, fractions
/// without trailing zeros (Rust's Display for f64 already does both).
fn fmt_mark(v: f64) -> String {
    format!("{v}")
}

pub async fn page(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Query(q): Query<PageQuery>,
) -> Result<MarksTemplate, AppError> {
    let sel = Sel::from_query(&q);
    render(&s, &session, &manager, sel, &HashMap::new(), Vec::new(), None).await
}

// ---------- Save the grid ----------

/// One `name=value` pair from the urlencoded body.
fn field<'a>(pairs: &'a [(String, String)], key: &str) -> &'a str {
    pairs.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .unwrap_or("")
}

pub async fn save(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    csrf::verify(&session, field(&pairs, "csrf_token")).await?;

    let sel = Sel::from_fields(|k| field(&pairs, k));

    let choose_first = "Choose a programme, semester and course first.";
    if sel.programme == 0 || sel.semester == 0 {
        shell::flash(&session, choose_first).await?;
        return Ok(Redirect::to(&ws(&manager, "/marks")).into_response());
    }
    if !academics::may_manage_programme(&s.db, sel.programme, manager.department()).await? {
        return Err(AppError::Forbidden);
    }

    // The course must be one the picker offered for this programme and semester.
    let courses = exams::course_options_for_semester(&s.db, sel.programme, sel.semester).await?;
    if !courses.iter().any(|c| c.id == sel.course) {
        shell::flash(&session, choose_first).await?;
        return Ok(Redirect::to(&ws(&manager, "/marks")).into_response());
    }

    let mut problems: Vec<String> = Vec::new();
    let (assessment, exam_kind, exam_name) = sel.sheet_key();

    // Max applies to the whole sheet, so a bad value files nothing.
    let max = match sel.max_value() {
        Ok(v) => Some(v),
        Err(msg) => {
            problems.push(msg);
            None
        }
    };

    // Only students on this roster may be written: a stale or forged id is
    // reported rather than silently filed.
    let roster = marks::roster(
        &s.db,
        sel.programme,
        sel.semester,
        sel.course,
        assessment,
        exam_kind,
        &exam_name,
    )
    .await?;
    let allowed: HashSet<i64> = roster.iter().map(|r| r.student_id).collect();
    let names: HashMap<i64, String> = roster
        .iter()
        .map(|r| (r.student_id, r.name.clone()))
        .collect();

    // What the grid came back with, so nothing typed is lost on re-render.
    let mut entered: HashMap<i64, String> = HashMap::new();
    let mut cells: Vec<marks::Cell> = Vec::new();

    for (key, raw) in &pairs {
        let Some(id) = key
            .strip_prefix("mark_")
            .and_then(|n| n.trim().parse::<i64>().ok())
        else {
            continue;
        };
        let value = raw.trim();
        if value.is_empty() {
            continue; // blank box: leave whatever is filed alone
        }
        if !allowed.contains(&id) {
            problems.push(format!(
                "{} is not on this sheet any more; nothing was filed for them.",
                names
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| format!("Student {id}"))
            ));
            continue;
        }
        entered.insert(id, value.to_string());
        let Some(max) = max else {
            continue; // max itself was rejected; collect the typed values only
        };
        match value.parse::<f64>() {
            Ok(v) if (0.0..=max).contains(&v) => cells.push(marks::Cell {
                student_id: id,
                obtained: v,
            }),
            Ok(v) => problems.push(format!(
                "{}: {v} is outside 0–{}.",
                names[&id],
                fmt_mark(max)
            )),
            Err(_) => problems.push(format!(
                "{}: “{value}” is not a number.",
                names[&id]
            )),
        }
    }

    let mut notice = None;
    if !cells.is_empty() {
        let n = marks::save_sheet(
            &s.db,
            sel.programme,
            sel.semester,
            sel.course,
            assessment,
            exam_kind,
            &exam_name,
            max.unwrap_or(100.0),
            &cells,
        )
        .await?;
        users::audit(
            &s.db,
            Some(manager.user.id),
            "marks_saved",
            "course",
            Some(sel.course),
        )
        .await?;
        notice = Some(format!(
            "Filed {n} mark row(s) under Semester {}.",
            sel.semester
        ));
    } else if problems.is_empty() {
        notice = Some("Nothing to save — every box was empty.".to_string());
    }

    Ok(render(
        &s,
        &session,
        &manager,
        sel,
        &entered,
        problems,
        notice,
    )
    .await?
    .into_response())
}

// ---------- CSV import ----------

/// Stage the CSV import and send the head to the review page, instead of
/// filing rows straight away. The commit is a separate confirmation.
pub async fn import_preview(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let form = crate::uploads::read(multipart).await?;
    csrf::verify(&session, form.field("csrf_token")).await?;

    let sel = Sel::from_fields(|k| form.field(k));
    if sel.programme == 0 || sel.semester == 0 {
        shell::flash(&session, "Choose a programme and semester first.").await?;
        return Ok(Redirect::to(&ws(&manager, "/marks")).into_response());
    }
    if !academics::may_manage_programme(&s.db, sel.programme, manager.department()).await? {
        return Err(AppError::Forbidden);
    }
    let Some(file) = form.file("csv_file") else {
        shell::flash(&session, "Choose a CSV file to upload.").await?;
        return Ok(Redirect::to(&ws(&manager, "/marks")).into_response());
    };
    if file.bytes.len() > 5 * 1024 * 1024 {
        shell::flash(&session, "That CSV file is larger than 5 MB — split it into smaller batches.").await?;
        return Ok(Redirect::to(&ws(&manager, "/marks")).into_response());
    }
    // A CSV/TSV file is decoded as before; a workbook is flattened to CSV first.
    let text = crate::services::import_students::to_csv_text(&file.bytes, &file.filename)
        .map_err(AppError::BadRequest)?;
    let (assessment, exam_kind, exam_name) = sel.sheet_key();
    let id = result_import::stage(
        &s.db,
        manager.user.id,
        manager.department(),
        sel.programme,
        sel.semester,
        exam_kind,
        assessment,
        &exam_name,
        if file.filename.is_empty() { "upload.csv" } else { &file.filename },
        &text,
        &[],
    )
    .await?;
    Ok(Redirect::to(&ws(&manager, &format!("/exams/results/preview?batch={id}"))).into_response())
}

/// One uploaded file of results, shaped
/// `key, course_code, marks_obtained, maxmarks` — the key is a PRN on a
/// university sheet and an admission number on any other. Everything else
/// about a row comes from the sheet the head has open: which assessment, which
/// exam kind, which exam name, and the semester it is filed under. A file can
/// therefore only ever file what the grid above would have filed itself.
pub async fn import_csv(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let form = crate::uploads::read(multipart).await?;
    csrf::verify(&session, form.field("csrf_token")).await?;

    let sel = Sel::from_fields(|k| form.field(k));
    if sel.programme == 0 || sel.semester == 0 {
        shell::flash(&session, "Choose a programme and semester first.").await?;
        return Ok(Redirect::to(&ws(&manager, "/marks")).into_response());
    }
    if !academics::may_manage_programme(&s.db, sel.programme, manager.department()).await? {
        return Err(AppError::Forbidden);
    }
    let semester = sel.semester;

    // What every row of the file becomes, and the key it is matched on: a
    // university sheet reads PRNs, every other sheet reads admission numbers.
    let (assessment, exam_kind, exam_name) = {
        let (a, k, n) = sel.sheet_key();
        (a.to_string(), k, n)
    };
    let kind = if exam_kind == "university" {
        exams::PushKind::University
    } else {
        exams::PushKind::Internal
    };

    let mut problems: Vec<String> = Vec::new();

    let Some(file) = form.file("csv_file") else {
        problems.push("Choose a CSV file to upload.".into());
        return Ok(render(&s, &session, &manager, sel, &HashMap::new(), problems, None)
            .await?
            .into_response());
    };
    if file.bytes.len() > 5 * 1024 * 1024 {
        problems.push(format!(
            "{} is larger than 5 MB — split it into smaller batches.",
            file.filename
        ));
        return Ok(render(&s, &session, &manager, sel, &HashMap::new(), problems, None)
            .await?
            .into_response());
    }
    let csv = match crate::services::import_students::to_csv_text(&file.bytes, &file.filename) {
        Ok(text) => text,
        Err(msg) => {
            problems.push(msg);
            return Ok(render(&s, &session, &manager, sel, &HashMap::new(), problems, None)
                .await?
                .into_response());
        }
    };
    let parsed = match exams::parse_csv(csv.as_bytes(), kind) {
        Ok(rows) => rows,
        Err(msg) => {
            problems.push(msg);
            return Ok(render(&s, &session, &manager, sel, &HashMap::new(), problems, None)
                .await?
                .into_response());
        }
    };

    // The picker only offers this semester's courses, and a file may not file
    // against anything else. One row per student and course, too: a later
    // repeat would silently overwrite the first.
    let offered = exams::course_options_for_semester(&s.db, sel.programme, semester).await?;
    let allowed: HashSet<String> = offered.iter().map(|c| c.code.to_ascii_uppercase()).collect();
    let mut accepted: Vec<exams::PushRow> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    for row in &parsed {
        let line = row.line;
        let echo = format!("{}, {}, {}/{}", row.key, row.course, row.obtained, row.max);
        if !allowed.contains(&row.course.to_ascii_uppercase()) {
            problems.push(format!(
                "Line {line}: {echo} — that course is not taught in Semester {semester}."
            ));
            continue;
        }
        let pair = (
            row.key.trim().to_ascii_uppercase(),
            row.course.trim().to_ascii_uppercase(),
        );
        if !seen.insert(pair) {
            problems.push(format!(
                "Line {line}: {echo} — there is already a row for that student and course."
            ));
            continue;
        }
        let mut row = row.clone();
        row.assessment = assessment.clone();
        row.exam_name = exam_name.clone();
        accepted.push(row);
    }

    let (good, more) = exams::validate_push(&s.db, manager.department(), kind, &accepted).await?;
    problems.extend(
        more.into_iter()
            .map(|p| format!("Line {}: {} — {}", p.line, p.text, p.problem)),
    );

    let notice = if good.is_empty() {
        if problems.is_empty() {
            Some("No result rows in that file — nothing was filed.".to_string())
        } else {
            None
        }
    } else {
        let n = exams::push_marks_as(&s.db, semester, &assessment, exam_kind, &exam_name, &good)
            .await?;
        users::audit(
            &s.db,
            Some(manager.user.id),
            "marks_imported",
            "programme",
            Some(sel.programme),
        )
        .await?;
        Some(if problems.is_empty() {
            format!("Filed {n} mark row(s) under Semester {semester}.")
        } else {
            format!(
                "Filed {n} mark row(s); {} line(s) need fixing (see below).",
                problems.len()
            )
        })
    };

    Ok(render(&s, &session, &manager, sel, &HashMap::new(), problems, notice)
        .await?
        .into_response())
}

/// Stage a course-wise sheet — one line per student, one column per course —
/// through the same review and commit as the one-row-per-result import. The
/// file is expanded into the rows the import already speaks, so the review
/// page, the per-course maximum and the commit are all shared.
pub async fn import_courses_preview(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let form = crate::uploads::read(multipart).await?;
    csrf::verify(&session, form.field("csrf_token")).await?;

    let back = ws(&manager, "/marks");
    let sel = Sel::from_fields(|k| form.field(k));
    if sel.programme == 0 || sel.semester == 0 {
        shell::flash(&session, "Choose a programme and semester first.").await?;
        return Ok(Redirect::to(&back).into_response());
    }
    if !academics::may_manage_programme(&s.db, sel.programme, manager.department()).await? {
        return Err(AppError::Forbidden);
    }
    let Some(file) = form.file("csv_file") else {
        shell::flash(&session, "Choose a CSV file to upload.").await?;
        return Ok(Redirect::to(&back).into_response());
    };
    if file.bytes.len() > 5 * 1024 * 1024 {
        shell::flash(
            &session,
            "That CSV file is larger than 5 MB — split it into smaller batches.",
        )
        .await?;
        return Ok(Redirect::to(&back).into_response());
    }
    let text = match crate::services::import_students::to_csv_text(&file.bytes, &file.filename) {
        Ok(text) => text,
        Err(msg) => {
            shell::flash(&session, msg).await?;
            return Ok(Redirect::to(&back).into_response());
        }
    };

    let (assessment, exam_kind, exam_name) = sel.sheet_key();
    let kind = if exam_kind == "university" {
        PushKind::University
    } else {
        PushKind::Internal
    };
    let sheet = match exams::expand_course_sheet(
        &text,
        kind.key_column(),
        sel.max_value().unwrap_or(100.0),
    ) {
        Ok(sheet) => sheet,
        Err(msg) => {
            shell::flash(&session, msg).await?;
            return Ok(Redirect::to(&back).into_response());
        }
    };

    // Every column must be a course taught in this semester: the same rule the
    // picker offers and the write accepts.
    let offered = exams::course_options_for_semester(&s.db, sel.programme, sel.semester).await?;
    let allowed: HashSet<String> = offered.iter().map(|c| c.code.to_ascii_uppercase()).collect();
    let unknown: Vec<String> = sheet
        .codes
        .iter()
        .filter(|c| !allowed.contains(&c.to_ascii_uppercase()))
        .cloned()
        .collect();
    if !unknown.is_empty() {
        shell::flash(
            &session,
            format!(
                "{} {} not taught in Semester {} — check the column names.",
                unknown.join(", "),
                if unknown.len() == 1 { "is" } else { "are" },
                sel.semester
            ),
        )
        .await?;
        return Ok(Redirect::to(&back).into_response());
    }

    let id = result_import::stage(
        &s.db,
        manager.user.id,
        manager.department(),
        sel.programme,
        sel.semester,
        exam_kind,
        assessment,
        &exam_name,
        if file.filename.is_empty() {
            "coursewise.csv"
        } else {
            &file.filename
        },
        &sheet.text,
        &sheet.problems,
    )
    .await?;
    Ok(Redirect::to(&ws(&manager, &format!("/exams/results/preview?batch={id}"))).into_response())
}

#[derive(Deserialize, Default)]
pub struct TemplateQuery {
    programme: Option<String>,
    semester: Option<String>,
    course: Option<String>,
    kind: Option<String>,
}

/// The course-wise blank form: `prn/admission_no, name, BCA101, BCA102, …`,
/// one line per student of the semester with the mark columns left blank. With
/// `course` set the form covers that one course; without it, every course
/// taught in the semester.
pub async fn template_csv(
    State(s): State<AppState>,
    manager: Manager,
    Query(q): Query<TemplateQuery>,
) -> Result<Response, AppError> {
    let programme = parse_i64(q.programme.as_deref().unwrap_or("")).unwrap_or(0);
    let semester = parse_i32(q.semester.as_deref().unwrap_or("")).unwrap_or(0);
    if programme == 0 || !(1..=12).contains(&semester) {
        return Err(AppError::BadRequest(
            "Choose a programme and semester first.".into(),
        ));
    }
    if !academics::may_manage_programme(&s.db, programme, manager.department()).await? {
        return Err(AppError::Forbidden);
    }

    let kind = PushKind::parse(q.kind.as_deref().unwrap_or(""));
    let offered = exams::course_options_for_semester(&s.db, programme, semester).await?;
    let courses: Vec<String> = match q.course.as_deref().and_then(parse_i64) {
        Some(id) if id > 0 => {
            let Some(course) = offered.iter().find(|c| c.id == id) else {
                return Err(AppError::BadRequest(
                    "That course is not taught in the semester you picked.".into(),
                ));
            };
            vec![course.code.clone()]
        }
        _ => offered.iter().map(|c| c.code.clone()).collect(),
    };
    if courses.is_empty() {
        return Err(AppError::BadRequest(
            "No courses are taught in that semester yet.".into(),
        ));
    }

    let roster =
        marks::form_roster(&s.db, programme, semester, kind == PushKind::University).await?;
    let bytes = marks::course_form_csv(kind.key_column(), &courses, &roster)
        .map_err(|e| crate::error::internal(format!("Could not build the course-wise form: {e}")))?;
    let scope = if courses.len() == 1 {
        courses[0].to_ascii_lowercase()
    } else {
        format!("semester-{semester}")
    };
    let name = format!("marks-{}-{scope}.csv", kind.key_column());
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\"").as_str(),
            ),
        ],
        bytes,
    )
        .into_response())
}

// ---------- Management: publish toggle and delete ----------

/// Guard one management action: the row must exist and belong to a programme
/// this manager may act for.
async fn authorise(
    s: &AppState,
    manager: &Manager,
    mark_id: i64,
) -> Result<i64, AppError> {
    let programme = marks::mark_programme(&s.db, mark_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if !academics::may_manage_programme(&s.db, programme, manager.department()).await? {
        return Err(AppError::Forbidden);
    }
    Ok(programme)
}

/// Refuse an unpublish/delete on a row that is no longer freely editable.
/// Returns the redirect to send back (after a flash) when the row is locked.
async fn locked(
    s: &AppState,
    session: &Session,
    mark_id: i64,
    back: &str,
) -> Result<Option<Redirect>, AppError> {
    let msg = match marks::mark_state(&s.db, mark_id).await?.as_deref() {
        Some("finalized") => "That row is finalized — use the correction workflow.",
        Some("correction_pending") => "A correction is pending on that row.",
        Some("superseded") => "That value has been superseded by a correction.",
        None => "That mark row is already gone.",
        _ => return Ok(None),
    };
    shell::flash(session, msg).await?;
    Ok(Some(Redirect::to(back)))
}

pub async fn toggle(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    authorise(&s, &manager, id).await?;
    let back = safe_back(&f.back, &ws(&manager, "/marks"));

    match locked(&s, &session, id, &back).await? {
        Some(redirect) => return Ok(redirect),
        None => {}
    }

    match marks::toggle_published(&s.db, id).await? {
        Some(true) => {
            users::audit(&s.db, Some(manager.user.id), "mark_published", "mark", Some(id))
                .await?;
            shell::flash(&session, "Published — students can see it on their Results page.")
                .await?;
        }
        Some(false) => {
            users::audit(
                &s.db,
                Some(manager.user.id),
                "mark_unpublished",
                "mark",
                Some(id),
            )
            .await?;
            shell::flash(&session, "Unpublished — hidden from students again.").await?;
        }
        None => shell::flash(&session, "That mark row is already gone.").await?,
    }
    Ok(Redirect::to(&back))
}

pub async fn delete(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    authorise(&s, &manager, id).await?;
    let back = safe_back(&f.back, &ws(&manager, "/marks"));

    match locked(&s, &session, id, &back).await? {
        Some(redirect) => return Ok(redirect),
        None => {}
    }

    if marks::delete_mark(&s.db, id).await?.is_some() {
        users::audit(&s.db, Some(manager.user.id), "mark_deleted", "mark", Some(id)).await?;
        shell::flash(&session, "Mark row deleted.").await?;
    } else {
        shell::flash(&session, "That mark row is already gone.").await?;
    }
    Ok(Redirect::to(&back))
}

// ---------- Finalization and corrections ----------

/// Lock every published mark for the open course and semester.
#[derive(Deserialize)]
pub struct FinalizeForm {
    csrf_token: String,
    programme: String,
    semester: String,
    course: String,
    back: String,
}

pub async fn finalize(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Form(f): Form<FinalizeForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    if let Err(redirect) = require_reauth(&session, &ws(&manager, "/marks")).await {
        shell::flash(&session, "Re-enter your password to continue.").await?;
        return Ok(redirect);
    }
    let back = safe_back(&f.back, &ws(&manager, "/marks"));
    let programme = parse_i64(&f.programme).unwrap_or(0);
    let semester = parse_i32(&f.semester).unwrap_or(0);
    let course = parse_i64(&f.course).unwrap_or(0);
    if programme == 0 || semester == 0 || course == 0 {
        shell::flash(&session, "Open a course's sheet first, then finalize it.").await?;
        return Ok(Redirect::to(&back));
    }
    if !academics::may_manage_programme(&s.db, programme, manager.department()).await? {
        return Err(AppError::Forbidden);
    }
    // The course must be one offered for this programme and semester.
    if !exams::course_options_for_semester(&s.db, programme, semester)
        .await?
        .iter()
        .any(|c| c.id == course)
    {
        shell::flash(&session, "That course is not on this semester's sheet.").await?;
        return Ok(Redirect::to(&back));
    }

    let n = marks::finalize_course(&s.db, programme, semester, course, manager.user.id).await?;
    users::audit(
        &s.db,
        Some(manager.user.id),
        "marks_finalized",
        "course",
        Some(course),
    )
    .await?;
    shell::flash(
        &session,
        if n == 0 {
            "Nothing to finalize — no published rows for that course.".to_string()
        } else {
            format!("Finalized {n} mark row(s); further changes need the correction workflow.")
        },
    )
    .await?;
    Ok(Redirect::to(&back))
}

#[derive(Deserialize)]
pub struct CorrectForm {
    csrf_token: String,
    obtained: String,
    max: String,
    reason: String,
    back: String,
}

pub async fn correct(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(f): Form<CorrectForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    if let Err(redirect) = require_reauth(&session, &ws(&manager, "/marks")).await {
        shell::flash(&session, "Re-enter your password to continue.").await?;
        return Ok(redirect);
    }
    authorise(&s, &manager, id).await?;
    let back = safe_back(&f.back, &ws(&manager, "/marks"));

    let reason = f.reason.trim();
    if reason.is_empty() {
        shell::flash(&session, "A correction needs a reason.").await?;
        return Ok(Redirect::to(&back));
    }
    if reason.len() > 500 {
        shell::flash(&session, "Keep the correction reason under 500 characters.").await?;
        return Ok(Redirect::to(&back));
    }
    let obtained = match f.obtained.trim().parse::<f64>() {
        Ok(v) => v,
        Err(_) => {
            shell::flash(&session, "Enter the corrected mark as a number.").await?;
            return Ok(Redirect::to(&back));
        }
    };
    let max = match f.max.trim() {
        "" => None,
        raw => match raw.parse::<f64>() {
            Ok(v) if v > 0.0 && v <= MAX_MARKS_CEILING => Some(v),
            _ => {
                shell::flash(&session, "The corrected maximum must be a number above 0.").await?;
                return Ok(Redirect::to(&back));
            }
        },
    };

    let msg = match marks::request_correction(&s.db, id, manager.user.id, obtained, max, reason).await? {
        CorrectionOutcome::Done(_) => {
            users::audit_detail(
                &s.db,
                Some(manager.user.id),
                "mark_correction_requested",
                "mark",
                Some(id),
                serde_json::json!({ "reason": reason, "obtained": obtained, "max": max }),
            )
            .await?;
            notifications::notify_roles(
                &s.db,
                &["admin", "principal"],
                "correction",
                "A result correction needs approval",
                "A finalized mark was proposed for correction; it needs a second manager.",
                "/admin/marks",
            )
            .await?;
            "Correction requested — a second manager must approve it."
        }
        CorrectionOutcome::NotFound => "That mark row is already gone.",
        CorrectionOutcome::NotFinalized => "Only a finalized mark needs the correction workflow.",
        CorrectionOutcome::AlreadyPending => "A correction for that mark is already waiting.",
        CorrectionOutcome::OutOfRange => "The corrected mark is outside 0 to the maximum.",
        _ => "That correction could not be requested.",
    };
    shell::flash(&session, msg).await?;
    Ok(Redirect::to(&back))
}

#[derive(Deserialize)]
pub struct DecideForm {
    csrf_token: String,
    back: String,
}

async fn decide(
    s: &AppState,
    session: &Session,
    manager: &Manager,
    id: i64,
    f: &DecideForm,
    approve: bool,
) -> Result<Redirect, AppError> {
    csrf::verify(session, &f.csrf_token).await?;
    if let Err(redirect) = require_reauth(session, &ws(&manager, "/marks")).await {
        shell::flash(session, "Re-enter your password to continue.").await?;
        return Ok(redirect);
    }
    authorise(s, manager, id).await?;
    let back = safe_back(&f.back, &ws(&manager, "/marks"));

    let msg = match marks::decide_correction(&s.db, id, manager.user.id, approve).await? {
        CorrectionOutcome::Done(_) => {
            users::audit_detail(
                &s.db,
                Some(manager.user.id),
                if approve {
                    "mark_correction_applied"
                } else {
                    "mark_correction_rejected"
                },
                "mark",
                Some(id),
                serde_json::json!({ "approved": approve }),
            )
            .await?;
            if approve {
                "Correction applied — the previous value is kept in the history."
            } else {
                "Correction rejected; the finalized mark is unchanged."
            }
        }
        CorrectionOutcome::SameActor => {
            "A correction must be approved by someone other than the requester."
        }
        CorrectionOutcome::NotPending => "There is no pending correction on that mark.",
        CorrectionOutcome::NotFound => "That mark row is already gone.",
        _ => "That correction could not be decided.",
    };
    shell::flash(session, msg).await?;
    Ok(Redirect::to(&back))
}

pub async fn approve(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(f): Form<DecideForm>,
) -> Result<Redirect, AppError> {
    decide(&s, &session, &manager, id, &f, true).await
}

pub async fn reject(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(f): Form<DecideForm>,
) -> Result<Redirect, AppError> {
    decide(&s, &session, &manager, id, &f, false).await
}
