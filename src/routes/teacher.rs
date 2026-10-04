use askama::Template;
use axum::{
    extract::{Path, State},
    Form,
};
use serde::Deserialize;
use sqlx::{types::time::OffsetDateTime, FromRow};
use tower_sessions::Session;

use crate::{
    auth::{csrf, TeacherOnly},
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
    pub csrf_token: String,
    pub students: Vec<StudentAttendanceItem>,
}

/// One roster row: the student's current status, defaulting to present.
#[derive(FromRow)]
struct RosterRow {
    student_id: i64,
    full_name: String,
    roll_number: String,
    status: String,
}

pub async fn get_attendance_sheet(
    State(s): State<AppState>,session: Session,
    TeacherOnly(user): TeacherOnly,
    Path(entry_id): Path<i64>,
) -> Result<AttendanceSheetTemplate, AppError> {
    let today = OffsetDateTime::now_utc().date();
    let today_str = today.to_string();

    let faculty_id: i64 = sqlx::query_scalar("SELECT id FROM faculty WHERE user_id = $1")
        .bind(user.id)
        .fetch_one(&s.db)
        .await?;

    let course_id: i64 =
        sqlx::query_scalar("SELECT course_id FROM timetable_entries WHERE id = $1")
            .bind(entry_id)
            .fetch_one(&s.db)
            .await?;

    // Upsert the session
    let session_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO attendance_sessions (timetable_entry_id, course_id, on_date, taught_by, marked_by)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (timetable_entry_id, on_date)
        DO UPDATE SET updated_at = now()
        RETURNING id
        "#,
    )
    .bind(entry_id)
    .bind(course_id)
    .bind(today)
    .bind(faculty_id)
    .bind(user.id)
    .fetch_one(&s.db)
    .await?;

    // Get the roster with current statuses (using st.admission_no as the roll number)
    let rows = sqlx::query_as::<_, RosterRow>(
        r#"
        SELECT
            st.id AS student_id,
            u.full_name,
            st.admission_no AS roll_number,
            COALESCE(ar.status, 'present') AS status
        FROM enrollments e
        JOIN students st ON st.id = e.student_id
        JOIN users u ON u.id = st.user_id
        LEFT JOIN attendance_records ar
               ON ar.session_id = $1
              AND ar.student_id = st.id
        WHERE e.course_id = $2 AND e.status = 'active'
        ORDER BY st.admission_no ASC
        "#,
    )
    .bind(session_id)
    .bind(course_id)
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
        csrf_token: csrf::token(&session).await?,
        students,
    })
}

#[derive(Deserialize)]
pub struct AttendanceToggleInput {
    pub student_id: i64,
    pub status: String,
    pub csrf_token: String,
}

#[derive(Template)]
#[template(path = "partials/attendance_badge.html")]
pub struct AttendanceBadgeTemplate {
    pub session_id: i64,
    pub student_id: i64,
    pub status: String,
    pub csrf_token: String,
}

pub async fn toggle_attendance_status(
    State(s): State<AppState>,
    session: Session,
    TeacherOnly(user): TeacherOnly,
    Path(session_id): Path<i64>,
    Form(payload): Form<AttendanceToggleInput>,
) -> Result<AttendanceBadgeTemplate, AppError> {
    csrf::verify(&session, &payload.csrf_token).await?;

    if !matches!(payload.status.as_str(), "present" | "absent" | "leave") {
        return Err(AppError::Forbidden);
    }

    let window_setting = sqlx::query_scalar::<_, String>(
        "SELECT value FROM site_settings WHERE key = 'attendance_edit_window_days'",
    )
    .fetch_optional(&s.db)
    .await?
    .and_then(|v| v.trim().parse::<i32>().ok())
    .unwrap_or(5);

    // Use Postgres to reliably calculate the date difference
    let is_valid = sqlx::query_scalar::<_, bool>(
        "SELECT (CURRENT_DATE - s.on_date) <= $2 FROM attendance_sessions s WHERE s.id = $1",
    )
    .bind(session_id)
    .bind(window_setting)
    .fetch_one(&s.db)
    .await?;

    if !is_valid {
        return Err(AppError::Forbidden);
    }

    sqlx::query(
        r#"
        INSERT INTO attendance_records (session_id, student_id, status, updated_by)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (session_id, student_id)
        DO UPDATE SET status = EXCLUDED.status, updated_by = EXCLUDED.updated_by, updated_at = now()
        "#,
    )
    .bind(session_id)
    .bind(payload.student_id)
    .bind(&payload.status)
    .bind(user.id)
    .execute(&s.db)
    .await?;

    Ok(AttendanceBadgeTemplate {
        session_id,
        student_id: payload.student_id,
        status: payload.status,
        csrf_token: payload.csrf_token,
    })
}