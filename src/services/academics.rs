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

/// Programmes in one department, for the HOD's timetable picker.
/// `None` means the IT admin, who sees every programme.
pub async fn programme_options_for(
    db: &PgPool,
    department: Option<i64>,
) -> Res<Vec<ProgrammeOption>> {
    let Some(dep) = department else {
        return programme_options(db).await;
    };
    sqlx::query_as::<_, ProgrammeOption>(
        r#"SELECT p.id, p.name, d.name AS department, p.level
           FROM programmes p JOIN departments d ON d.id = p.department_id
           WHERE p.status = 'published' AND p.department_id = $1
           ORDER BY p.sort_order, p.name"#,
    )
    .bind(dep)
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

/// The department a programme belongs to, for scoping HOD actions.
pub async fn programme_department(db: &PgPool, id: i64) -> Res<Option<i64>> {
    sqlx::query_scalar("SELECT department_id FROM programmes WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await
}

/// Whether a manager may act on this programme.
///
/// The IT admin (`department` is `None`) may touch any programme. An HOD or an
/// approved teacher may only touch programmes in their own department, and a
/// teacher whose department is not set yet may touch nothing.
pub async fn may_manage_programme(
    db: &PgPool,
    programme_id: i64,
    department: Option<i64>,
) -> Res<bool> {
    let Some(want) = department else {
        return Ok(true);
    };
    Ok(matches!(programme_department(db, programme_id).await?, Some(id) if id == want))
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

/// Programme summaries limited to one department, for the HOD's own view.
/// `None` means the IT admin, who sees every programme.
pub async fn programme_summaries_for(
    db: &PgPool,
    department: Option<i64>,
) -> Res<Vec<ProgrammeSummary>> {
    let Some(dep) = department else {
        return programme_summaries(db).await;
    };
    sqlx::query_as::<_, ProgrammeSummary>(
        r#"SELECT p.id, p.name, d.name AS department, p.level,
                  (SELECT count(*) FROM courses c WHERE c.programme_id = p.id) AS course_count,
                  (SELECT count(*) FROM students s WHERE s.programme_id = p.id AND s.is_active) AS student_count
           FROM programmes p JOIN departments d ON d.id = p.department_id
           WHERE p.status = 'published' AND p.department_id = $1
           ORDER BY p.sort_order, p.name"#,
    )
    .bind(dep)
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

/// Teachers in one department, for HOD substitution pickers.
pub async fn teacher_options_for(
    db: &PgPool,
    department: Option<i64>,
) -> Res<Vec<TeacherOption>> {
    let Some(dep) = department else {
        return teacher_options(db).await;
    };
    sqlx::query_as::<_, TeacherOption>(
        r#"SELECT f.id, f.name, COALESCE(d.name, '') AS department
           FROM faculty f LEFT JOIN departments d ON d.id = f.department_id
           WHERE f.status = 'published' AND f.department_id = $1 ORDER BY f.name"#,
    )
    .bind(dep)
    .fetch_all(db)
    .await
}

/// Whether a manager may act on this teacher, for substitutions and timetable
/// assignment. The IT admin may act on anyone.
pub async fn may_manage_teacher(
    db: &PgPool,
    faculty_id: i64,
    department: Option<i64>,
) -> Res<bool> {
    let Some(want) = department else {
        return Ok(true);
    };
    let dept: Option<i64> =
        sqlx::query_scalar("SELECT department_id FROM faculty WHERE id = $1")
            .bind(faculty_id)
            .fetch_optional(db)
            .await?;
    Ok(matches!(dept, Some(id) if id == want))
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
///
/// Hybrid enrollment: a course under a published, non-FIXED offering is chosen
/// by students (or assigned by a HOD / cohort decision), so the bulk enrol keeps
/// it out and lets the selection system create those enrollments instead.
/// Fixed courses and courses with no offering behave exactly as before.
pub async fn enroll_semester(db: &PgPool, programme_id: i64, semester: i32) -> Res<u64> {
    let done = sqlx::query(
        r#"INSERT INTO enrollments (student_id, course_id, semester, offering_id)
           SELECT s.id, c.id, s.semester,
                  -- A fixed offering produced this row when one exists, so
                  -- seats are counted against it rather than the bare course.
                  (SELECT o.id FROM course_offerings o
                    WHERE o.course_id = c.id AND o.status = 'published'
                      AND o.selection_mode = 'FIXED' AND o.semester = s.semester
                    ORDER BY o.id LIMIT 1)
             FROM students s
             JOIN courses c ON c.programme_id = s.programme_id AND c.semester = s.semester
            WHERE s.programme_id = $1 AND s.semester = $2 AND s.is_active
              AND NOT EXISTS (
                  SELECT 1 FROM course_offerings o
                   WHERE o.course_id = c.id
                     AND o.status = 'published'
                     AND o.selection_mode <> 'FIXED')
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
    /// True when the period belongs to a course offering rather than the
    /// programme itself. The programme grid shows those read-only so a clash
    /// is visible before it is attempted.
    pub is_offering: bool,
}

/// The programme's own periods plus the periods of every course offering that
/// targets it in the same semester.
///
/// The two halves match `clashes()` exactly: whatever is shown here is what
/// the clash check will reject a booking against, so the grid is a true view
/// of the class's time. Offering rows keep `programme_id` NULL, so they need
/// their own targeting clause rather than a plain `programme_id = $1`.
pub async fn slots(db: &PgPool, programme_id: i64, semester: i32) -> Res<Vec<SlotRow>> {
    sqlx::query_as::<_, SlotRow>(
        r#"SELECT t.id, t.weekday,
                  to_char(t.start_time, 'HH24:MI') AS start_at,
                  to_char(t.end_time, 'HH24:MI') AS end_at,
                  c.code, c.title AS course,
                  COALESCE(f.name, '') AS teacher,
                  COALESCE(t.room, '') AS room,
                  (t.course_offering_id IS NOT NULL) AS is_offering
           FROM timetable_entries t
           JOIN courses c ON c.id = t.course_id
           LEFT JOIN faculty f ON f.id = t.faculty_id
           WHERE (t.programme_id = $1 AND t.semester = $2)
              OR (t.course_offering_id IS NOT NULL AND t.semester = $2
                  AND EXISTS (SELECT 1 FROM course_offering_targets tg
                               WHERE tg.offering_id = t.course_offering_id
                                 AND tg.programme_id = $1))
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
///
/// "Same class" covers both halves of the timetable: another period of this
/// programme and semester, and a period of a course offering that targets this
/// programme in this semester (an offering period has `programme_id` NULL, so
/// a plain `t.programme_id = $6` comparison would never see it).
pub async fn clashes(db: &PgPool, n: &NewSlot) -> Res<Vec<Clash>> {
    sqlx::query_as::<_, Clash>(
        r#"SELECT COALESCE(t.faculty_id = NULLIF($4, 0), false) AS same_teacher,
                  COALESCE(t.room IS NOT NULL AND t.room <> '' AND lower(t.room) = lower($5), false) AS same_room,
                  (t.programme_id = $6 AND t.semester = $7)
                  OR (t.course_offering_id IS NOT NULL AND t.semester = $7
                      AND EXISTS (SELECT 1 FROM course_offering_targets tg
                                   WHERE tg.offering_id = t.course_offering_id
                                     AND tg.programme_id = $6)) AS same_class,
                  c.title AS course,
                  to_char(t.start_time, 'HH24:MI') AS start_at,
                  to_char(t.end_time, 'HH24:MI') AS end_at
           FROM timetable_entries t JOIN courses c ON c.id = t.course_id
           WHERE t.weekday = $1
             AND t.start_time < $3::time AND t.end_time > $2::time
             AND ( COALESCE(t.faculty_id = NULLIF($4, 0), false)
                   OR COALESCE(t.room IS NOT NULL AND t.room <> '' AND lower(t.room) = lower($5), false)
                   OR (t.programme_id = $6 AND t.semester = $7)
                   OR (t.course_offering_id IS NOT NULL AND t.semester = $7
                       AND EXISTS (SELECT 1 FROM course_offering_targets tg
                                    WHERE tg.offering_id = t.course_offering_id
                                      AND tg.programme_id = $6)) )"#,
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

// ---------- Periods of a course offering ----------

/// The periods already scheduled for one course offering.
pub async fn offering_slots(db: &PgPool, offering_id: i64) -> Res<Vec<SlotRow>> {
    sqlx::query_as::<_, SlotRow>(
        r#"SELECT t.id, t.weekday,
                  to_char(t.start_time, 'HH24:MI') AS start_at,
                  to_char(t.end_time, 'HH24:MI') AS end_at,
                  c.code, c.title AS course,
                  COALESCE(f.name, '') AS teacher,
                  COALESCE(t.room, '') AS room,
                  true AS is_offering
           FROM timetable_entries t
           JOIN courses c ON c.id = t.course_id
           LEFT JOIN faculty f ON f.id = t.faculty_id
           WHERE t.course_offering_id = $1
           ORDER BY t.weekday, t.start_time"#,
    )
    .bind(offering_id)
    .fetch_all(db)
    .await
}

pub struct NewOfferingSlot {
    pub offering_id: i64,
    pub faculty_id: i64, // 0 = none
    pub weekday: i16,
    pub start_at: String, // HH:MM
    pub end_at: String,   // HH:MM
    pub room: String,     // may be empty
}

/// Overlapping periods that share this offering's teacher, room or class.
///
/// "Same class" is the offering itself plus any programme class the offering
/// targets in the same semester: those students are booked twice otherwise.
pub async fn offering_clashes(db: &PgPool, n: &NewOfferingSlot) -> Res<Vec<Clash>> {
    sqlx::query_as::<_, Clash>(
        r#"SELECT COALESCE(t.faculty_id = NULLIF($3, 0), false) AS same_teacher,
                  COALESCE(t.room IS NOT NULL AND t.room <> '' AND lower(t.room) = lower($4), false) AS same_room,
                   (t.course_offering_id = $1)
                   OR (t.course_offering_id IS NOT NULL
                       AND t.semester = (SELECT o.semester FROM course_offerings o WHERE o.id = $1)
                       AND EXISTS (
                           SELECT 1
                             FROM course_offering_targets own_target
                             JOIN course_offering_targets other_target
                               ON other_target.programme_id = own_target.programme_id
                            WHERE own_target.offering_id = $1
                              AND other_target.offering_id = t.course_offering_id))
                   OR (t.programme_id IS NOT NULL AND t.semester = (SELECT o.semester FROM course_offerings o WHERE o.id = $1)
                      AND EXISTS (SELECT 1 FROM course_offering_targets tg
                                   WHERE tg.offering_id = $1 AND tg.programme_id = t.programme_id)) AS same_class,
                  c.title AS course,
                  to_char(t.start_time, 'HH24:MI') AS start_at,
                  to_char(t.end_time, 'HH24:MI') AS end_at
           FROM timetable_entries t JOIN courses c ON c.id = t.course_id
           WHERE t.weekday = $2
             AND t.start_time < $6::time AND t.end_time > $5::time
             AND ( COALESCE(t.faculty_id = NULLIF($3, 0), false)
                   OR COALESCE(t.room IS NOT NULL AND t.room <> '' AND lower(t.room) = lower($4), false)
                    OR t.course_offering_id = $1
                    OR (t.course_offering_id IS NOT NULL
                        AND t.semester = (SELECT o.semester FROM course_offerings o WHERE o.id = $1)
                        AND EXISTS (
                            SELECT 1
                              FROM course_offering_targets own_target
                              JOIN course_offering_targets other_target
                                ON other_target.programme_id = own_target.programme_id
                             WHERE own_target.offering_id = $1
                               AND other_target.offering_id = t.course_offering_id))
                    OR (t.programme_id IS NOT NULL
                       AND t.semester = (SELECT o.semester FROM course_offerings o WHERE o.id = $1)
                       AND EXISTS (SELECT 1 FROM course_offering_targets tg
                                    WHERE tg.offering_id = $1 AND tg.programme_id = t.programme_id)) )"#,
    )
    .bind(n.offering_id)
    .bind(n.weekday)
    .bind(n.faculty_id)
    .bind(&n.room)
    .bind(&n.start_at)
    .bind(&n.end_at)
    .fetch_all(db)
    .await
}

/// Add one period to a course offering. The row carries the offering's course
/// and semester and leaves `programme_id` NULL — the constraint on the table
/// says exactly one of the two is set.
pub async fn add_offering_slot(db: &PgPool, n: &NewOfferingSlot) -> Res<i64> {
    sqlx::query_scalar(
        r#"INSERT INTO timetable_entries
               (course_offering_id, programme_id, semester, course_id, faculty_id,
                weekday, start_time, end_time, room)
           SELECT $1, NULL, o.semester, o.course_id, NULLIF($3, 0), $2, $4::time, $5::time, NULLIF($6, '')
             FROM course_offerings o WHERE o.id = $1
           RETURNING id"#,
    )
    .bind(n.offering_id)
    .bind(n.weekday)
    .bind(n.faculty_id)
    .bind(&n.start_at)
    .bind(&n.end_at)
    .bind(&n.room)
    .fetch_one(db)
    .await
}
