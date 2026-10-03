use askama::Template;
use axum::{
    extract::{Path, State},
    Form,
};
use serde::Deserialize;
use sqlx::types::time::OffsetDateTime;

use crate::{
    auth::TeacherOnly,
    error::AppError,
    state::AppState,
};

pub struct StudentAttendanceItem {
    pub student_id: i64,
    pub full_name: String,
    pub roll_number: String,
    pub status: String,
}

#[derive(Template)]
#[template(path = "partials/attendance_sheet.html")]
pub struct AttendanceSheetTemplate {
    pub session_id: i64,
    pub on_date_str: String,
    pub students: Vec<StudentAttendanceItem>,
}

pub async fn get_attendance_sheet(
    State(s): State<AppState>,
    TeacherOnly(user): TeacherOnly,
    Path(entry_id): Path<i64>,
) -> Result<AttendanceSheetTemplate, AppError> {
    let today = OffsetDateTime::now_utc().date();
    let today_str = today.to_string();

    let faculty = sqlx::query!("SELECT id FROM faculty WHERE user_id = $1", user.id)
        .fetch_one(&s.db)
        .await?;

    let entry = sqlx::query!("SELECT course_id FROM timetable_entries WHERE id = $1", entry_id)
        .fetch_one(&s.db)
        .await?;

    // Upsert the session
    let session_id = sqlx::query!(
        r#"
        INSERT INTO attendance_sessions (timetable_entry_id, course_id, on_date, taught_by, marked_by)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (timetable_entry_id, on_date) 
        DO UPDATE SET updated_at = now()
        RETURNING id
        "#,
        entry_id, entry.course_id, today, faculty.id, user.id
    )
    .fetch_one(&s.db)
    .await?
    .id;

    // Get the roster with current statuses (using st.admission_no as the roll number)
    let rows = sqlx::query!(
        r#"
        SELECT 
            st.id AS student_id,
            u.full_name,
            st.admission_no AS roll_number,
            COALESCE(ar.status, 'present') AS "status!"
        FROM enrollments e
        JOIN students st ON st.id = e.student_id
        JOIN users u ON u.id = st.user_id
        LEFT JOIN attendance_records ar 
               ON ar.session_id = $1 
              AND ar.student_id = st.id
        WHERE e.course_id = $2 AND e.status = 'active'
        ORDER BY st.admission_no ASC
        "#,
        session_id, entry.course_id
    )
    .fetch_all(&s.db)
    .await?;

    let students = rows.into_iter().map(|r| StudentAttendanceItem {
        student_id: r.student_id,
        full_name: r.full_name,
        roll_number: r.roll_number,
        status: r.status,
    }).collect();

    Ok(AttendanceSheetTemplate {
        session_id,
        on_date_str: today_str,
        students,
    })
}

#[derive(Deserialize)]
pub struct AttendanceToggleInput {
    pub student_id: i64,
    pub status: String,
}

#[derive(Template)]
#[template(path = "partials/attendance_badge.html")]
pub struct AttendanceBadgeTemplate {
    pub session_id: i64,
    pub student_id: i64,
    pub status: String,
}

pub async fn toggle_attendance_status(
    State(s): State<AppState>,
    TeacherOnly(user): TeacherOnly,
    Path(session_id): Path<i64>,
    Form(payload): Form<AttendanceToggleInput>,
) -> Result<AttendanceBadgeTemplate, AppError> {
    let window_setting = sqlx::query!("SELECT value FROM site_settings WHERE key = 'attendance_edit_window_days'")
        .fetch_optional(&s.db)
        .await?
        .map(|r| r.value.parse::<i32>().unwrap_or(5))
        .unwrap_or(5);

    let session = sqlx::query!("SELECT on_date FROM attendance_sessions WHERE id = $1", session_id)
        .fetch_one(&s.db)
        .await?;

    // Use Postgres to reliably calculate the date difference
    let is_valid = sqlx::query!(
        "SELECT (CURRENT_DATE - $1) <= $2 AS valid", 
        session.on_date, window_setting
    )
    .fetch_one(&s.db)
    .await?
    .valid
    .unwrap_or(false);

    if !is_valid {
        return Err(AppError::Forbidden);
    }

    sqlx::query!(
        r#"
        INSERT INTO attendance_records (session_id, student_id, status, updated_by)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (session_id, student_id)
        DO UPDATE SET status = EXCLUDED.status, updated_by = EXCLUDED.updated_by, updated_at = now()
        "#,
        session_id, payload.student_id, payload.status, user.id
    )
    .execute(&s.db)
    .await?;

    Ok(AttendanceBadgeTemplate {
        session_id,
        student_id: payload.student_id,
        status: payload.status,
    })
}