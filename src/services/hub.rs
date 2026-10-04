//! Queries for the Student Hub (Layer 4d). Plain `query_as` (not the compile-time
//! macros) so the project builds without a live database.

use sqlx::{FromRow, PgPool};

use crate::models::Notice;

type Res<T> = Result<T, sqlx::Error>;

/// The signed-in student's row, joined with their programme.
#[derive(Debug, FromRow)]
pub struct HubStudent {
    pub id: i64,
    pub admission_no: String,
    pub programme_id: i64,
    pub programme: String,
    pub semester: i32,
    pub egrants: bool,
}

pub async fn student_profile(db: &PgPool, user_id: i64) -> Res<Option<HubStudent>> {
    sqlx::query_as::<_, HubStudent>(
        r#"SELECT st.id,
                  st.admission_no,
                  st.programme_id,
                  p.name AS programme,
                  st.semester,
                  st.egrants
           FROM students st
           JOIN programmes p ON p.id = st.programme_id
           WHERE st.user_id = $1 AND st.is_active"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
}

/// One period of the weekly timetable (weekday: 1 = Monday .. 7 = Sunday).
#[derive(Debug, FromRow)]
pub struct Period {
    pub weekday: i16,
    pub code: String,
    pub title: String,
    pub start_time: String,
    pub end_time: String,
    pub room: String,
    pub faculty: String,
}

pub async fn week_schedule(
    db: &PgPool,
    programme_id: i64,
    semester: i32,
) -> Res<Vec<Period>> {
    sqlx::query_as::<_, Period>(
        r#"SELECT te.weekday,
                  c.code,
                  c.title,
                  to_char(te.start_time, 'HH24:MI') AS start_time,
                  to_char(te.end_time, 'HH24:MI')   AS end_time,
                  COALESCE(te.room, '')   AS room,
                  COALESCE(f.name, '')    AS faculty
           FROM timetable_entries te
           JOIN courses c ON c.id = te.course_id
           LEFT JOIN faculty f ON f.id = te.faculty_id
           WHERE te.programme_id = $1 AND te.semester = $2
           ORDER BY te.weekday, te.start_time"#,
    )
    .bind(programme_id)
    .bind(semester)
    .fetch_all(db)
    .await
}

/// A course the student is currently enrolled in, with its assigned teacher.
#[derive(Debug, FromRow)]
pub struct EnrolledCourse {
    pub course_id: i64,
    pub code: String,
    pub title: String,
    pub credits: i32,
    pub teacher: String,
}

pub async fn enrolled_courses(db: &PgPool, student_id: i64) -> Res<Vec<EnrolledCourse>> {
    sqlx::query_as::<_, EnrolledCourse>(
        r#"SELECT c.id AS course_id,
                  c.code,
                  c.title,
                  c.credits,
                  COALESCE(f.name, '') AS teacher
           FROM enrollments e
           JOIN courses c ON c.id = e.course_id
           LEFT JOIN faculty f ON f.id = c.faculty_id
           WHERE e.student_id = $1 AND e.status = 'active'
           ORDER BY c.semester, c.code"#,
    )
    .bind(student_id)
    .fetch_all(db)
    .await
}

/// Attendance per enrolled course: sessions marked for the course versus the
/// sessions where this student was recorded present or on leave.
#[derive(Debug, FromRow)]
pub struct CourseAttendance {
    pub course_id: i64,
    pub marked: i64,
    pub attended: i64,
}

pub async fn course_attendance(db: &PgPool, student_id: i64) -> Res<Vec<CourseAttendance>> {
    sqlx::query_as::<_, CourseAttendance>(
        r#"SELECT e.course_id,
                  COUNT(s.id) AS marked,
                  COUNT(ar.id) FILTER (WHERE ar.status IN ('present', 'leave')) AS attended
           FROM enrollments e
           LEFT JOIN attendance_sessions s ON s.course_id = e.course_id
           LEFT JOIN attendance_records ar ON ar.session_id = s.id
                                          AND ar.student_id = e.student_id
           WHERE e.student_id = $1 AND e.status = 'active'
           GROUP BY e.course_id"#,
    )
    .bind(student_id)
    .fetch_all(db)
    .await
}

/// Current-month totals across every enrolled course; used for the e-grants check.
#[derive(Debug, FromRow)]
pub struct MonthlyAttendance {
    pub marked: i64,
    pub attended: i64,
    pub month_label: String,
}

pub async fn monthly_attendance(db: &PgPool, student_id: i64) -> Res<MonthlyAttendance> {
    sqlx::query_as::<_, MonthlyAttendance>(
        r#"SELECT COUNT(s.id) AS marked,
                  COUNT(ar.id) FILTER (WHERE ar.status IN ('present', 'leave')) AS attended,
                  to_char(date_trunc('month', CURRENT_DATE), 'FMMonth YYYY') AS month_label
           FROM enrollments e
           JOIN attendance_sessions s ON s.course_id = e.course_id
              AND s.on_date >= date_trunc('month', CURRENT_DATE)
              AND s.on_date <  date_trunc('month', CURRENT_DATE) + interval '1 month'
           LEFT JOIN attendance_records ar ON ar.session_id = s.id
                                          AND ar.student_id = e.student_id
           WHERE e.student_id = $1 AND e.status = 'active'"#,
    )
    .bind(student_id)
    .fetch_one(db)
    .await
}

/// A number from `site_settings`, falling back when the key is missing.
pub async fn setting(db: &PgPool, key: &str, default: i32) -> Res<i32> {
    let value = sqlx::query_scalar::<_, String>(
        "SELECT value FROM site_settings WHERE key = $1",
    )
    .bind(key)
    .fetch_optional(db)
    .await?;
    Ok(value
        .and_then(|v| v.trim().parse::<i32>().ok())
        .unwrap_or(default))
}

/// A yes/no setting such as `attendance_leave_counts_as_present`, which is seeded
/// as the word "false" rather than a number, so it must not go through `setting`.
pub async fn setting_bool(db: &PgPool, key: &str, default: bool) -> Res<bool> {
    let value = sqlx::query_scalar::<_, String>(
        "SELECT value FROM site_settings WHERE key = $1",
    )
    .bind(key)
    .fetch_optional(db)
    .await?;
    Ok(match value {
        None => default,
        Some(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes" | "on"
        ),
    })
}

/// Announcements aimed at students (public ones included), newest first.
pub async fn student_feed(db: &PgPool, limit: i64) -> Res<Vec<Notice>> {
    sqlx::query_as::<_, Notice>(
        r#"SELECT title,
                  category,
                  COALESCE(attachment_path, '/notices') AS href,
                  (published_at > now() - interval '7 days') AS is_new
           FROM notices
           WHERE status = 'published'
             AND audience IN ('public', 'students')
             AND published_at <= now()
           ORDER BY is_pinned DESC, published_at DESC
           LIMIT $1"#,
    )
    .bind(limit)
    .fetch_all(db)
    .await
}
