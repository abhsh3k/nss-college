//! Attendance reporting for the admin dashboard.
//!
//! Two views over the same filters: one row per student (the "who is at risk"
//! report) and one row per session (the "when and what was taught" report).

use sqlx::{FromRow, PgPool};

type Res<T> = Result<T, sqlx::Error>;

/// The filters shared by both reports. A zero id means "no filter".
///
/// The dates are validated ISO `YYYY-MM-DD` strings and cast by Postgres, which
/// keeps this module free of date-parsing dependencies.
#[derive(Debug, Clone)]
pub struct Filter {
    pub from: String,
    pub to: String,
    pub programme_id: i64,
    pub course_id: i64,
    pub faculty_id: i64,
}

/// One student across the selected period.
#[derive(Debug, FromRow)]
pub struct StudentSummary {
    pub admission_no: String,
    pub name: String,
    pub programme: String,
    pub marked: i64,
    pub present: i64,
    pub absent: i64,
    pub leave: i64,
}

/// One taught session across the selected period.
#[derive(Debug, FromRow)]
pub struct SessionSummary {
    #[allow(dead_code)]
    pub on_date: String,
    pub date_label: String,
    pub course_code: String,
    pub course_title: String,
    pub teacher: String,
    pub time_label: String,
    pub marked: i64,
    pub present: i64,
    pub absent: i64,
    pub leave: i64,
}

/// Totals shown above both tables.
#[derive(Debug, FromRow)]
pub struct ReportTotals {
    pub sessions: i64,
    pub students: i64,
    pub marked: i64,
    pub absent: i64,
}

/// Shared FROM/WHERE so the two reports always agree on what is in scope.
const FROM_WHERE: &str = r#"
    FROM attendance_sessions sess
    JOIN attendance_records ar ON ar.session_id = sess.id
    JOIN students st ON st.id = ar.student_id
    LEFT JOIN programmes p ON p.id = st.programme_id
    JOIN courses c ON c.id = sess.course_id
    WHERE sess.on_date BETWEEN $1::date AND $2::date
      AND ($3 = 0 OR st.programme_id = $3)
      AND ($4 = 0 OR sess.course_id = $4)
      AND ($5 = 0 OR sess.taught_by = $5)
"#;

fn bind_filter<'q, O>(
    q: sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments>,
    f: &Filter,
) -> sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments> {
    q.bind(f.from.clone())
        .bind(f.to.clone())
        .bind(f.programme_id)
        .bind(f.course_id)
        .bind(f.faculty_id)
}

/// Per-student totals for the period. Unsorted; `decorate` orders the rows.
pub async fn by_student(db: &PgPool, f: &Filter) -> Res<Vec<StudentSummary>> {
    let sql = format!(
        r#"SELECT st.admission_no,
                  st.name,
                  COALESCE(p.name, '—') AS programme,
                  count(ar.id) AS marked,
                  count(ar.id) FILTER (WHERE ar.status = 'present') AS present,
                  count(ar.id) FILTER (WHERE ar.status = 'absent') AS absent,
                  count(ar.id) FILTER (WHERE ar.status = 'leave') AS leave
           {FROM_WHERE}
           GROUP BY st.id, st.admission_no, st.name, p.name"#
    );
    let rows = bind_filter(sqlx::query_as::<_, StudentSummary>(&sql), f)
        .fetch_all(db)
        .await?;
    Ok(rows)
}

/// Per-session totals, newest first.
pub async fn by_session(db: &PgPool, f: &Filter) -> Res<Vec<SessionSummary>> {
    let sql = format!(
            r#"SELECT to_char(sess.on_date, 'YYYY-MM-DD') AS on_date,
                  to_char(sess.on_date, 'DD Mon YYYY') AS date_label,
                  c.code AS course_code,
                  c.title AS course_title,
                  COALESCE(f.name, '—') AS teacher,
                  COALESCE(to_char(te.start_time, 'HH24:MI') || '–' || to_char(te.end_time, 'HH24:MI'), '—') AS time_label,
                  count(ar.id) AS marked,
                  count(ar.id) FILTER (WHERE ar.status = 'present') AS present,
                  count(ar.id) FILTER (WHERE ar.status = 'absent') AS absent,
                  count(ar.id) FILTER (WHERE ar.status = 'leave') AS leave
            FROM attendance_sessions sess
            JOIN courses c ON c.id = sess.course_id
            LEFT JOIN faculty f ON f.id = sess.taught_by
            LEFT JOIN timetable_entries te ON te.id = sess.timetable_entry_id
            JOIN attendance_records ar ON ar.session_id = sess.id
            JOIN students st ON st.id = ar.student_id
            WHERE sess.on_date BETWEEN $1::date AND $2::date
              AND ($3 = 0 OR st.programme_id = $3)
             AND ($4 = 0 OR sess.course_id = $4)
             AND ($5 = 0 OR sess.taught_by = $5)
           GROUP BY sess.id, sess.on_date, c.code, c.title, f.name, te.start_time, te.end_time
           ORDER BY sess.on_date DESC, te.start_time DESC"#
    );
    bind_filter(sqlx::query_as::<_, SessionSummary>(&sql), f)
        .fetch_all(db)
        .await
}

pub async fn totals(db: &PgPool, f: &Filter) -> Res<ReportTotals> {
    let sql = format!(
        r#"SELECT (SELECT count(DISTINCT sess.id) {FROM_WHERE}) AS sessions,
                  (SELECT count(DISTINCT st.id)  {FROM_WHERE}) AS students,
                  (SELECT count(ar.id)            {FROM_WHERE}) AS marked,
                  (SELECT count(ar.id) FILTER (WHERE ar.status = 'absent') {FROM_WHERE}) AS absent"#
    );
    bind_filter(sqlx::query_as::<_, ReportTotals>(&sql), f)
        .fetch_one(db)
        .await
}

/// Leave counts towards the percentage only when the college has switched it on.
fn attended(present: i64, leave: i64, leave_counts: bool) -> i64 {
    present + if leave_counts { leave } else { 0 }
}

/// A percentage that stays ordered sensibly: no sessions yet sorts last.
fn percent(marked: i64, attended: i64) -> f64 {
    if marked <= 0 {
        -1.0
    } else {
        100.0 * attended as f64 / marked as f64
    }
}

/// The same percentage, formatted for the table.
pub fn percent_label(marked: i64, attended: i64) -> String {
    if marked <= 0 {
        "—".to_string()
    } else {
        format!("{:.1}%", 100.0 * attended as f64 / marked as f64)
    }
}

/// Below this a student is flagged on the report.
pub const DEFAULT_LOW_PERCENT: f64 = 75.0;

/// One report row after the "does leave count" rule has been applied.
pub struct StudentRow {
    pub admission_no: String,
    pub name: String,
    pub programme: String,
    pub marked: i64,
    pub present: i64,
    pub absent: i64,
    pub leave: i64,
    pub attended: i64,
    pub percent_label: String,
    pub is_low: bool,
}

/// Apply the leave rule and the low-attendance flag, and order worst-first so the
/// at-risk students lead the report. Sorting has to happen here rather than in SQL
/// because the percentage depends on the "does leave count" setting.
pub fn decorate(rows: Vec<StudentSummary>, leave_counts: bool, low_percent: f64) -> Vec<StudentRow> {
    let mut out: Vec<StudentRow> = rows.into_iter()
        .map(|r| {
            let attended = attended(r.present, r.leave, leave_counts);
            let pct = percent(r.marked, attended);
            StudentRow {
                percent_label: percent_label(r.marked, attended),
                is_low: r.marked > 0 && pct < low_percent,
                admission_no: r.admission_no,
                name: r.name,
                programme: r.programme,
                marked: r.marked,
                present: r.present,
                absent: r.absent,
                leave: r.leave,
                attended,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        percent(a.marked, a.attended)
            .total_cmp(&percent(b.marked, b.attended))
            .then_with(|| a.admission_no.cmp(&b.admission_no))
    });
    out
}

/// The per-student report as CSV, for the IT admin to hand to the office.
pub fn students_csv(rows: &[StudentRow]) -> csv::Result<Vec<u8>> {
    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record([
        "Admission no",
        "Name",
        "Programme",
        "Sessions marked",
        "Present",
        "Absent",
        "Leave",
        "Counted as attended",
        "Attendance %",
    ])?;
    for r in rows {
        w.write_record([
            r.admission_no.as_str(),
            r.name.as_str(),
            r.programme.as_str(),
            &r.marked.to_string(),
            &r.present.to_string(),
            &r.absent.to_string(),
            &r.leave.to_string(),
            &r.attended.to_string(),
            r.percent_label.as_str(),
        ])?;
    }
    w.into_inner()
        .map_err(|e| csv::Error::from(e.into_error()))
}



// ---------- Filter option lists ----------

#[derive(Debug, FromRow)]
pub struct CourseOption {
    pub id: i64,
    pub code: String,
    pub title: String,
}

/// Courses offered by a programme, for the course filter. Zero id means all.
pub async fn course_options(db: &PgPool, programme_id: i64) -> Res<Vec<CourseOption>> {
    sqlx::query_as::<_, CourseOption>(
        r#"SELECT id, code, title FROM courses
           WHERE ($1 = 0 OR programme_id = $1)
           ORDER BY code"#,
    )
    .bind(programme_id)
    .fetch_all(db)
    .await
}
