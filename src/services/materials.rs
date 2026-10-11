//! Study materials: the files a teacher or head of department shares with the
//! students of a course.
//!
//! The `study_materials` table has existed since migration `0002` but had no
//! route, service or UI behind it. This module is the service layer.
//!
//! Access is scoped, not merely role-based:
//!
//! * an IT admin (`department` is `None`) manages every course;
//! * a head of department only manages courses owned by their department,
//!   whether the course carries the department itself or inherits it from its
//!   programme;
//! * a student only ever sees *published* material for a course they are
//!   actively enrolled in.
//!
//! The download path (`/uploads/...`) re-checks the same rules, so a guessed
//! file path is not enough to read an unpublished or unrelated file.

use sqlx::{FromRow, PgPool};

type Res<T> = Result<T, sqlx::Error>;

/// One material row as the list pages need it. The student query fills
/// `department` and `uploaded_by` with empty strings, because a student has no
/// business seeing either.
#[derive(Debug, FromRow)]
pub struct MaterialRow {
    pub id: i64,
    pub title: String,
    pub file_path: String,
    pub status: String,
    pub course_code: String,
    pub course_title: String,
    pub department: String,
    pub uploaded_by: String,
    pub created_label: String,
}

/// A course that may be picked on the upload form.
#[derive(Debug, FromRow)]
pub struct CourseChoice {
    pub id: i64,
    pub code: String,
    pub title: String,
}

/// Whether a manager scoped to `scope` may act on a course owned by
/// `course_department`. `None` is the IT admin, who may act on anything.
///
/// Kept as a pure function so the scope rule itself can be unit-tested without
/// a database.
pub fn scope_allows(scope: Option<i64>, course_department: Option<i64>) -> bool {
    match scope {
        None => true,
        Some(want) => course_department == Some(want),
    }
}

/// The department that owns a course: its own, or its programme's when the
/// course is programme-bound and carries no department of its own.
pub async fn course_department(db: &PgPool, course_id: i64) -> Res<Option<i64>> {
    sqlx::query_scalar(
        r#"SELECT COALESCE(c.department_id, p.department_id)
             FROM courses c
             LEFT JOIN programmes p ON p.id = c.programme_id
            WHERE c.id = $1"#,
    )
    .bind(course_id)
    .fetch_optional(db)
    .await
}

/// Every material a manager may see, newest first.
pub async fn for_manager(db: &PgPool, department: Option<i64>) -> Res<Vec<MaterialRow>> {
    sqlx::query_as::<_, MaterialRow>(
        r#"SELECT sm.id, sm.title, sm.file_path, sm.status,
                  c.code AS course_code, c.title AS course_title,
                  COALESCE(d.name, dp.name, 'Unassigned') AS department,
                  COALESCE(u.full_name, '') AS uploaded_by,
                  to_char(sm.created_at, 'YYYY-MM-DD') AS created_label
             FROM study_materials sm
             JOIN courses c ON c.id = sm.course_id
             LEFT JOIN programmes p ON p.id = c.programme_id
             LEFT JOIN departments d ON d.id = c.department_id
             LEFT JOIN departments dp ON dp.id = p.department_id
             LEFT JOIN users u ON u.id = sm.uploaded_by
            WHERE ($1::bigint IS NULL OR c.department_id = $1 OR p.department_id = $1)
            ORDER BY sm.created_at DESC, sm.id DESC"#,
    )
    .bind(department)
    .fetch_all(db)
    .await
}

/// The courses a manager may attach material to.
pub async fn courses_for_manager(db: &PgPool, department: Option<i64>) -> Res<Vec<CourseChoice>> {
    sqlx::query_as::<_, CourseChoice>(
        r#"SELECT c.id, c.code, c.title
             FROM courses c
             LEFT JOIN programmes p ON p.id = c.programme_id
            WHERE c.is_active
              AND ($1::bigint IS NULL OR c.department_id = $1 OR p.department_id = $1)
            ORDER BY c.code"#,
    )
    .bind(department)
    .fetch_all(db)
    .await
}

/// Published material for the courses the signed-in student is enrolled in.
pub async fn for_student(db: &PgPool, user_id: i64) -> Res<Vec<MaterialRow>> {
    sqlx::query_as::<_, MaterialRow>(
        r#"SELECT sm.id, sm.title, sm.file_path, sm.status,
                  c.code AS course_code, c.title AS course_title,
                  '' AS department, '' AS uploaded_by,
                  to_char(sm.created_at, 'YYYY-MM-DD') AS created_label
             FROM study_materials sm
             JOIN courses c ON c.id = sm.course_id
            WHERE sm.status = 'published'
              AND EXISTS (
                    SELECT 1
                      FROM enrollments e
                      JOIN students st ON st.id = e.student_id
                     WHERE st.user_id = $1
                       AND st.is_active
                       AND e.course_id = sm.course_id
                       AND e.status = 'active')
            ORDER BY c.code, sm.created_at DESC, sm.id DESC"#,
    )
    .bind(user_id)
    .fetch_all(db)
    .await
}

pub async fn create(
    db: &PgPool,
    course_id: i64,
    title: &str,
    file_path: &str,
    uploaded_by: i64,
    status: &str,
) -> Res<i64> {
    sqlx::query_scalar(
        r#"INSERT INTO study_materials (course_id, title, file_path, uploaded_by, status)
           VALUES ($1, $2, $3, $4, $5) RETURNING id"#,
    )
    .bind(course_id)
    .bind(title)
    .bind(file_path)
    .bind(uploaded_by)
    .bind(status)
    .fetch_one(db)
    .await
}

pub async fn set_status(db: &PgPool, id: i64, status: &str) -> Res<()> {
    sqlx::query("UPDATE study_materials SET status = $2 WHERE id = $1")
        .bind(id)
        .bind(status)
        .execute(db)
        .await?;
    Ok(())
}

/// The stored file path for a row, used to clean the file up before deleting.
pub async fn file_path(db: &PgPool, id: i64) -> Res<Option<String>> {
    sqlx::query_scalar("SELECT file_path FROM study_materials WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await
}

/// The department that owns a material row, so a manager's scope can be
/// re-checked before a status change or delete.
pub async fn material_department(db: &PgPool, id: i64) -> Res<Option<Option<i64>>> {
    let dept: Option<Option<i64>> = sqlx::query_scalar(
        r#"SELECT COALESCE(c.department_id, p.department_id)
             FROM study_materials sm
             JOIN courses c ON c.id = sm.course_id
             LEFT JOIN programmes p ON p.id = c.programme_id
            WHERE sm.id = $1"#,
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    Ok(dept)
}

pub async fn delete(db: &PgPool, id: i64) -> Res<()> {
    sqlx::query("DELETE FROM study_materials WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::scope_allows;

    #[test]
    fn the_it_admin_may_manage_any_course() {
        assert!(scope_allows(None, None));
        assert!(scope_allows(None, Some(4)));
    }

    #[test]
    fn a_department_manager_only_matches_their_own_department() {
        assert!(scope_allows(Some(3), Some(3)));
        assert!(!scope_allows(Some(3), Some(4)));
        // A course with no resolvable department must not be treated as global.
        assert!(!scope_allows(Some(3), None));
    }
}

/// The courses a teacher is attached to: the course owner, or any course they
/// have a timetable period for.
pub async fn courses_taught(db: &PgPool, faculty_id: i64) -> Res<Vec<CourseChoice>> {
    sqlx::query_as::<_, CourseChoice>(
        r#"SELECT DISTINCT c.id, c.code, c.title
             FROM courses c
            WHERE c.is_active
              AND (c.faculty_id = $1
                   OR EXISTS (SELECT 1 FROM timetable_entries te
                               WHERE te.course_id = c.id AND te.faculty_id = $1))
            ORDER BY c.code"#,
    )
    .bind(faculty_id)
    .fetch_all(db)
    .await
}

/// Whether `faculty_id` teaches `course_id`, used to re-check an upload.
pub async fn teacher_teaches(db: &PgPool, faculty_id: i64, course_id: i64) -> Res<bool> {
    sqlx::query_scalar(
        r#"SELECT EXISTS (
               SELECT 1 FROM courses c
                WHERE c.id = $2
                  AND (c.faculty_id = $1
                       OR EXISTS (SELECT 1 FROM timetable_entries te
                                   WHERE te.course_id = c.id AND te.faculty_id = $1)))"#,
    )
    .bind(faculty_id)
    .bind(course_id)
    .fetch_one(db)
    .await
}

/// Every material for the courses a teacher is attached to, newest first.
pub async fn for_teacher(db: &PgPool, faculty_id: i64) -> Res<Vec<MaterialRow>> {
    sqlx::query_as::<_, MaterialRow>(
        r#"SELECT sm.id, sm.title, sm.file_path, sm.status,
                  c.code AS course_code, c.title AS course_title,
                  COALESCE(d.name, dp.name, 'Unassigned') AS department,
                  COALESCE(u.full_name, '') AS uploaded_by,
                  to_char(sm.created_at, 'YYYY-MM-DD') AS created_label
             FROM study_materials sm
             JOIN courses c ON c.id = sm.course_id
             LEFT JOIN programmes p ON p.id = c.programme_id
             LEFT JOIN departments d ON d.id = c.department_id
             LEFT JOIN departments dp ON dp.id = p.department_id
             LEFT JOIN users u ON u.id = sm.uploaded_by
            WHERE c.faculty_id = $1
               OR EXISTS (SELECT 1 FROM timetable_entries te
                           WHERE te.course_id = c.id AND te.faculty_id = $1)
            ORDER BY sm.created_at DESC, sm.id DESC"#,
    )
    .bind(faculty_id)
    .fetch_all(db)
    .await
}
