//! Attendance reporting for the IT administrator: one row per student and one row
//! per session over a chosen period, with a CSV export of the same figures.

use askama::Template;
use axum::{
    extract::{Query, State},
    http::header,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use tower_sessions::Session;

use super::parse_i64;
use crate::{
    auth::AdminOnly,
    error::AppError,
    services::{
        academics::{self, ProgrammeOption, TeacherOption},
        hub,
        reports::{self, Filter, ReportTotals, SessionSummary, StudentRow},
    },
    shell::Shell,
    state::AppState,
};

#[derive(Deserialize)]
pub struct ReportQuery {
    from: Option<String>,
    to: Option<String>,
    programme: Option<String>,
    course: Option<String>,
    teacher: Option<String>,
}

/// A date from the query string, but only when it is a real `YYYY-MM-DD` date.
fn date_or(value: Option<&String>, fallback: &str) -> String {
    match value.map(|v| v.trim()) {
        Some(v) if super::is_valid_date(v) => v.to_string(),
        _ => fallback.to_string(),
    }
}

async fn read_filter(db: &sqlx::PgPool, q: &ReportQuery) -> Result<Filter, AppError> {
    let default_to = crate::services::attendance::today(db).await?;
    let default_from = format!("{}-01", &default_to[..7]);
    let from = date_or(q.from.as_ref(), &default_from);
    let to = date_or(q.to.as_ref(), &default_to);

    // ISO dates compare correctly as strings, so a period typed backwards
    // would silently return nothing: swap it instead.
    let (from, to) = if from > to { (to, from) } else { (from, to) };

    Ok(Filter {
        from,
        to,
        programme_id: q.programme.as_deref().and_then(parse_i64).unwrap_or(0).max(0),
        course_id: q.course.as_deref().and_then(parse_i64).unwrap_or(0).max(0),
        faculty_id: q.teacher.as_deref().and_then(parse_i64).unwrap_or(0).max(0),
    })
}

#[derive(Template)]
#[template(path = "admin/attendance.html")]
pub struct ReportPage {
    shell: Shell,
    rows: Vec<StudentRow>,
    sessions: Vec<SessionSummary>,
    totals: ReportTotals,
    from_value: String,
    to_value: String,
    programme_id: i64,
    course_id: i64,
    faculty_id: i64,
    programmes: Vec<ProgrammeOption>,
    courses: Vec<reports::CourseOption>,
    teachers: Vec<TeacherOption>,
    low_percent: i32,
    leave_counts: bool,
}

/// Everything the page and the CSV export share, so both show the same numbers.
async fn gather(
    s: &AppState,
    f: &Filter,
) -> Result<(Vec<StudentRow>, Vec<SessionSummary>, ReportTotals, i32, bool), AppError> {
    // Whether "leave" counts towards the percentage is a college-wide setting.
    let leave_counts = hub::setting_bool(&s.db, "attendance_leave_counts_as_present", false).await?;
    let low_percent = hub::setting(
        &s.db,
        "attendance_min_percent",
        reports::DEFAULT_LOW_PERCENT as i32,
    )
    .await?;

    let students = reports::by_student(&s.db, f).await?;
    let rows = reports::decorate(students, leave_counts, low_percent as f64);
    let sessions = reports::by_session(&s.db, f).await?;
    let totals = reports::totals(&s.db, f).await?;
    Ok((rows, sessions, totals, low_percent, leave_counts))
}

pub async fn report(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Query(q): Query<ReportQuery>,
) -> Result<ReportPage, AppError> {
    let filter = read_filter(&s.db, &q).await?;
    let (rows, sessions, totals, low_percent, leave_counts) = gather(&s, &filter).await?;

    Ok(ReportPage {
        shell: Shell::build(&user, &session).await?,
        from_value: filter.from.clone(),
        to_value: filter.to.clone(),
        programme_id: filter.programme_id,
        course_id: filter.course_id,
        faculty_id: filter.faculty_id,
        rows,
        sessions,
        totals,
        programmes: academics::programme_options(&s.db).await?,
        courses: reports::course_options(&s.db, filter.programme_id).await?,
        teachers: academics::teacher_options(&s.db).await?,
        low_percent,
        leave_counts,
    })
}

pub async fn csv(
    State(s): State<AppState>,
    AdminOnly(_user): AdminOnly,
    Query(q): Query<ReportQuery>,
) -> Result<Response, AppError> {
    let filter = read_filter(&s.db, &q).await?;
    let (rows, _sessions, _totals, _low, _leave) = gather(&s, &filter).await?;

    let bytes = reports::students_csv(&rows).map_err(|e| {
        tracing::error!(error = ?e, "could not build the attendance CSV");
        AppError::Internal("Could not build the CSV.".into())
    })?;
    let name = format!("attendance-{}_to_{}.csv", filter.from, filter.to);

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
