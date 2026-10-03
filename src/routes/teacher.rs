use axum::{
    extract::{Path, State, Extension},
    response::{Html, IntoResponse},
    Form,
};
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use crate::{state::AppState, error::AppError, auth::AuthenticatedUser};

#[derive(Deserialize)]
pub struct AttendanceToggleInput {
    pub student_id: i64,
    pub status: String, // "present", "absent", or "leave"
}

/// GET /dashboard/teacher
/// Renders today's scheduled classes and any substitute classes assigned for today.
pub async fn teacher_dashboard(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Result<impl IntoResponse, AppError> {
    let today = Utc::now().naive_utc().date();

    // Fetch faculty ID for the logged-in user
    let faculty = sqlx::query!(
        "SELECT id FROM faculty WHERE user_id = $1",
        user.id
    )
    .fetch_one(&state.db)
    .await?;

    // Query today's classes: regular scheduled entries OR active substitutions
    let classes = sqlx::query!(
        r#"
        SELECT 
            te.id AS timetable_entry_id,
            c.id AS course_id,
            c.code AS course_code,
            c.name AS course_name,
            te.period_number,
            te.room,
            COALESCE(s.id IS NOT NULL, false) AS is_substitution,
            COALESCE(att.id IS NOT NULL, false) AS is_marked
        FROM timetable_entries te
        JOIN courses c ON c.id = te.course_id
        LEFT JOIN substitutions s 
               ON s.timetable_entry_id = te.id 
              AND s.on_date = $2
        LEFT JOIN attendance_sessions att 
               ON att.timetable_entry_id = te.id 
              AND att.on_date = $2
        WHERE (te.faculty_id = $1 AND s.id IS NULL) 
           OR s.substitute_faculty_id = $1
        ORDER BY te.period_number ASC
        "#,
        faculty.id,
        today
    )
    .fetch_all(&state.db)
    .await?;

    let template = state.templates.render("dashboard/teacher.html", &serde_json::json!({
        "user": user,
        "today": today,
        "classes": classes,
    }))?;

    Ok(Html(template))
}

/// GET /dashboard/teacher/session/:entry_id/attendance
/// Retrieves or creates an attendance_session and returns the roster via HTMX.
pub async fn get_attendance_sheet(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(entry_id): i64,
) -> Result<impl IntoResponse, AppError> {
    let today = Utc::now().naive_utc().date();

    let faculty = sqlx::query!(
        "SELECT id FROM faculty WHERE user_id = $1",
        user.id
    )
    .fetch_one(&state.db)
    .await?;

    // Get course ID for timetable entry
    let entry = sqlx::query!(
        "SELECT course_id FROM timetable_entries WHERE id = $1",
        entry_id
    )
    .fetch_one(&state.db)
    .await?;

    // Upsert attendance_session for today
    let session = sqlx::query!(
        r#"
        INSERT INTO attendance_sessions (timetable_entry_id, course_id, on_date, taught_by, marked_by)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (timetable_entry_id, on_date) 
        DO UPDATE SET updated_at = now()
        RETURNING id
        "#,
        entry_id,
        entry.course_id,
        today,
        faculty.id,
        user.id
    )
    .fetch_one(&state.db)
    .await?;

    // Fetch enrolled students and their existing status in this session
    let students = sqlx::query!(
        r#"
        SELECT 
            st.id AS student_id,
            u.full_name,
            st.roll_number,
            COALESCE(ar.status, 'present') AS "status!"
        FROM enrollments e
        JOIN students st ON st.id = e.student_id
        JOIN users u ON u.id = st.user_id
        LEFT JOIN attendance_records ar 
               ON ar.session_id = $1 
              AND ar.student_id = st.id
        WHERE e.course_id = $2 AND e.status = 'active'
        ORDER BY st.roll_number ASC
        "#,
        session.id,
        entry.course_id
    )
    .fetch_all(&state.db)
    .await?;

    let template = state.templates.render("partials/attendance_sheet.html", &serde_json::json!({
        "session_id": session.id,
        "entry_id": entry_id,
        "on_date": today,
        "students": students,
    }))?;

    Ok(Html(template))
}

/// POST /dashboard/teacher/session/:session_id/toggle
/// HTMX endpoint to mark or update individual student attendance status.
pub async fn toggle_attendance_status(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(session_id): i64,
    Form(payload): Form<AttendanceToggleInput>,
) -> Result<impl IntoResponse, AppError> {
    // Read attendance edit window setting (defaulting to 5 days)
    let window_setting = sqlx::query!(
        "SELECT value FROM site_settings WHERE key = 'attendance_edit_window_days'"
    )
    .fetch_optional(&state.db)
    .await?
    .map(|r| r.value.parse::<i64>().unwrap_or(5))
    .unwrap_or(5);

    // Verify session date falls within allowed edit window
    let session = sqlx::query!(
        "SELECT on_date FROM attendance_sessions WHERE id = $1",
        session_id
    )
    .fetch_one(&state.db)
    .await?;

    let today = Utc::now().naive_utc().date();
    let days_diff = (today - session.on_date).num_days();

    if days_diff < 0 || days_diff > window_setting {
        return Err(AppError::Forbidden("Attendance edit window has closed.".into()));
    }

    // Upsert student attendance record
    sqlx::query!(
        r#"
        INSERT INTO attendance_records (session_id, student_id, status, updated_by)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (session_id, student_id)
        DO UPDATE SET status = EXCLUDED.status, updated_by = EXCLUDED.updated_by, updated_at = now()
        "#,
        session_id,
        payload.student_id,
        payload.status,
        user.id
    )
    .execute(&state.db)
    .await?;

    // Render updated badge pill snippet
    let template = state.templates.render("partials/attendance_badge.html", &serde_json::json!({
        "session_id": session_id,
        "student_id": payload.student_id,
        "status": payload.status,
    }))?;

    Ok(Html(template))
}