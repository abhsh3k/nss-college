//! Head-of-department exam timetable and semester results.
//!
//! The editor assembles exam rows for a programme and semester and pushes
//! them as a group: students then see the timetable between the push and the
//! date of the last exam. Results are pushed as rows under one of two keys —
//! the student's PRN for the university's semester exam, the admission number
//! for an internal exam the college itself conducts — and publish straight to
//! the student's Results page under the semester picked on the page. Every
//! write re-checks the programme's department.

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::Redirect,
    Form,
};
use serde::Deserialize;
use tower_sessions::Session;

use super::{parse_i32, parse_i64};
use crate::{
    auth::{csrf, Manager},
    error::AppError,
    services::{
        academics::{self, ProgrammeOption},
        exams::{self, ExamCourseOption, ExamEntry, ExamGroup, PushKind, PushProblem},
        users,
    },
    shell::{self, Shell},
    state::AppState,
};

const SEMESTERS: [i32; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

#[derive(Deserialize, Default)]
pub struct PageQuery {
    programme: Option<String>,
    semester: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct ExamForm {
    csrf_token: String,
    programme: String,
    semester: String,
    course_id: String,
    name: String,
    exam_date: String,
    start_time: String,
    end_time: String,
    venue: String,
}

/// Group-level buttons: the whole timetable moves together.
#[derive(Deserialize)]
pub struct GroupForm {
    csrf_token: String,
    programme: String,
    semester: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct ResultsForm {
    csrf_token: String,
    programme: String,
    semester: String,
    /// "university" (keyed by PRN) or "internal" (keyed by admission number).
    kind: String,
    rows: String,
}

#[derive(Deserialize)]
pub struct DeleteForm {
    csrf_token: String,
    programme: String,
    semester: String,
}

#[derive(Template)]
#[template(path = "admin/exams.html")]
pub struct ExamsTemplate {
    shell: Shell,
    programmes: Vec<ProgrammeOption>,
    /// (value, is the selected one) for the semester pickers.
    semester_opts: Vec<(i32, bool, bool)>,
    sel_programme: i64,
    sel_semester: i32,
    courses: Vec<ExamCourseOption>,
    entries: Vec<ExamEntry>,
    group: ExamGroup,
    results_rows: String,
    /// Which push box is selected: university (PRN) or internal (admission no).
    results_kind: String,
    problems: Vec<PushProblem>,
    notice: Option<String>,
}

async fn render(
    s: &AppState,
    session: &Session,
    manager: &Manager,
    programme_id: i64,
    semester: i32,
    results_rows: String,
    results_kind: String,
    problems: Vec<PushProblem>,
    notice: Option<String>,
) -> Result<ExamsTemplate, AppError> {
    let shell = Shell::build(&manager.user, session).await?;
    let programmes = academics::programme_options_for(&s.db, manager.department()).await?;
    let mut page = ExamsTemplate {
        shell,
        programmes,
        semester_opts: SEMESTERS.iter().map(|n| (*n, *n == semester, false)).collect(),
        sel_programme: programme_id,
        sel_semester: semester,
        courses: Vec::new(),
        entries: Vec::new(),
        group: ExamGroup {
            pushed: false,
            exam_count: 0,
            pushed_label: None,
            last_date_label: None,
        },
        results_rows,
        results_kind,
        problems,
        notice,
    };
    if programme_id > 0 {
        page.courses = exams::course_options(&s.db, programme_id).await?;
        page.entries = exams::exams_for(&s.db, programme_id, semester).await?;
        page.group = exams::group_status(&s.db, programme_id, semester).await?;
    }
    Ok(page)
}

/// The selected programme, forced to one this manager may act for (an admin
/// manages every programme; a head only their department's).
async fn selected_programme(
    s: &AppState,
    department: Option<i64>,
    raw: Option<&str>,
) -> Result<i64, AppError> {
    let want = raw.and_then(parse_i64).unwrap_or(0);
    if want > 0 && academics::may_manage_programme(&s.db, want, department).await? {
        return Ok(want);
    }
    Ok(0)
}

pub async fn page(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Query(q): Query<PageQuery>,
) -> Result<ExamsTemplate, AppError> {
    let programme_id = selected_programme(&s, manager.department(), q.programme.as_deref()).await?;
    let semester = q
        .semester
        .as_deref()
        .and_then(parse_i32)
        .filter(|v| (1..=12).contains(v))
        .unwrap_or(1);
    render(
        &s,
        &session,
        &manager,
        programme_id,
        semester,
        String::new(),
        PushKind::University.as_str().to_string(),
        Vec::new(),
        None,
    )
    .await
}

/// A plain `YYYY-MM-DD` calendar date (validated without pulling in a date
/// parser: month lengths and leap years are all it has to get right).
fn valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let num = |slice: &str| slice.parse::<i32>().ok();
    let (Some(year), Some(month), Some(day)) = (
        num(s.get(0..4).unwrap_or("")),
        num(s.get(5..7).unwrap_or("")),
        num(s.get(8..10).unwrap_or("")),
    ) else {
        return false;
    };
    if !(1..=12).contains(&month) || day < 1 {
        return false;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 => 28 + leap as i32,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 30,
    };
    day <= days
}

/// One exam row, still a draft until the whole timetable is pushed.
pub async fn add(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Form(f): Form<ExamForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let back = format!(
        "/admin/exams?programme={}&semester={}",
        f.programme, f.semester
    );
    let flash = |msg: String| async move {
        shell::flash(&session, msg).await?;
        Ok::<Redirect, AppError>(Redirect::to(&back))
    };

    let programme_id = parse_i64(&f.programme).unwrap_or(0);
    let semester = parse_i32(&f.semester).filter(|v| (1..=12).contains(v)).unwrap_or(0);
    if programme_id == 0 || semester == 0 {
        return flash("Choose a programme and semester first.".into()).await;
    }
    if !academics::may_manage_programme(&s.db, programme_id, manager.department()).await? {
        return Err(AppError::Forbidden);
    }
    let name = f.name.trim();
    let course_id = parse_i64(&f.course_id).unwrap_or(0);
    if course_id == 0 || name.is_empty() {
        return flash("Pick a course and give the exam a name.".into()).await;
    }
    if !valid_date(f.exam_date.trim()) {
        return flash("Choose a valid exam date.".into()).await;
    }
    let start = super::normalise_time(f.start_time.trim());
    let end = super::normalise_time(f.end_time.trim());
    let (Some(start), Some(end)) = (start, end) else {
        return flash("Choose the start and end time.".into()).await;
    };
    if end <= start {
        return flash("The exam must end after it starts.".into()).await;
    }

    let id = exams::create_exam(
        &s.db,
        &exams::NewExam {
            programme_id,
            semester,
            course_id,
            name,
            exam_date: f.exam_date.trim(),
            start_time: &start,
            end_time: &end,
            venue: f.venue.trim(),
        },
    )
    .await?;
    users::audit(
        &s.db,
        Some(manager.user.id),
        "exam_created",
        "exam",
        Some(id),
    )
    .await?;
    flash(format!("Added {name} to the timetable (draft).")).await
}

/// Only a draft row can be deleted; a pushed timetable has to be unpublished
/// first, so students never watch a row vanish from under the window.
pub async fn delete(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(f): Form<DeleteForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let back = format!(
        "/admin/exams?programme={}&semester={}",
        f.programme, f.semester
    );
    let programme_id = parse_i64(&f.programme).unwrap_or(0);
    if !academics::may_manage_programme(&s.db, programme_id, manager.department()).await? {
        return Err(AppError::Forbidden);
    }
    if exams::delete_draft(&s.db, id).await? {
        users::audit(&s.db, Some(manager.user.id), "exam_deleted", "exam", Some(id)).await?;
        shell::flash(&session, "Draft exam removed.").await?;
    } else {
        shell::flash(
            &session,
            "That exam is already pushed — unpublish the timetable before removing it.",
        )
        .await?;
    }
    Ok(Redirect::to(&back))
}

pub async fn push(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Form(f): Form<GroupForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let programme_id = parse_i64(&f.programme).unwrap_or(0);
    let semester = parse_i32(&f.semester).filter(|v| (1..=12).contains(v)).unwrap_or(0);
    if programme_id == 0 || semester == 0
        || !academics::may_manage_programme(&s.db, programme_id, manager.department()).await?
    {
        return Err(AppError::Forbidden);
    }
    let count = exams::push_group(&s.db, programme_id, semester).await?;
    users::audit(
        &s.db,
        Some(manager.user.id),
        "exam_timetable_pushed",
        "programme",
        Some(programme_id),
    )
    .await?;
    shell::flash(
        &session,
        format!("Pushed to students: {count} exam(s), visible until the last exam date."),
    )
    .await?;
    Ok(Redirect::to(&format!(
        "/admin/exams?programme={programme_id}&semester={semester}"
    )))
}

pub async fn unpush(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Form(f): Form<GroupForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let programme_id = parse_i64(&f.programme).unwrap_or(0);
    let semester = parse_i32(&f.semester).filter(|v| (1..=12).contains(v)).unwrap_or(0);
    if programme_id == 0 || semester == 0
        || !academics::may_manage_programme(&s.db, programme_id, manager.department()).await?
    {
        return Err(AppError::Forbidden);
    }
    let count = exams::unpush_group(&s.db, programme_id, semester).await?;
    users::audit(
        &s.db,
        Some(manager.user.id),
        "exam_timetable_unpushed",
        "programme",
        Some(programme_id),
    )
    .await?;
    shell::flash(&session, format!("Pulled back {count} exam(s); students can no longer see them.")).await?;
    Ok(Redirect::to(&format!(
        "/admin/exams?programme={programme_id}&semester={semester}"
    )))
}

/// The results paste box — university rows keyed by PRN, internal rows keyed
/// by admission number. Renders in place with per-line problems so nothing
/// the head typed is lost.
pub async fn push_results(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Form(f): Form<ResultsForm>,
) -> Result<ExamsTemplate, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let semester = parse_i32(&f.semester).filter(|v| (1..=12).contains(v)).unwrap_or(1);
    let kind = PushKind::parse(&f.kind);
    // Stay on the timetable that was open, so the outcome renders in place.
    let programme_id = selected_programme(&s, manager.department(), Some(f.programme.as_str())).await?;
    // A head's rows only ever land on their own department's programmes;
    // validate_push checks each student's programme against `department`.
    let department = manager.department();

    let (parsed, mut problems) = exams::parse_push(&f.rows, kind);
    if parsed.is_empty() && problems.is_empty() {
        problems.push(PushProblem {
            line: 0,
            text: String::new(),
            problem: "Paste at least one result row.".into(),
        });
    }
    let (good, more) = exams::validate_push(&s.db, department, kind, &parsed).await?;
    problems.extend(more);

    let filed = if good.is_empty() {
        None
    } else {
        // Filed under the semester picked on the page, not the semester the
        // student happens to have reached.
        let n = exams::push_marks(&s.db, kind, semester, &good).await?;
        users::audit(
            &s.db,
            Some(manager.user.id),
            "exam_results_pushed",
            "student",
            None,
        )
        .await?;
        Some(n)
    };

    let notice = match filed {
        Some(n) if problems.is_empty() => Some(format!(
            "Filed {n} {} result row(s) under semester {semester}.",
            kind.as_str()
        )),
        Some(n) => Some(format!("Filed {n} row(s); the rest need fixing (see below).")),
        None => None,
    };
    render(
        &s,
        &session,
        &manager,
        programme_id,
        semester,
        f.rows,
        kind.as_str().to_string(),
        problems,
        notice,
    )
    .await
}
