use sqlx::{FromRow, PgPool};

type Res<T> = Result<T, sqlx::Error>;

#[derive(Debug, FromRow)]
pub struct PersonRow {
    pub id: i64,
    pub full_name: String,
    pub email: String,
    pub role: String,
    pub is_active: bool,
    pub admission_no: String,
    pub detail: String,
}

pub async fn list(db: &PgPool, role: &str, q: &str) -> Res<Vec<PersonRow>> {
    sqlx::query_as::<_, PersonRow>(
        r#"SELECT u.id, u.full_name, u.email, u.role, u.is_active,
                  COALESCE(s.admission_no, '') AS admission_no,
                  CASE WHEN s.id IS NOT NULL THEN p.name || ', semester ' || s.semester::text
                       ELSE COALESCE(d.name, '') END AS detail
           FROM users u
           LEFT JOIN students s ON s.user_id = u.id
           LEFT JOIN programmes p ON p.id = s.programme_id
           LEFT JOIN faculty f ON f.user_id = u.id
           LEFT JOIN departments d ON d.id = f.department_id
           WHERE ($1 = '' OR u.role = $1)
             AND ($2 = '' OR u.full_name ILIKE '%' || $2 || '%'
                          OR u.email ILIKE '%' || $2 || '%'
                          OR s.admission_no ILIKE '%' || $2 || '%')
           ORDER BY u.full_name, u.id
           LIMIT 300"#,
    )
    .bind(role)
    .bind(q)
    .fetch_all(db)
    .await
}

#[derive(Debug, FromRow)]
pub struct PersonDetail {
    pub id: i64,
    pub full_name: String,
    pub email: String,
    pub role: String,
    pub is_active: bool,
    pub admission_no: String,
    /// The university's permanent registration number (candidate key).
    pub prn: String,
    pub programme_id: i64,
    pub batch_year: i32,
    pub semester: i32,
    pub phone: String,
    pub department_id: i64,
    pub designation: String,
    pub qualification: String,
    pub is_hod: bool,
    pub can_manage: bool,
}

pub async fn detail(db: &PgPool, user_id: i64) -> Res<Option<PersonDetail>> {
    sqlx::query_as::<_, PersonDetail>(
        r#"SELECT u.id, u.full_name, u.email, u.role, u.is_active,
                  COALESCE(s.admission_no, '') AS admission_no,
                  COALESCE(s.prn, '') AS prn,
                  COALESCE(s.programme_id, 0) AS programme_id,
                  COALESCE(s.batch_year, 0) AS batch_year,
                  COALESCE(s.semester, 1) AS semester,
                  COALESCE(s.phone, '') AS phone,
                  COALESCE(f.department_id, 0) AS department_id,
                  COALESCE(f.designation, '') AS designation,
                  COALESCE(f.qualification, '') AS qualification,
                  COALESCE(f.is_hod, false) AS is_hod,
                  COALESCE(f.can_manage, false) AS can_manage
           FROM users u
           LEFT JOIN students s ON s.user_id = u.id
           LEFT JOIN faculty f ON f.user_id = u.id
           WHERE u.id = $1"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
}

pub struct NewStudent<'a> {
    pub admission_no: &'a str,
    /// The permanent registration number; empty means none on file.
    pub prn: &'a str,
    pub name: &'a str,
    pub email: &'a str,
    pub programme_id: i64,
    pub batch_year: i32,
    pub semester: i32,
    pub phone: &'a str,
}

const ENROLL_SEMESTER_SQL: &str = r#"INSERT INTO enrollments (student_id, course_id, semester, offering_id)
    SELECT $1, id, $3,
           -- The fixed offering behind this row, when there is one, so the
           -- offering's seat count sees the enrollment.
           (SELECT o.id FROM course_offerings o
             WHERE o.course_id = courses.id AND o.status = 'published'
               AND o.selection_mode = 'FIXED' AND o.semester = $3
             ORDER BY o.id LIMIT 1)
      FROM courses WHERE programme_id = $2 AND semester = $3
      AND NOT EXISTS (
          SELECT 1 FROM course_offerings o
           WHERE o.course_id = courses.id
             AND o.status = 'published'
             AND o.selection_mode <> 'FIXED')
    ON CONFLICT DO NOTHING"#;

/// Creates the login, the student record, and enrolls the student in the semester's courses.
pub async fn create_student(db: &PgPool, s: &NewStudent<'_>, password_hash: &str) -> Res<i64> {
    let mut tx = db.begin().await?;
    let user_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO users (email, full_name, role, password_hash, must_change_password)
           VALUES ($1, $2, 'student', $3, true) RETURNING id"#,
    )
    .bind(s.email)
    .bind(s.name)
    .bind(password_hash)
    .fetch_one(&mut *tx)
    .await?;
    let student_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO students (user_id, admission_no, prn, name, programme_id, batch_year, semester, phone)
           VALUES ($1, $2, NULLIF($3, ''), $4, $5, $6, $7, NULLIF($8, '')) RETURNING id"#,
    )
    .bind(user_id)
    .bind(s.admission_no)
    .bind(s.prn)
    .bind(s.name)
    .bind(s.programme_id)
    .bind(s.batch_year)
    .bind(s.semester)
    .bind(s.phone)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(ENROLL_SEMESTER_SQL)
        .bind(student_id)
        .bind(s.programme_id)
        .bind(s.semester)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(user_id)
}

/// The highest semester a student can be moved to: the edit form goes up to
/// eight, and a student there is at the end of the course.
pub const MAX_SEMESTER: i32 = 8;

/// What happened when a student was asked to move up a semester.
pub enum Promoted {
    /// Moved into the returned semester.
    To(i32),
    /// Already in the final semester.
    AtEnd,
    /// The account is not a student, so there is nothing to promote.
    NotAStudent,
}

/// Moves one student up one semester at the end of a term. Only the student's
/// own semester moves: enrollments and results stay filed under the semester
/// they belong to, so the Results page keeps every past semester.
pub async fn promote(db: &PgPool, user_id: i64) -> Res<Promoted> {
    let current: Option<i32> =
        sqlx::query_scalar("SELECT semester FROM students WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(db)
            .await?;
    let Some(current) = current else {
        return Ok(Promoted::NotAStudent);
    };
    if current >= MAX_SEMESTER {
        return Ok(Promoted::AtEnd);
    }
    sqlx::query("UPDATE students SET semester = semester + 1 WHERE user_id = $1")
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(Promoted::To(current + 1))
}

/// Promotes every active student of a programme who is sitting in
/// `from_semester` — the end-of-term roll for a whole class. Returns how many
/// moved.
pub async fn promote_cohort(db: &PgPool, programme_id: i64, from_semester: i32) -> Res<u64> {
    let done = sqlx::query(
        r#"UPDATE students SET semester = semester + 1
           WHERE programme_id = $1 AND semester = $2
             AND is_active AND semester < $3"#,
    )
    .bind(programme_id)
    .bind(from_semester)
    .bind(MAX_SEMESTER)
    .execute(db)
    .await?;
    Ok(done.rows_affected())
}

pub struct NewTeacher<'a> {
    pub name: &'a str,
    pub email: &'a str,
    pub department_id: i64,
    pub designation: &'a str,
    pub qualification: &'a str,
    pub is_hod: bool,
    pub can_manage: bool,
}

pub async fn create_teacher(db: &PgPool, t: &NewTeacher<'_>, password_hash: &str) -> Res<i64> {
    let mut tx = db.begin().await?;
    let user_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO users (email, full_name, role, password_hash, must_change_password)
           VALUES ($1, $2, 'faculty', $3, true) RETURNING id"#,
    )
    .bind(t.email)
    .bind(t.name)
    .bind(password_hash)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO faculty (user_id, department_id, name, designation, qualification, email, is_hod, can_manage, status)
           VALUES ($1, NULLIF($2, 0), $3, $4, $5, $6, $7, $8, 'published')"#,
    )
    .bind(user_id)
    .bind(t.department_id)
    .bind(t.name)
    .bind(t.designation)
    .bind(t.qualification)
    .bind(t.email)
    .bind(t.is_hod)
    .bind(t.can_manage)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(user_id)
}

pub async fn create_staff(db: &PgPool, name: &str, email: &str, password_hash: &str) -> Res<i64> {
    sqlx::query_scalar(
        r#"INSERT INTO users (email, full_name, role, password_hash, must_change_password)
           VALUES ($1, $2, 'staff', $3, true) RETURNING id"#,
    )
    .bind(email)
    .bind(name)
    .bind(password_hash)
    .fetch_one(db)
    .await
}

/// Updates the login plus whichever profile (student or teacher) the user has.
pub async fn update(db: &PgPool, d: &PersonDetail) -> Res<()> {
    let mut tx = db.begin().await?;
    sqlx::query("UPDATE users SET full_name = $2, email = $3 WHERE id = $1")
        .bind(d.id)
        .bind(&d.full_name)
        .bind(&d.email)
        .execute(&mut *tx)
        .await?;

    if d.role == "student" {
        let student_id: Option<i64> = sqlx::query_scalar(
            r#"UPDATE students
               SET name = $2, programme_id = $3, batch_year = $4, semester = $5,
                   phone = NULLIF($6, ''), prn = NULLIF($7, '')
               WHERE user_id = $1 RETURNING id"#,
        )
        .bind(d.id)
        .bind(&d.full_name)
        .bind(d.programme_id)
        .bind(d.batch_year)
        .bind(d.semester)
        .bind(&d.phone)
        .bind(&d.prn)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(student_id) = student_id {
            sqlx::query(ENROLL_SEMESTER_SQL)
                .bind(student_id)
                .bind(d.programme_id)
                .bind(d.semester)
                .execute(&mut *tx)
                .await?;
        }
    } else if d.role == "faculty" {
        sqlx::query(
            r#"UPDATE faculty
               SET name = $2, email = $3, department_id = NULLIF($4, 0),
                   designation = $5, qualification = $6, is_hod = $7, can_manage = $8
               WHERE user_id = $1"#,
        )
        .bind(d.id)
        .bind(&d.full_name)
        .bind(&d.email)
        .bind(d.department_id)
        .bind(&d.designation)
        .bind(&d.qualification)
        .bind(d.is_hod)
        .bind(d.can_manage)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn set_active(db: &PgPool, user_id: i64, active: bool) -> Res<()> {
    let mut tx = db.begin().await?;
    sqlx::query("UPDATE users SET is_active = $2 WHERE id = $1")
        .bind(user_id)
        .bind(active)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE students SET is_active = $2 WHERE user_id = $1")
        .bind(user_id)
        .bind(active)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
