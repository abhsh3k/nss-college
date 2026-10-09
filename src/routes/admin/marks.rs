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
    response::{IntoResponse, Redirect, Response},
    Form,
};
use serde::Deserialize;
use tower_sessions::Session;

use super::{parse_i32, parse_i64, safe_back, TokenForm};
use crate::{
    auth::{csrf, Manager},
    error::AppError,
    services::{
        academics::{self, ProgrammeOption},
        exams::{self, ExamCourseOption},
        marks::{self, FiledRow},
        users,
    },
    shell::{self, Shell},
    state::AppState,
};

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

    /// The query string that brings the browser back to this exact sheet.
    fn back_url(&self) -> String {
        let (assessment, _, exam_name) = self.sheet_key();
        format!(
            "/admin/marks?programme={}&semester={}&course={}&kind={}&assessment={}&exam_name={}&max={}",
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
    }

    Ok(MarksTemplate {
        shell: Shell::build(&manager.user, session).await?,
        programmes: academics::programme_options_for(&s.db, manager.department()).await?,
        semester_opts: SEMESTERS.iter().map(|n| (*n, *n == sel.semester)).collect(),
        courses,
        sel,
        rows,
        filed,
        filed_more,
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
        return Ok(Redirect::to("/admin/marks").into_response());
    }
    if !academics::may_manage_programme(&s.db, sel.programme, manager.department()).await? {
        return Err(AppError::Forbidden);
    }

    // The course must be one the picker offered for this programme and semester.
    let courses = exams::course_options_for_semester(&s.db, sel.programme, sel.semester).await?;
    if !courses.iter().any(|c| c.id == sel.course) {
        shell::flash(&session, choose_first).await?;
        return Ok(Redirect::to("/admin/marks").into_response());
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
        return Ok(Redirect::to("/admin/marks").into_response());
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
    let parsed = match exams::parse_csv(&file.bytes, kind) {
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

pub async fn toggle(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    authorise(&s, &manager, id).await?;
    let back = safe_back(&f.back, "/admin/marks");

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
    let back = safe_back(&f.back, "/admin/marks");

    if marks::delete_mark(&s.db, id).await?.is_some() {
        users::audit(&s.db, Some(manager.user.id), "mark_deleted", "mark", Some(id)).await?;
        shell::flash(&session, "Mark row deleted.").await?;
    } else {
        shell::flash(&session, "That mark row is already gone.").await?;
    }
    Ok(Redirect::to(&back))
}
