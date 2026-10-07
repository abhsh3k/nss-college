//! Attendance: periods, rosters, saving, reports and substitutions.
//! "Today" is always the college's local date (India), never the server's UTC date.

use sqlx::{FromRow, PgPool};

type Res<T> = Result<T, sqlx::Error>;

const TODAY: &str = "(now() AT TIME ZONE 'Asia/Kolkata')::date";

// ---------- Rules ----------

#[derive(Debug, Clone)]
pub struct Rules {
    pub min_percent: i32,
    pub edit_window_days: i32,
    pub leave_counts: bool,
}

pub async fn rules(db: &PgPool) -> Res<Rules> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT key, value FROM site_settings WHERE key LIKE 'attendance\\_%'")
            .fetch_all(db)
            .await?;
    let get = |k: &str| rows.iter().find(|(key, _)| key == k).map(|(_, v)| v.trim().to_string());
    let num = |k: &str, default: i32| get(k).and_then(|v| v.parse().ok()).unwrap_or(default);
    Ok(Rules {
        min_percent: num("attendance_min_percent", 75),
        edit_window_days: num("attendance_edit_window_days", 5),
        leave_counts: get("attendance_leave_counts_as_present").map(|v| v == "true").unwrap_or(false),
    })
}

/// Today's date at the college, as YYYY-MM-DD.
pub async fn today(db: &PgPool) -> Res<String> {
    sqlx::query_scalar(&format!("SELECT to_char({TODAY}, 'YYYY-MM-DD')"))
        .fetch_one(db)
        .await
}

/// The earliest date that can still be edited, as YYYY-MM-DD.
pub async fn window_start(db: &PgPool, days: i32) -> Res<String> {
    sqlx::query_scalar(&format!("SELECT to_char({TODAY} - $1::int, 'YYYY-MM-DD')"))
        .bind(days)
        .fetch_one(db)
        .await
}

/// A real calendar date in YYYY-MM-DD form.
pub fn valid_date(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 || parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return false;
    }
    let (Ok(y), Ok(m), Ok(d)) = (parts[0].parse::<i32>(), parts[1].parse::<u32>(), parts[2].parse::<u32>()) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let max = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if leap { 29 } else { 28 },
        _ => return false,
    };
    (2000..=2100).contains(&y) && d >= 1 && d <= max
}

pub async fn faculty_id_for_user(db: &PgPool, user_id: i64) -> Res<Option<i64>> {
    sqlx::query_scalar("SELECT id FROM faculty WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(db)
        .await
}

// ---------- A teacher's periods ----------

#[derive(Debug, Clone, FromRow)]
pub struct PeriodRow {
    pub on_date: String,
    pub day_label: String,
    pub entry_id: i64,
    pub course_id: i64,
    pub code: String,
    pub course: String,
    pub programme: String,
    pub semester: i32,
    pub start_at: String,
    pub end_at: String,
    pub room: String,
    pub is_substitute: bool,
    pub marked: bool,
    pub present: i64,
    pub absent: i64,
    pub on_leave: i64,
}

const PERIOD_BODY: &str = r#"
SELECT to_char(days.d, 'YYYY-MM-DD') AS on_date,
       to_char(days.d, 'Dy DD Mon') AS day_label,
       t.id AS entry_id,
       t.course_id,
       c.code,
       c.title AS course,
       COALESCE(p.name,
           -- A period of a course offering: show the programmes it is taught to.
           (SELECT string_agg(DISTINCT pp.name, ' / ' ORDER BY pp.name)
              FROM course_offering_targets t2
              JOIN programmes pp ON pp.id = t2.programme_id
             WHERE t2.offering_id = t.course_offering_id),
           'Offered course') AS programme,
       t.semester,
       to_char(t.start_time, 'HH24:MI') AS start_at,
       to_char(t.end_time, 'HH24:MI') AS end_at,
       COALESCE(t.room, '') AS room,
       (sub.id IS NOT NULL) AS is_substitute,
       (s.id IS NOT NULL) AS marked,
       COALESCE((SELECT count(*) FROM attendance_records r WHERE r.session_id = s.id AND r.status = 'present'), 0) AS present,
       COALESCE((SELECT count(*) FROM attendance_records r WHERE r.session_id = s.id AND r.status = 'absent'), 0) AS absent,
       COALESCE((SELECT count(*) FROM attendance_records r WHERE r.session_id = s.id AND r.status = 'leave'), 0) AS on_leave
FROM days
JOIN timetable_entries t ON t.weekday = EXTRACT(ISODOW FROM days.d)::int
JOIN courses c ON c.id = t.course_id
LEFT JOIN programmes p ON p.id = t.programme_id
LEFT JOIN substitutions sub ON sub.timetable_entry_id = t.id AND sub.on_date = days.d
                           AND sub.substitute_faculty_id = $1
LEFT JOIN substitutions any_sub ON any_sub.timetable_entry_id = t.id AND any_sub.on_date = days.d
LEFT JOIN attendance_sessions s ON s.timetable_entry_id = t.id AND s.on_date = days.d
WHERE ( (any_sub.id IS NULL AND t.faculty_id = $1) OR sub.id IS NOT NULL )
"#;

/// Periods from `window_days` ago up to today that this teacher teaches or is covering.
pub async fn periods(db: &PgPool, faculty_id: i64, window_days: i32) -> Res<Vec<PeriodRow>> {
    let sql = format!(
        "WITH days(d) AS (SELECT g::date FROM generate_series(({TODAY}) - $2::int, {TODAY}, interval '1 day') g) {PERIOD_BODY}
         ORDER BY days.d DESC, t.start_time"
    );
    sqlx::query_as::<_, PeriodRow>(&sql)
        .bind(faculty_id)
        .bind(window_days)
        .fetch_all(db)
        .await
}

/// One period on one date, only if this teacher may take its attendance (own class or cover) and
/// the date is today or inside the edit window.
pub async fn period(
    db: &PgPool,
    faculty_id: i64,
    entry_id: i64,
    date: &str,
    window_days: i32,
) -> Res<Option<PeriodRow>> {
    let sql = format!(
        "WITH days(d) AS (SELECT $2::date WHERE $2::date BETWEEN ({TODAY}) - $3::int AND ({TODAY})) {PERIOD_BODY}
         AND t.id = $4"
    );
    sqlx::query_as::<_, PeriodRow>(&sql)
        .bind(faculty_id)
        .bind(date)
        .bind(window_days)
        .bind(entry_id)
        .fetch_optional(db)
        .await
}

// ---------- Department-wide views (head of department) ----------

/// The department that owns a timetable period's programme — or, for a period
/// attached to a course offering, the department that offers it.
pub async fn entry_department(db: &PgPool, entry_id: i64) -> Res<Option<i64>> {
    sqlx::query_scalar(
        "SELECT COALESCE(p.department_id, o.offering_department_id)
           FROM timetable_entries t
           LEFT JOIN programmes p ON p.id = t.programme_id
           LEFT JOIN course_offerings o ON o.id = t.course_offering_id
          WHERE t.id = $1",
    )
    .bind(entry_id)
    .fetch_optional(db)
    .await
}

/// One period by id for someone who is not its teacher, used by the HOD.
///
/// The period is resolved against the period's own teacher rather than the
/// caller's, so this reuses `period` and inherits its edit-window rules.
/// Callers must scope the entry to a department themselves first.
pub async fn period_for_manager(
    db: &PgPool,
    entry_id: i64,
    date: &str,
    window_days: i32,
) -> Res<Option<PeriodRow>> {
    let faculty: Option<i64> =
        sqlx::query_scalar("SELECT faculty_id FROM timetable_entries WHERE id = $1")
            .bind(entry_id)
            .fetch_optional(db)
            .await?;
    let Some(faculty_id) = faculty else {
        return Ok(None);
    };
    period(db, faculty_id, entry_id, date, window_days).await
}

/// A course in one department, for the HOD's whole-department report.
#[derive(Debug, Clone, FromRow)]
pub struct DeptCourse {
    pub id: i64,
    pub code: String,
    pub title: String,
    pub programme: String,
    pub semester: i32,
}

pub async fn department_courses(
    db: &PgPool,
    department: Option<i64>,
) -> Res<Vec<DeptCourse>> {
    sqlx::query_as::<_, DeptCourse>(
        r#"SELECT c.id, c.code, c.title, COALESCE(p.name, 'Catalogue') AS programme,
                  COALESCE(c.semester, 0) AS semester
           FROM courses c
           LEFT JOIN programmes p ON p.id = c.programme_id
          WHERE (c.programme_id IS NULL OR p.status = 'published')
            AND ($1::bigint IS NULL OR p.department_id = $1 OR c.department_id = $1)
          ORDER BY COALESCE(p.name, ''), c.semester, c.code"#,
    )
    .bind(department)
    .fetch_all(db)
    .await
}

// ---------- Roster and saving ----------

#[derive(Debug, FromRow)]
pub struct RosterRow {
    pub student_id: i64,
    pub name: String,
    pub admission_no: String,
    pub status: String,
    pub recorded: bool,
}

/// Who is on the sheet for one period.
///
/// A programme class takes every active enrollment of its course. A period
/// that belongs to a course offering is narrower: only the students holding a
/// confirmed (or locked) selection *for that offering* are on it, so a mixed
/// group sees the students who actually chose the offering instead of every
/// student ever enrolled in the underlying course.
pub async fn roster(db: &PgPool, course_id: i64, entry_id: i64, date: &str) -> Res<Vec<RosterRow>> {
    sqlx::query_as::<_, RosterRow>(
        r#"SELECT st.id AS student_id, st.name, st.admission_no,
                  COALESCE(r.status, 'present') AS status,
                  (r.id IS NOT NULL) AS recorded
           FROM enrollments e
           JOIN students st ON st.id = e.student_id AND st.is_active
           LEFT JOIN attendance_sessions s ON s.timetable_entry_id = $2 AND s.on_date = $3::date
           LEFT JOIN attendance_records r ON r.session_id = s.id AND r.student_id = st.id
           WHERE e.course_id = $1 AND e.status = 'active'
             AND ( -- the offering this period belongs to, if it has one
                  NOT EXISTS (SELECT 1 FROM timetable_entries te
                               WHERE te.id = $2 AND te.course_offering_id IS NOT NULL)
                  OR EXISTS (SELECT 1 FROM timetable_entries te
                              JOIN student_course_selections sel
                                ON sel.offering_id = te.course_offering_id
                               AND sel.student_id = st.id
                              WHERE te.id = $2
                                AND sel.state IN ('confirmed', 'locked')))
           ORDER BY st.name, st.admission_no"#,
    )
    .bind(course_id)
    .bind(entry_id)
    .bind(date)
    .fetch_all(db)
    .await
}

/// Creates or updates the session and all its records in one transaction.
pub async fn save(
    db: &PgPool,
    period: &PeriodRow,
    taught_by: i64,
    marked_by: i64,
    student_ids: &[i64],
    statuses: &[String],
) -> Res<i64> {
    let mut tx = db.begin().await?;
    let session_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO attendance_sessions (timetable_entry_id, course_id, on_date, taught_by, marked_by)
           VALUES ($1, $2, $3::date, $4, $5)
           ON CONFLICT (timetable_entry_id, on_date)
           DO UPDATE SET taught_by = EXCLUDED.taught_by, marked_by = EXCLUDED.marked_by
           RETURNING id"#,
    )
    .bind(period.entry_id)
    .bind(period.course_id)
    .bind(&period.on_date)
    .bind(taught_by)
    .bind(marked_by)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO attendance_records (session_id, student_id, status, updated_by)
           SELECT $1, u.sid, u.st, $4 FROM UNNEST($2::bigint[], $3::text[]) AS u(sid, st)
           ON CONFLICT (session_id, student_id)
           DO UPDATE SET status = EXCLUDED.status, updated_by = EXCLUDED.updated_by"#,
    )
    .bind(session_id)
    .bind(student_ids)
    .bind(statuses)
    .bind(marked_by)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(session_id)
}

// ---------- Weekly timetable and covers ----------

#[derive(Debug, FromRow)]
pub struct TeacherSlot {
    pub weekday: i16,
    pub start_at: String,
    pub end_at: String,
    pub code: String,
    pub course: String,
    pub programme: String,
    pub semester: i32,
    pub room: String,
}

pub async fn teacher_slots(db: &PgPool, faculty_id: i64) -> Res<Vec<TeacherSlot>> {
    sqlx::query_as::<_, TeacherSlot>(
        r#"SELECT t.weekday,
                  to_char(t.start_time, 'HH24:MI') AS start_at,
                  to_char(t.end_time, 'HH24:MI') AS end_at,
                  c.code, c.title AS course,
                  COALESCE(p.name,
                      (SELECT string_agg(DISTINCT pp.name, ' / ' ORDER BY pp.name)
                         FROM course_offering_targets t2
                         JOIN programmes pp ON pp.id = t2.programme_id
                        WHERE t2.offering_id = t.course_offering_id),
                      'Offered course') AS programme,
                  t.semester,
                  COALESCE(t.room, '') AS room
           FROM timetable_entries t
           JOIN courses c ON c.id = t.course_id
           LEFT JOIN programmes p ON p.id = t.programme_id
           WHERE t.faculty_id = $1
           ORDER BY t.weekday, t.start_time"#,
    )
    .bind(faculty_id)
    .fetch_all(db)
    .await
}

#[derive(Debug, FromRow)]
pub struct CoverRow {
    pub day_label: String,
    pub start_at: String,
    pub end_at: String,
    pub course: String,
    pub programme: String,
    pub semester: i32,
    pub covering_for: String,
}

/// Upcoming classes this teacher has been asked to cover.
pub async fn upcoming_covers(db: &PgPool, faculty_id: i64) -> Res<Vec<CoverRow>> {
    sqlx::query_as::<_, CoverRow>(&format!(
        r#"SELECT to_char(sub.on_date, 'Dy DD Mon') AS day_label,
                  to_char(t.start_time, 'HH24:MI') AS start_at,
                  to_char(t.end_time, 'HH24:MI') AS end_at,
                  c.title AS course,
                  COALESCE(p.name, 'Offered course') AS programme,
                  t.semester,
                  COALESCE(orig.name, 'another teacher') AS covering_for
           FROM substitutions sub
           JOIN timetable_entries t ON t.id = sub.timetable_entry_id
           JOIN courses c ON c.id = t.course_id
           LEFT JOIN programmes p ON p.id = t.programme_id
           LEFT JOIN faculty orig ON orig.id = t.faculty_id
           WHERE sub.substitute_faculty_id = $1 AND sub.on_date >= {TODAY}
           ORDER BY sub.on_date, t.start_time
           LIMIT 30"#
    ))
    .bind(faculty_id)
    .fetch_all(db)
    .await
}

// ---------- Reports ----------

#[derive(Debug, FromRow)]
pub struct TeacherCourse {
    pub id: i64,
    pub code: String,
    pub title: String,
    pub programme: String,
    pub semester: i32,
}

pub async fn teacher_courses(db: &PgPool, faculty_id: i64) -> Res<Vec<TeacherCourse>> {
    sqlx::query_as::<_, TeacherCourse>(
        r#"SELECT DISTINCT c.id, c.code, c.title, COALESCE(p.name, 'Catalogue') AS programme,
                  COALESCE(c.semester, 0) AS semester
           FROM courses c
           LEFT JOIN programmes p ON p.id = c.programme_id
           WHERE c.faculty_id = $1
              OR EXISTS (SELECT 1 FROM timetable_entries t WHERE t.course_id = c.id AND t.faculty_id = $1)
              OR EXISTS (SELECT 1 FROM course_offerings o WHERE o.course_id = c.id AND o.faculty_id = $1)
              OR EXISTS (SELECT 1 FROM attendance_sessions s WHERE s.course_id = c.id AND s.taught_by = $1)
           -- DISTINCT: order by the selected expressions (aliases), never the
           -- underlying columns, or Postgres rejects the query.
           ORDER BY programme, semester, c.code"#,
    )
    .bind(faculty_id)
    .fetch_all(db)
    .await
}

#[derive(Debug, FromRow)]
pub struct ReportRaw {
    pub name: String,
    pub admission_no: String,
    pub marked: i64,
    pub present: i64,
    pub absent: i64,
    pub on_leave: i64,
}

pub async fn course_report(db: &PgPool, course_id: i64) -> Res<Vec<ReportRaw>> {
    sqlx::query_as::<_, ReportRaw>(&format!(
        r#"SELECT st.name, st.admission_no,
                  count(r.id) AS marked,
                  count(r.id) FILTER (WHERE r.status = 'present') AS present,
                  count(r.id) FILTER (WHERE r.status = 'absent') AS absent,
                  count(r.id) FILTER (WHERE r.status = 'leave') AS on_leave
           FROM enrollments e
           JOIN students st ON st.id = e.student_id AND st.is_active
           LEFT JOIN attendance_sessions s ON s.course_id = e.course_id
           LEFT JOIN attendance_records r ON r.session_id = s.id AND r.student_id = st.id
           WHERE e.course_id = $1 AND e.status = 'active'
           GROUP BY st.id
           ORDER BY st.name"#
    ))
    .bind(course_id)
    .fetch_all(db)
    .await
}

/// Attendance percentage, or None when nothing has been marked yet.
pub fn percent(present: i64, on_leave: i64, marked: i64, leave_counts: bool) -> Option<f64> {
    if marked == 0 {
        return None;
    }
    let good = present + if leave_counts { on_leave } else { 0 };
    Some(good as f64 * 100.0 / marked as f64)
}

// ---------- Substitutions (admin side) ----------

#[derive(Debug, FromRow)]
pub struct CoverCandidate {
    pub entry_id: i64,
    pub start_at: String,
    pub end_at: String,
    pub course: String,
    pub programme: String,
    pub semester: i32,
    pub substitution_id: i64,
    pub substitute: String,
}

pub async fn teacher_day(db: &PgPool, teacher_id: i64, date: &str) -> Res<Vec<CoverCandidate>> {
    sqlx::query_as::<_, CoverCandidate>(
        r#"SELECT t.id AS entry_id,
                  to_char(t.start_time, 'HH24:MI') AS start_at,
                  to_char(t.end_time, 'HH24:MI') AS end_at,
                  c.title AS course, p.name AS programme, t.semester,
                  COALESCE(sub.id, 0) AS substitution_id,
                  COALESCE(sf.name, '') AS substitute
           FROM timetable_entries t
           JOIN courses c ON c.id = t.course_id
           JOIN programmes p ON p.id = t.programme_id
           LEFT JOIN substitutions sub ON sub.timetable_entry_id = t.id AND sub.on_date = $2::date
           LEFT JOIN faculty sf ON sf.id = sub.substitute_faculty_id
           WHERE t.faculty_id = $1 AND t.weekday = EXTRACT(ISODOW FROM $2::date)::int
           ORDER BY t.start_time"#,
    )
    .bind(teacher_id)
    .bind(date)
    .fetch_all(db)
    .await
}

/// The teacher the timetable gives this period to (0 if none).
pub async fn entry_teacher(db: &PgPool, entry_id: i64) -> Res<Option<i64>> {
    let r: Option<Option<i64>> = sqlx::query_scalar("SELECT faculty_id FROM timetable_entries WHERE id = $1")
        .bind(entry_id)
        .fetch_optional(db)
        .await?;
    Ok(r.map(|v| v.unwrap_or(0)))
}

/// Is `faculty_id` already teaching or covering something that overlaps this period on that date?
pub async fn faculty_busy(db: &PgPool, faculty_id: i64, entry_id: i64, date: &str) -> Res<bool> {
    sqlx::query_scalar(
        r#"SELECT EXISTS (
               SELECT 1
               FROM timetable_entries t2
               JOIN timetable_entries me ON me.id = $3
               LEFT JOIN substitutions s2 ON s2.timetable_entry_id = t2.id AND s2.on_date = $2::date
               WHERE t2.id <> me.id
                 AND t2.weekday = EXTRACT(ISODOW FROM $2::date)::int
                 AND t2.start_time < me.end_time AND t2.end_time > me.start_time
                 AND ( (s2.id IS NULL AND t2.faculty_id = $1) OR s2.substitute_faculty_id = $1 ))"#,
    )
    .bind(faculty_id)
    .bind(date)
    .bind(entry_id)
    .fetch_one(db)
    .await
}

pub async fn set_substitution(
    db: &PgPool,
    entry_id: i64,
    date: &str,
    substitute_id: i64,
    reason: &str,
    created_by: i64,
) -> Res<()> {
    sqlx::query(
        r#"INSERT INTO substitutions (timetable_entry_id, on_date, substitute_faculty_id, reason, created_by)
           VALUES ($1, $2::date, $3, $4, $5)
           ON CONFLICT (timetable_entry_id, on_date)
           DO UPDATE SET substitute_faculty_id = EXCLUDED.substitute_faculty_id,
                         reason = EXCLUDED.reason, created_by = EXCLUDED.created_by"#,
    )
    .bind(entry_id)
    .bind(date)
    .bind(substitute_id)
    .bind(reason)
    .bind(created_by)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn remove_substitution(db: &PgPool, id: i64) -> Res<()> {
    sqlx::query("DELETE FROM substitutions WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}
