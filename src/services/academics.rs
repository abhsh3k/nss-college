use sqlx::{FromRow, PgPool};

type Res<T> = Result<T, sqlx::Error>;

// ---------- Programmes and courses ----------

#[derive(Debug, FromRow)]
pub struct ProgrammeOption {
    pub id: i64,
    pub name: String,
    pub department: String,
    pub level: String,
}

pub async fn programme_options(db: &PgPool) -> Res<Vec<ProgrammeOption>> {
    sqlx::query_as::<_, ProgrammeOption>(
        r#"SELECT p.id, p.name, d.name AS department, p.level
           FROM programmes p JOIN departments d ON d.id = p.department_id
           WHERE p.status = 'published' ORDER BY p.sort_order, p.name"#,
    )
    .fetch_all(db)
    .await
}

pub async fn programme(db: &PgPool, id: i64) -> Res<Option<ProgrammeOption>> {
    sqlx::query_as::<_, ProgrammeOption>(
        r#"SELECT p.id, p.name, d.name AS department, p.level
           FROM programmes p JOIN departments d ON d.id = p.department_id
           WHERE p.id = $1"#,
    )
    .bind(id)
    .fetch_optional(db)
    .await
}

#[derive(Debug, FromRow)]
pub struct ProgrammeSummary {
    pub id: i64,
    pub name: String,
    pub department: String,
    pub level: String,
    pub course_count: i64,
    pub student_count: i64,
}

pub async fn programme_summaries(db: &PgPool) -> Res<Vec<ProgrammeSummary>> {
    sqlx::query_as::<_, ProgrammeSummary>(
        r#"SELECT p.id, p.name, d.name AS department, p.level,
                  (SELECT count(*) FROM courses c WHERE c.programme_id = p.id) AS course_count,
                  (SELECT count(*) FROM students s WHERE s.programme_id = p.id AND s.is_active) AS student_count
           FROM programmes p JOIN departments d ON d.id = p.department_id
           WHERE p.status = 'published' ORDER BY p.sort_order, p.name"#,
    )
    .fetch_all(db)
    .await
}

#[derive(Debug, Clone, FromRow)]
pub struct CourseRow {
    pub id: i64,
    pub code: String,
    pub title: String,
    pub semester: i32,
    pub credits: i32,
    pub teacher: String,
    pub faculty_id: i64,
}

pub async fn courses(db: &PgPool, programme_id: i64) -> Res<Vec<CourseRow>> {
    sqlx::query_as::<_, CourseRow>(
        r#"SELECT c.id, c.code, c.title, c.semester, c.credits,
                  COALESCE(f.name, '') AS teacher,
                  COALESCE(c.faculty_id, 0) AS faculty_id
           FROM courses c LEFT JOIN faculty f ON f.id = c.faculty_id
           WHERE c.programme_id = $1 ORDER BY c.semester, c.code"#,
    )
    .bind(programme_id)
    .fetch_all(db)
    .await
}

#[derive(Debug, FromRow)]
pub struct CourseDetail {
    pub id: i64,
    pub programme_id: i64,
    pub programme: String,
    pub code: String,
    pub title: String,
    pub semester: i32,
    pub credits: i32,
    pub faculty_id: i64,
}

pub async fn course(db: &PgPool, id: i64) -> Res<Option<CourseDetail>> {
    sqlx::query_as::<_, CourseDetail>(
        r#"SELECT c.id, c.programme_id, p.name AS programme, c.code, c.title, c.semester, c.credits,
                  COALESCE(c.faculty_id, 0) AS faculty_id
           FROM courses c JOIN programmes p ON p.id = c.programme_id
           WHERE c.id = $1"#,
    )
    .bind(id)
    .fetch_optional(db)
    .await
}

#[derive(Debug, FromRow)]
pub struct TeacherOption {
    pub id: i64,
    pub name: String,
    pub department: String,
}

pub async fn teacher_options(db: &PgPool) -> Res<Vec<TeacherOption>> {
    sqlx::query_as::<_, TeacherOption>(
        r#"SELECT f.id, f.name, COALESCE(d.name, '') AS department
           FROM faculty f LEFT JOIN departments d ON d.id = f.department_id
           WHERE f.status = 'published' ORDER BY f.name"#,
    )
    .fetch_all(db)
    .await
}

pub struct CourseInput<'a> {
    pub code: &'a str,
    pub title: &'a str,
    pub semester: i32,
    pub credits: i32,
    pub faculty_id: i64,
}

pub async fn create_course(db: &PgPool, programme_id: i64, c: &CourseInput<'_>) -> Res<i64> {
    sqlx::query_scalar(
        r#"INSERT INTO courses (programme_id, faculty_id, code, title, semester, credits)
           VALUES ($1, NULLIF($2, 0), $3, $4, $5, $6) RETURNING id"#,
    )
    .bind(programme_id)
    .bind(c.faculty_id)
    .bind(c.code)
    .bind(c.title)
    .bind(c.semester)
    .bind(c.credits)
    .fetch_one(db)
    .await
}

pub async fn update_course(db: &PgPool, id: i64, c: &CourseInput<'_>) -> Res<()> {
    sqlx::query(
        r#"UPDATE courses
           SET faculty_id = NULLIF($2, 0), code = $3, title = $4, semester = $5, credits = $6
           WHERE id = $1"#,
    )
    .bind(id)
    .bind(c.faculty_id)
    .bind(c.code)
    .bind(c.title)
    .bind(c.semester)
    .bind(c.credits)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete_course(db: &PgPool, id: i64) -> Res<()> {
    sqlx::query("DELETE FROM courses WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

/// Enrolls every active student of a programme semester in that semester's courses.
pub async fn enroll_semester(db: &PgPool, programme_id: i64, semester: i32) -> Res<u64> {
    let done = sqlx::query(
        r#"INSERT INTO enrollments (student_id, course_id)
           SELECT s.id, c.id
           FROM students s
           JOIN courses c ON c.programme_id = s.programme_id AND c.semester = s.semester
           WHERE s.programme_id = $1 AND s.semester = $2 AND s.is_active
           ON CONFLICT DO NOTHING"#,
    )
    .bind(programme_id)
    .bind(semester)
    .execute(db)
    .await?;
    Ok(done.rows_affected())
}

// ---------- Timetable ----------

#[derive(Debug, FromRow)]
pub struct SlotRow {
    pub id: i64,
    pub weekday: i16,
    pub start_at: String,
    pub end_at: String,
    pub code: String,
    pub course: String,
    pub teacher: String,
    pub room: String,
}

pub async fn slots(db: &PgPool, programme_id: i64, semester: i32) -> Res<Vec<SlotRow>> {
    sqlx::query_as::<_, SlotRow>(
        r#"SELECT t.id, t.weekday,
                  to_char(t.start_time, 'HH24:MI') AS start_at,
                  to_char(t.end_time, 'HH24:MI') AS end_at,
                  c.code, c.title AS course,
                  COALESCE(f.name, '') AS teacher,
                  COALESCE(t.room, '') AS room
           FROM timetable_entries t
           JOIN courses c ON c.id = t.course_id
           LEFT JOIN faculty f ON f.id = t.faculty_id
           WHERE t.programme_id = $1 AND t.semester = $2
           ORDER BY t.weekday, t.start_time"#,
    )
    .bind(programme_id)
    .bind(semester)
    .fetch_all(db)
    .await
}

pub struct NewSlot {
    pub programme_id: i64,
    pub semester: i32,
    pub course_id: i64,
    pub faculty_id: i64, // 0 = none
    pub weekday: i16,
    pub start_at: String, // HH:MM
    pub end_at: String,   // HH:MM
    pub room: String,     // may be empty
}

#[derive(Debug, FromRow)]
pub struct Clash {
    pub same_teacher: bool,
    pub same_room: bool,
    pub same_class: bool,
    pub course: String,
    pub start_at: String,
    pub end_at: String,
}

pub async fn course_in_class(db: &PgPool, course_id: i64, programme_id: i64, semester: i32) -> Res<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM courses WHERE id = $1 AND programme_id = $2 AND semester = $3)",
    )
    .bind(course_id)
    .bind(programme_id)
    .bind(semester)
    .fetch_one(db)
    .await
}

pub async fn course_teacher(db: &PgPool, course_id: i64) -> Res<i64> {
    let id: Option<Option<i64>> = sqlx::query_scalar("SELECT faculty_id FROM courses WHERE id = $1")
        .bind(course_id)
        .fetch_optional(db)
        .await?;
    Ok(id.flatten().unwrap_or(0))
}

/// Existing periods that overlap the new one and share its teacher, room or class.
pub async fn clashes(db: &PgPool, n: &NewSlot) -> Res<Vec<Clash>> {
    sqlx::query_as::<_, Clash>(
        r#"SELECT COALESCE(t.faculty_id = NULLIF($4, 0), false) AS same_teacher,
                  COALESCE(t.room IS NOT NULL AND t.room <> '' AND lower(t.room) = lower($5), false) AS same_room,
                  (t.programme_id = $6 AND t.semester = $7) AS same_class,
                  c.title AS course,
                  to_char(t.start_time, 'HH24:MI') AS start_at,
                  to_char(t.end_time, 'HH24:MI') AS end_at
           FROM timetable_entries t JOIN courses c ON c.id = t.course_id
           WHERE t.weekday = $1
             AND t.start_time < $3::time AND t.end_time > $2::time
             AND ( COALESCE(t.faculty_id = NULLIF($4, 0), false)
                   OR COALESCE(t.room IS NOT NULL AND t.room <> '' AND lower(t.room) = lower($5), false)
                   OR (t.programme_id = $6 AND t.semester = $7) )"#,
    )
    .bind(n.weekday)
    .bind(&n.start_at)
    .bind(&n.end_at)
    .bind(n.faculty_id)
    .bind(&n.room)
    .bind(n.programme_id)
    .bind(n.semester)
    .fetch_all(db)
    .await
}

pub async fn add_slot(db: &PgPool, n: &NewSlot) -> Res<i64> {
    sqlx::query_scalar(
        r#"INSERT INTO timetable_entries
               (programme_id, semester, course_id, faculty_id, weekday, start_time, end_time, room)
           VALUES ($1, $2, $3, NULLIF($4, 0), $5, $6::time, $7::time, NULLIF($8, ''))
           RETURNING id"#,
    )
    .bind(n.programme_id)
    .bind(n.semester)
    .bind(n.course_id)
    .bind(n.faculty_id)
    .bind(n.weekday)
    .bind(&n.start_at)
    .bind(&n.end_at)
    .bind(&n.room)
    .fetch_one(db)
    .await
}

pub async fn delete_slot(db: &PgPool, id: i64) -> Res<()> {
    sqlx::query("DELETE FROM timetable_entries WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}
