//! Course offerings and student course selection.
//!
//! The flow is: an offering department's HOD publishes an offering to
//! programmes/cohorts, each receiving department's HOD approves it, and
//! eligible students then select, review and confirm. A confirmed selection
//! creates the existing `enrollments` row, and timetable/attendance follow the
//! enrollment. Confirmed choices move only through change requests, which keep
//! who decided what, when and why.
//!
//! Like the rest of the services, queries are runtime `query_as` calls so the
//! project builds without a live database.

use sqlx::{FromRow, PgPool};

use crate::error::AppError;

type Res<T> = Result<T, sqlx::Error>;

// A selection moves through these states: `draft` on the student's sheet,
// `confirmed` once accepted, `locked` by the HOD, `change_requested` while a
// move is pending (`submitted` is reserved for a review step).

/// Whether the student row `st` may take the offering row `o` (joined to the
/// offering's academic year as `y`). The single source of truth for
/// eligibility: the student's course list, selecting/confirming, the HOD's
/// assignment, and which students a FIXED offering applies itself to all use
/// this, so the rules cannot drift apart between them.
///
/// Note the target rule: a whole-programme target has `cohort_id` NULL and
/// matches every batch of that programme, while a cohort target only matches
/// students whose programme *and* batch year line up — so targeting the BCA
/// 2025 batch never leaks the offering to BCA 2024.
const ELIGIBLE: &str = r#"
    o.status = 'published'
    AND y.is_current
    AND o.semester = st.semester
    AND EXISTS (
        SELECT 1 FROM course_offering_targets t
         WHERE t.offering_id = o.id
           AND ( (t.cohort_id IS NULL AND t.programme_id = st.programme_id)
              OR EXISTS (SELECT 1 FROM cohorts cb
                          WHERE cb.id = t.cohort_id
                            AND cb.programme_id = st.programme_id
                            AND cb.batch_year = st.batch_year))
    )
    AND (
        o.offering_department_id = COALESCE(st.department_id,
            (SELECT p.department_id FROM programmes p WHERE p.id = st.programme_id))
        OR EXISTS (SELECT 1 FROM course_offering_approvals a
                    WHERE a.offering_id = o.id
                      AND a.department_id = COALESCE(st.department_id,
                          (SELECT p.department_id FROM programmes p WHERE p.id = st.programme_id))
                      AND a.status = 'approved')
    )
    AND (
        o.selection_mode <> 'COHORT_CHOICE'
        OR EXISTS (SELECT 1 FROM cohorts cb
                    WHERE cb.programme_id = st.programme_id
                      AND cb.batch_year = st.batch_year
                      AND EXISTS (SELECT 1 FROM cohort_specializations cs
                                   WHERE cs.cohort_id = cb.id
                                     AND cs.offering_id = o.id))
    )"#;

// ---------- Academic years ----------

#[derive(Debug, Clone, FromRow)]
pub struct AcademicYear {
    pub id: i64,
    pub label: String,
    pub start_year: i32,
    pub is_current: bool,
}

pub async fn academic_years(db: &PgPool) -> Res<Vec<AcademicYear>> {
    sqlx::query_as::<_, AcademicYear>(
        "SELECT id, label, start_year, is_current FROM academic_years ORDER BY start_year DESC",
    )
    .fetch_all(db)
    .await
}

// ---------- Catalogue courses ----------

#[derive(Debug, FromRow)]
pub struct CatalogueCourse {
    pub id: i64,
    pub code: String,
    pub title: String,
    pub credits: i32,
    pub category: String,
    pub semester: Option<i32>,
    pub is_active: bool,
    pub department: String,
    pub programme: Option<String>,
}

pub async fn catalogue_courses(db: &PgPool, department: Option<i64>) -> Res<Vec<CatalogueCourse>> {
    sqlx::query_as::<_, CatalogueCourse>(
        r#"SELECT c.id, c.code, c.title, c.credits, c.category,
                  c.semester, c.is_active,
                  COALESCE(d.name, '') AS department,
                  p.name AS programme
           FROM courses c
           LEFT JOIN departments d ON d.id = c.department_id
           LEFT JOIN programmes p ON p.id = c.programme_id
           WHERE ($1::bigint IS NULL OR c.department_id = $1)
           ORDER BY c.programme_id NULLS FIRST, c.code"#,
    )
    .bind(department)
    .fetch_all(db)
    .await
}

/// Courses this department owns: its catalogue entries plus any
/// programme-bound courses of its own programmes.
pub async fn department_courses(db: &PgPool, department_id: i64) -> Res<Vec<CatalogueCourse>> {
    sqlx::query_as::<_, CatalogueCourse>(
        r#"SELECT c.id, c.code, c.title, c.credits, c.category,
                  c.semester, c.is_active,
                  COALESCE(d.name, '') AS department,
                  p.name AS programme
           FROM courses c
           LEFT JOIN departments d ON d.id = c.department_id
           LEFT JOIN programmes p ON p.id = c.programme_id
           WHERE c.department_id = $1
           ORDER BY c.programme_id NULLS FIRST, c.code"#,
    )
    .bind(department_id)
    .fetch_all(db)
    .await
}

pub struct CourseInput {
    pub code: String,
    pub title: String,
    pub credits: i32,
    pub category: String,
    pub semester: i32, // 0 = no fixed semester (catalogue course)
}

pub async fn create_catalogue_course(
    db: &PgPool,
    department_id: i64,
    c: &CourseInput,
) -> Res<i64> {
    sqlx::query_scalar(
        r#"INSERT INTO courses (department_id, programme_id, code, title, credits, category, semester, faculty_id)
           VALUES ($1, NULL, $2, $3, $4, $5, NULLIF($6, 0), NULL)
           RETURNING id"#,
    )
    .bind(department_id)
    .bind(&c.code)
    .bind(&c.title)
    .bind(c.credits)
    .bind(&c.category)
    .bind(c.semester)
    .fetch_one(db)
    .await
}

pub async fn update_catalogue_course(db: &PgPool, id: i64, c: &CourseInput) -> Res<()> {
    sqlx::query(
        r#"UPDATE courses
              SET code = $2, title = $3, credits = $4, category = $5, semester = NULLIF($6, 0)
            WHERE id = $1 AND department_id IS NOT NULL AND programme_id IS NULL"#,
    )
    .bind(id)
    .bind(&c.code)
    .bind(&c.title)
    .bind(c.credits)
    .bind(&c.category)
    .bind(c.semester)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn set_course_active(db: &PgPool, id: i64, is_active: bool) -> Res<()> {
    sqlx::query("UPDATE courses SET is_active = $2 WHERE id = $1 AND department_id IS NOT NULL AND programme_id IS NULL")
        .bind(id)
        .bind(is_active)
        .execute(db)
        .await?;
    Ok(())
}

/// The department that owns a course, for permission checks.
pub async fn course_department(db: &PgPool, course_id: i64) -> Res<Option<i64>> {
    sqlx::query_scalar("SELECT department_id FROM courses WHERE id = $1")
        .bind(course_id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| sqlx::Error::RowNotFound)
}

// ---------- Offerings ----------

#[derive(Debug, Clone, FromRow)]
pub struct Offering {
    pub id: i64,
    pub course_id: i64,
    pub code: String,
    pub title: String,
    pub credits: i32,
    pub category: String,
    pub offering_department_id: i64,
    pub department: String,
    pub academic_year_id: i64,
    pub year_label: String,
    pub semester: i32,
    pub course_type: String,
    pub selection_mode: String,
    pub choice_group: String,
    pub capacity: Option<i32>,
    pub faculty_name: String,
    pub status: String,
    pub enrolled: i64,
    pub has_target: bool,
}

pub async fn offering(db: &PgPool, id: i64) -> Res<Option<Offering>> {
    sqlx::query_as::<_, Offering>(
        r#"SELECT o.id, o.course_id, c.code, c.title, c.credits, c.category,
                  o.offering_department_id, d.name AS department,
                  o.academic_year_id, y.label AS year_label, o.semester,
                  o.course_type, o.selection_mode, o.choice_group, o.capacity,
                  COALESCE(f.name, '') AS faculty_name, o.status,
                  (SELECT count(*) FROM enrollments e
                    JOIN students est ON est.id = e.student_id AND est.is_active
                   WHERE e.offering_id = o.id AND e.status = 'active') AS enrolled,
                  EXISTS (SELECT 1 FROM course_offering_targets t WHERE t.offering_id = o.id) AS has_target
           FROM course_offerings o
           JOIN courses c ON c.id = o.course_id
           JOIN departments d ON d.id = o.offering_department_id
           JOIN academic_years y ON y.id = o.academic_year_id
           LEFT JOIN faculty f ON f.id = o.faculty_id
           WHERE o.id = $1"#,
    )
    .bind(id)
    .fetch_optional(db)
    .await
}

#[derive(Debug, FromRow)]
pub struct OfferingRow {
    pub id: i64,
    pub code: String,
    pub title: String,
    pub credits: i32,
    pub category: String,
    pub department: String,
    pub year_label: String,
    pub semester: i32,
    pub course_type: String,
    pub selection_mode: String,
    pub choice_group: String,
    pub capacity: Option<i32>,
    pub status: String,
    pub enrolled: i64,
}

pub async fn offerings_for_department(db: &PgPool, department: Option<i64>) -> Res<Vec<OfferingRow>> {
    sqlx::query_as::<_, OfferingRow>(
        r#"SELECT o.id, c.code, c.title, c.credits, c.category,
                  d.name AS department, y.label AS year_label, o.semester,
                  o.course_type, o.selection_mode, o.choice_group, o.capacity, o.status,
                  (SELECT count(*) FROM enrollments e
                    JOIN students est ON est.id = e.student_id AND est.is_active
                   WHERE e.offering_id = o.id AND e.status = 'active') AS enrolled
           FROM course_offerings o
           JOIN courses c ON c.id = o.course_id
           JOIN departments d ON d.id = o.offering_department_id
           JOIN academic_years y ON y.id = o.academic_year_id
           WHERE ($1::bigint IS NULL OR o.offering_department_id = $1)
           ORDER BY y.start_year DESC, o.semester, c.code"#,
    )
    .bind(department)
    .fetch_all(db)
    .await
}

/// Published offerings whose periods a manager may schedule right now: their
/// own department's, or every department's for the IT admin. Drives the
/// timetable page's offering picker.
#[derive(Debug, FromRow)]
pub struct SchedulableOffering {
    pub id: i64,
    pub code: String,
    pub title: String,
    pub year_label: String,
    pub semester: i32,
    pub department: String,
}

pub async fn schedulable_offerings(db: &PgPool, department: Option<i64>) -> Res<Vec<SchedulableOffering>> {
    sqlx::query_as::<_, SchedulableOffering>(
        r#"SELECT o.id, c.code, c.title, y.label AS year_label, o.semester,
                  od.name AS department
             FROM course_offerings o
             JOIN courses c ON c.id = o.course_id
             JOIN academic_years y ON y.id = o.academic_year_id
             JOIN departments od ON od.id = o.offering_department_id
            WHERE o.status = 'published'
              AND ($1::bigint IS NULL OR o.offering_department_id = $1)
            ORDER BY y.start_year DESC, o.semester, c.code"#,
    )
    .bind(department)
    .fetch_all(db)
    .await
}

// ---------- The work queue (one page for everything awaiting a decision) ----------

/// A published offering with no timetable periods: students can select it and
/// enroll, but there is no class to teach or take attendance for.
#[derive(Debug, FromRow)]
pub struct UnscheduledOffering {
    pub id: i64,
    pub code: String,
    pub title: String,
    pub year_label: String,
    pub semester: i32,
    pub department: String,
    pub selection_mode: String,
}

pub async fn unscheduled_offerings(db: &PgPool, department: Option<i64>) -> Res<Vec<UnscheduledOffering>> {
    sqlx::query_as::<_, UnscheduledOffering>(
        r#"SELECT o.id, c.code, c.title, y.label AS year_label, o.semester,
                  od.name AS department, o.selection_mode
             FROM course_offerings o
             JOIN courses c ON c.id = o.course_id
             JOIN academic_years y ON y.id = o.academic_year_id
             JOIN departments od ON od.id = o.offering_department_id
            WHERE o.status = 'published'
              AND ($1::bigint IS NULL OR o.offering_department_id = $1)
              AND NOT EXISTS (
                  SELECT 1 FROM timetable_entries t WHERE t.course_offering_id = o.id)
            ORDER BY y.start_year DESC, o.semester, c.code"#,
    )
    .bind(department)
    .fetch_all(db)
    .await
}

/// Published offerings whose seats are gone. Seats are counted from
/// `enrollments.offering_id`, so only this offering's own enrollments fill it.
#[derive(Debug, FromRow)]
pub struct CapacityWarning {
    pub id: i64,
    pub code: String,
    pub title: String,
    pub year_label: String,
    pub semester: i32,
    pub department: String,
    pub capacity: i64,
    pub enrolled: i64,
}

pub async fn capacity_warnings(db: &PgPool, department: Option<i64>) -> Res<Vec<CapacityWarning>> {
    sqlx::query_as::<_, CapacityWarning>(
        r#"SELECT w.id, w.code, w.title, w.year_label, w.semester, w.department,
                  w.capacity, w.enrolled
             FROM (
                 SELECT o.id, c.code, c.title, y.label AS year_label, o.semester,
                        od.name AS department, o.capacity::bigint AS capacity,
                        (SELECT count(*) FROM enrollments e
                           JOIN students est ON est.id = e.student_id AND est.is_active
                          WHERE e.offering_id = o.id AND e.status = 'active') AS enrolled
                   FROM course_offerings o
                   JOIN courses c ON c.id = o.course_id
                   JOIN academic_years y ON y.id = o.academic_year_id
                   JOIN departments od ON od.id = o.offering_department_id
                  WHERE o.status = 'published'
                    AND o.capacity IS NOT NULL
                    AND ($1::bigint IS NULL OR o.offering_department_id = $1)
             ) w
            WHERE w.enrolled >= w.capacity
            ORDER BY (w.enrolled - w.capacity) DESC, w.code"#,
    )
    .bind(department)
    .fetch_all(db)
    .await
}

pub struct NewOffering {
    pub course_id: i64,
    pub offering_department_id: i64,
    pub academic_year_id: i64,
    pub semester: i32,
    pub course_type: String,
    pub selection_mode: String,
    pub choice_group: String,
    pub capacity: Option<i32>,
    pub faculty_id: Option<i64>,
}

pub async fn create_offering(db: &PgPool, n: &NewOffering) -> Res<i64> {
    sqlx::query_scalar(
        r#"INSERT INTO course_offerings
               (course_id, offering_department_id, academic_year_id, semester,
                course_type, selection_mode, choice_group, capacity, faculty_id, status)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'draft')
           RETURNING id"#,
    )
    .bind(n.course_id)
    .bind(n.offering_department_id)
    .bind(n.academic_year_id)
    .bind(n.semester)
    .bind(&n.course_type)
    .bind(&n.selection_mode)
    .bind(&n.choice_group)
    .bind(n.capacity)
    .bind(n.faculty_id)
    .fetch_one(db)
    .await
}

pub async fn update_offering(
    db: &PgPool,
    id: i64,
    semester: i32,
    course_type: &str,
    selection_mode: &str,
    choice_group: &str,
    capacity: Option<i32>,
) -> Res<()> {
    sqlx::query(
        r#"UPDATE course_offerings
              SET semester = $2, course_type = $3, selection_mode = $4,
                  choice_group = $5, capacity = $6
            WHERE id = $1"#,
    )
    .bind(id)
    .bind(semester)
    .bind(course_type)
    .bind(selection_mode)
    .bind(choice_group)
    .bind(capacity)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn set_offering_status(db: &PgPool, id: i64, status: &str) -> Res<()> {
    if status == "published" {
        sqlx::query("UPDATE course_offerings SET status = $2, published_at = COALESCE(published_at, now()) WHERE id = $1")
            .bind(id)
            .bind(status)
            .execute(db)
            .await?;
    } else {
        sqlx::query("UPDATE course_offerings SET status = $2 WHERE id = $1")
            .bind(id)
            .bind(status)
            .execute(db)
            .await?;
    }
    Ok(())
}

pub async fn delete_offering(db: &PgPool, id: i64) -> Res<bool> {
    let done = sqlx::query("DELETE FROM course_offerings WHERE id = $1 AND status = 'draft'")
        .bind(id)
        .execute(db)
        .await?;
    Ok(done.rows_affected() > 0)
}

// ---------- Targets ----------

/// The programmes this offering targets, for display and the approval worklist.
#[derive(Debug, FromRow)]
pub struct TargetProgramme {
    pub programme_id: i64,
    pub programme: String,
    pub department_id: i64,
    pub department: String,
    pub cohort_id: Option<i64>,
    pub batch_year: Option<i32>,
}

pub async fn offering_targets(db: &PgPool, offering_id: i64) -> Res<Vec<TargetProgramme>> {
    sqlx::query_as::<_, TargetProgramme>(
        r#"SELECT t.programme_id, p.name AS programme,
                  p.department_id, d.name AS department,
                  t.cohort_id, cb.batch_year
           FROM course_offering_targets t
           JOIN programmes p ON p.id = t.programme_id
           JOIN departments d ON d.id = p.department_id
           LEFT JOIN cohorts cb ON cb.id = t.cohort_id
           WHERE t.offering_id = $1
           ORDER BY d.name, p.name"#,
    )
    .bind(offering_id)
    .fetch_all(db)
    .await
}

pub async fn replace_targets(
    db: &PgPool,
    offering_id: i64,
    programme_ids: &[i64],
    cohort_id: Option<i64>,
) -> Res<()> {
    sqlx::query("DELETE FROM course_offering_targets WHERE offering_id = $1")
        .bind(offering_id)
        .execute(db)
        .await?;
    sqlx::query(
        r#"INSERT INTO course_offering_targets (offering_id, programme_id, cohort_id)
           SELECT $1, pid, $2 FROM UNNEST($3::bigint[]) AS pid
           ON CONFLICT DO NOTHING"#,
    )
    .bind(offering_id)
    .bind(cohort_id)
    .bind(programme_ids)
    .execute(db)
    .await?;
    Ok(())
}

// ---------- Approvals ----------

#[derive(Debug, FromRow)]
pub struct Approval {
    pub id: i64,
    pub offering_id: i64,
    pub code: String,
    pub title: String,
    pub credits: i32,
    pub department: String,
    pub year_label: String,
    pub semester: i32,
    pub course_type: String,
    pub selection_mode: String,
    pub choice_group: String,
    pub capacity: Option<i32>,
    pub status: String,
    pub note: String,
}

/// External offerings aimed at this department, for the receiving HOD's view.
/// `department` is `None` for the IT admin, who sees every department's
/// worklist. The offering department's own rows are never listed here: you do
/// not approve your own course for yourself.
pub async fn external_offerings_for(db: &PgPool, department: Option<i64>) -> Res<Vec<Approval>> {
    sqlx::query_as::<_, Approval>(
        r#"SELECT a.id, a.offering_id, c.code, c.title, c.credits,
                  od.name AS department, y.label AS year_label, o.semester,
                  o.course_type, o.selection_mode, o.choice_group, o.capacity,
                  a.status, a.note
           FROM course_offering_approvals a
           JOIN course_offerings o ON o.id = a.offering_id
           JOIN courses c ON c.id = o.course_id
           JOIN departments od ON od.id = o.offering_department_id
           JOIN academic_years y ON y.id = o.academic_year_id
           WHERE ($1::bigint IS NULL OR a.department_id = $1)
             AND a.department_id <> o.offering_department_id
           ORDER BY (a.status = 'pending') DESC, y.start_year DESC, c.code"#,
    )
    .bind(department)
    .fetch_all(db)
    .await
}

/// Decide an approval. The offering department cannot approve its own offering
/// for another department; only the receiving department's HOD (or the admin)
/// can. Returns false when the approval row does not exist.
pub async fn decide_approval(
    db: &PgPool,
    approval_id: i64,
    department: Option<i64>,
    status: &str,
    note: &str,
    decided_by: i64,
) -> Res<bool> {
    let updated = sqlx::query(
        r#"UPDATE course_offering_approvals
              SET status = $3, note = $4, decided_by = $5, decided_at = now()
            WHERE id = $1 AND ($2::bigint IS NULL OR department_id = $2)"#,
    )
    .bind(approval_id)
    .bind(department)
    .bind(status)
    .bind(note)
    .bind(decided_by)
    .execute(db)
    .await?;
    Ok(updated.rows_affected() > 0)
}

/// When the offering is published, create a pending approval for each
/// department that has a targeted programme. The offering department never
/// approves its own offering: its students are eligible without a decision.
pub async fn sync_approvals(db: &PgPool, offering_id: i64) -> Res<()> {
    sqlx::query(
        r#"INSERT INTO course_offering_approvals (offering_id, department_id)
           SELECT DISTINCT $1, p.department_id
             FROM course_offering_targets t
             JOIN programmes p ON p.id = t.programme_id
             JOIN course_offerings o ON o.id = t.offering_id
            WHERE t.offering_id = $1
              AND p.department_id <> o.offering_department_id
           ON CONFLICT (offering_id, department_id) DO NOTHING"#,
    )
    .bind(offering_id)
    .execute(db)
    .await?;
    Ok(())
}

// ---------- Student eligibility ----------

/// Whether a student may see and select this offering:
///   * the offering is published for the student's academic year and semester;
///   * it targets their programme, or their cohort (programme + batch year);
///   * the receiving department approved it (self-offerings skip this);
///   * for a COHORT_CHOICE, the cohort has a finalised specialization.
pub async fn eligible(db: &PgPool, student_id: i64, offering_id: i64) -> Res<bool> {
    sqlx::query_scalar::<_, bool>(&format!(
        r#"SELECT EXISTS (
               SELECT 1
                 FROM course_offerings o
                 JOIN students st ON st.id = $1 AND st.is_active
                 JOIN academic_years y ON y.id = o.academic_year_id
                WHERE o.id = $2
                  AND {ELIGIBLE}
           )"#,
    ))
    .bind(student_id)
    .bind(offering_id)
    .fetch_one(db)
    .await
}

/// One offering on the student's selection page.
pub struct StudentOffering {
    pub id: i64,
    pub code: String,
    pub title: String,
    pub credits: i32,
    pub department: String,
    pub course_type: String,
    pub selection_mode: String,
    pub choice_group: String,
    pub state: String,
}

/// Offerings the student may choose from, each with their current selection
/// state (empty when not selected yet).
pub async fn student_eligible_offerings(
    db: &PgPool,
    student_id: i64,
) -> Res<Vec<StudentOffering>> {
    // A student always keeps sight of anything they already hold a selection
    // for, even if the offering's audience is edited afterwards.
    let rows = sqlx::query_as::<_, (i64, String, String, i32, String, String, String, String, Option<String>)>(
        &format!(
            r#"SELECT o.id, c.code, c.title, c.credits, d.name AS department,
                      o.course_type, o.selection_mode, o.choice_group,
                      (SELECT sel.state FROM student_course_selections sel
                        WHERE sel.student_id = $1 AND sel.offering_id = o.id)
               FROM course_offerings o
               JOIN courses c ON c.id = o.course_id
               JOIN departments d ON d.id = o.offering_department_id
               JOIN students st ON st.id = $1 AND st.is_active
               JOIN academic_years y ON y.id = o.academic_year_id
               WHERE {ELIGIBLE}
                  OR EXISTS (SELECT 1 FROM student_course_selections sel
                              WHERE sel.student_id = st.id AND sel.offering_id = o.id)
               ORDER BY d.name, c.code"#,
        ),
    )
    .bind(student_id)
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(id, code, title, credits, department, course_type, selection_mode, choice_group, state)| {
            StudentOffering {
                id,
                code,
                title,
                credits,
                department,
                course_type,
                selection_mode,
                choice_group,
                state: state.unwrap_or_default(),
            }
        })
        .collect())
}

// ---------- Selections ----------

/// Create or update a draft selection. Guarded server-side by `eligible`.
pub async fn select_course(
    db: &PgPool,
    student_id: i64,
    offering_id: i64,
) -> Result<(), AppError> {
    if !eligible(db, student_id, offering_id).await? {
        return Err(AppError::Forbidden);
    }
    // Only INDIVIDUAL_CHOICE is picked by the student; FIXED applies itself,
    // HOD_ASSIGNED is placed by the department, COHORT_CHOICE comes from the
    // cohort decision.
    let mode: Option<String> = sqlx::query_scalar(
        "SELECT selection_mode FROM course_offerings WHERE id = $1 AND status = 'published'",
    )
    .bind(offering_id)
    .fetch_optional(db)
    .await?;
    if mode.as_deref() != Some("INDIVIDUAL_CHOICE") {
        return Err(AppError::BadRequest(
            "That course is not yours to pick: it is fixed, assigned by your department, or decided for your batch.".into(),
        ));
    }
    sqlx::query(
        r#"INSERT INTO student_course_selections (student_id, offering_id, state)
           VALUES ($1, $2, 'draft')
           ON CONFLICT (student_id, offering_id)
           DO UPDATE SET state = 'draft', updated_at = now()
             WHERE student_course_selections.state IN ('draft', 'change_requested')"#,
    )
    .bind(student_id)
    .bind(offering_id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn withdraw_selection(db: &PgPool, student_id: i64, offering_id: i64) -> Res<()> {
    sqlx::query(
        r#"DELETE FROM student_course_selections
            WHERE student_id = $1 AND offering_id = $2
              AND state IN ('draft', 'submitted')"#,
    )
    .bind(student_id)
    .bind(offering_id)
    .execute(db)
    .await?;
    Ok(())
}

/// The offering must be published, in the expected selection mode, and still
/// have a free seat if it sets a capacity. Runs inside the caller's
/// transaction and returns the course to enroll in.
///
/// Seats taken = this offering's own active enrollments, read from
/// `enrollments.offering_id`. Students of the same course under another
/// offering (or an earlier year's run of it) never hold this offering's seats.
async fn offering_for_confirm(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    offering_id: i64,
    expected_mode: &str,
) -> Result<i64, AppError> {
    let row: (i64, String, Option<i32>, i64) = sqlx::query_as(
        r#"SELECT o.course_id, o.selection_mode, o.capacity,
                  (SELECT count(*) FROM enrollments e
                    JOIN students est ON est.id = e.student_id AND est.is_active
                   WHERE e.offering_id = o.id AND e.status = 'active')
              FROM course_offerings o
             WHERE o.id = $1 AND o.status = 'published'
             FOR UPDATE"#,
    )
    .bind(offering_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(AppError::NotFound)?;

    let (course_id, mode, capacity, taken) = row;
    if mode != expected_mode {
        return Err(AppError::BadRequest(
            if expected_mode == "INDIVIDUAL_CHOICE" {
                "You cannot confirm this course yourself: it is applied automatically or assigned by your department."
            } else {
                "That course is not assigned by the department."
            }
            .into(),
        ));
    }
    if let Some(cap) = capacity {
        if taken >= i64::from(cap) {
            return Err(AppError::BadRequest(
                "This course is full; choose another while seats remain.".into(),
            ));
        }
    }
    Ok(course_id)
}

/// The enrollment the rest of the college already runs on: timetable and
/// attendance hang off it, whichever way the selection was decided.
///
/// `offering_id` records which offering produced it, so seat counts, capacity
/// checks and an offering period's roster all read this row instead of
/// guessing from the course. Programmed bulk enrollment (no offering) leaves
/// it NULL.
async fn enroll(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    student_id: i64,
    course_id: i64,
    offering_id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        r#"INSERT INTO enrollments (student_id, course_id, semester, offering_id)
           VALUES ($1, $2, (SELECT semester FROM students WHERE id = $1), $3)
           ON CONFLICT (student_id, course_id)
           DO UPDATE SET status = 'active', semester = EXCLUDED.semester,
                         offering_id = EXCLUDED.offering_id"#,
    )
    .bind(student_id)
    .bind(course_id)
    .bind(offering_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Confirm one selection: the student's final choice. Creates the enrollment
/// that timetable and attendance hang off, inside one transaction.
pub async fn confirm_selection(db: &PgPool, student_id: i64, offering_id: i64) -> Result<(), AppError> {
    if !eligible(db, student_id, offering_id).await? {
        return Err(AppError::Forbidden);
    }
    let mut tx = db.begin().await?;
    let course_id = offering_for_confirm(&mut tx, offering_id, "INDIVIDUAL_CHOICE").await?;

    // Mark confirmed, unless it is already confirmed or locked.
    let updated = sqlx::query(
        r#"INSERT INTO student_course_selections (student_id, offering_id, state, confirmed_at)
           VALUES ($1, $2, 'confirmed', now())
           ON CONFLICT (student_id, offering_id)
           DO UPDATE SET state = 'confirmed', confirmed_at = now()
             WHERE student_course_selections.state IN ('draft', 'submitted', 'change_requested')"#,
    )
    .bind(student_id)
    .bind(offering_id)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(AppError::BadRequest(
            "That selection is already confirmed or locked; ask your HOD for a change.".into(),
        ));
    }

    enroll(&mut tx, student_id, course_id, offering_id).await?;
    tx.commit().await?;
    Ok(())
}

/// All of a student's selections, for the hub.
#[derive(Debug, FromRow)]
pub struct SelectionRow {
    pub id: i64,
    pub offering_id: i64,
    pub code: String,
    pub title: String,
    pub credits: i32,
    pub department: String,
    pub course_type: String,
    pub selection_mode: String,
    pub choice_group: String,
    pub state: String,
}

pub async fn student_selections(db: &PgPool, student_id: i64) -> Res<Vec<SelectionRow>> {
    sqlx::query_as::<_, SelectionRow>(
        r#"SELECT s.id, s.offering_id, c.code, c.title, c.credits,
                  d.name AS department, o.course_type, o.selection_mode,
                  o.choice_group, s.state
           FROM student_course_selections s
           JOIN course_offerings o ON o.id = s.offering_id
           JOIN courses c ON c.id = o.course_id
           JOIN departments d ON d.id = o.offering_department_id
           WHERE s.student_id = $1
           ORDER BY s.state, c.code"#,
    )
    .bind(student_id)
    .fetch_all(db)
    .await
}

/// The HOD's view: everyone in the department who has selected something.
#[derive(Debug, FromRow)]
pub struct DeptSelection {
    pub selection_id: i64,
    pub student_id: i64,
    pub name: String,
    pub admission_no: String,
    pub programme: String,
    pub semester: i32,
    pub code: String,
    pub title: String,
    pub department: String,
    pub selection_mode: String,
    pub state: String,
}

pub async fn department_selections(db: &PgPool, department: Option<i64>) -> Res<Vec<DeptSelection>> {
    sqlx::query_as::<_, DeptSelection>(
        r#"SELECT s.id AS selection_id, s.student_id, st.name, st.admission_no,
                  p.name AS programme, st.semester, c.code, c.title,
                  od.name AS department, o.selection_mode, s.state
           FROM student_course_selections s
           JOIN students st ON st.id = s.student_id
           JOIN programmes p ON p.id = st.programme_id
           JOIN course_offerings o ON o.id = s.offering_id
           JOIN courses c ON c.id = o.course_id
           JOIN departments od ON od.id = o.offering_department_id
           WHERE ($1::bigint IS NULL OR st.department_id = $1
                  OR p.department_id = $1)
           ORDER BY st.name, c.code"#,
    )
    .bind(department)
    .fetch_all(db)
    .await
}

// ---------- Change requests ----------

pub struct NewChangeRequest {
    pub student_id: i64,
    pub offering_id: i64,
    pub new_offering_id: i64,
    pub reason: String,
}

/// The student asks to move from one confirmed offering to another.
pub async fn create_change_request(
    db: &PgPool,
    r: &NewChangeRequest,
) -> Result<(), AppError> {
    // The current one must really be the student's confirmed selection, and the
    // target must be one they would have been allowed to take.
    let confirmed: Option<i64> = sqlx::query_scalar(
        r#"SELECT offering_id FROM student_course_selections
            WHERE student_id = $1 AND offering_id = $2
              AND state IN ('confirmed', 'locked', 'change_requested')"#,
    )
    .bind(r.student_id)
    .bind(r.offering_id)
    .fetch_optional(db)
    .await?;
    if confirmed.is_none() {
        return Err(AppError::BadRequest("That course is not your confirmed selection.".into()));
    }
    if !eligible(db, r.student_id, r.new_offering_id).await? {
        return Err(AppError::Forbidden);
    }
    // The target must be a course the student chooses themselves; fixed,
    // HOD-assigned and cohort-choice courses are not swapped through a
    // student's request.
    let new_mode: Option<String> = sqlx::query_scalar(
        "SELECT selection_mode FROM course_offerings WHERE id = $1 AND status = 'published'",
    )
    .bind(r.new_offering_id)
    .fetch_optional(db)
    .await?;
    if new_mode.as_deref() != Some("INDIVIDUAL_CHOICE") {
        return Err(AppError::BadRequest(
            "You can only move to another course that you choose yourself.".into(),
        ));
    }
    let mut tx = db.begin().await?;
    sqlx::query(
        r#"INSERT INTO course_change_requests (student_id, offering_id, new_offering_id, reason)
           VALUES ($1, $2, $3, $4)"#,
    )
    .bind(r.student_id)
    .bind(r.offering_id)
    .bind(r.new_offering_id)
    .bind(&r.reason)
    .execute(&mut *tx)
    .await?;
    // The student cannot confirm anything else while a request is pending.
    sqlx::query(
        r#"UPDATE student_course_selections
              SET state = 'change_requested', updated_at = now()
            WHERE student_id = $1 AND offering_id = $2
              AND state IN ('confirmed', 'locked')"#,
    )
    .bind(r.student_id)
    .bind(r.offering_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Pending requests visible to this HOD (their department's students, or the admin).
#[derive(Debug, FromRow)]
pub struct ChangeRequest {
    pub id: i64,
    pub student_id: i64,
    pub name: String,
    pub admission_no: String,
    pub programme: String,
    pub current_code: String,
    pub current_title: String,
    pub new_code: String,
    pub new_title: String,
    pub reason: String,
    pub status: String,
}

pub async fn pending_change_requests(db: &PgPool, department: Option<i64>) -> Res<Vec<ChangeRequest>> {
    sqlx::query_as::<_, ChangeRequest>(
        r#"SELECT r.id, r.student_id, st.name, st.admission_no, p.name AS programme,
                  cc.code AS current_code, cc.title AS current_title,
                  cn.code AS new_code, cn.title AS new_title,
                  r.reason, r.status
           FROM course_change_requests r
           JOIN students st ON st.id = r.student_id
           JOIN programmes p ON p.id = st.programme_id
           JOIN course_offerings co ON co.id = r.offering_id
           JOIN courses cc ON cc.id = co.course_id
           JOIN course_offerings no_ ON no_.id = r.new_offering_id
           JOIN courses cn ON cn.id = no_.course_id
           WHERE r.status = 'pending'
             AND ($1::bigint IS NULL OR st.department_id = $1 OR p.department_id = $1)
           ORDER BY r.created_at"#,
    )
    .bind(department)
    .fetch_all(db)
    .await
}

/// The HOD's decision on a change request. On approval the confirmed selection
/// moves to the new offering and the enrollment follows, so timetable and
/// attendance continue from the same enrollment.
pub async fn decide_change_request(
    db: &PgPool,
    request_id: i64,
    department: Option<i64>,
    approve: bool,
    note: &str,
    decided_by: i64,
) -> Result<bool, AppError> {
    let Some(r) = sqlx::query_as::<_, (i64, i64)>(
        r#"SELECT r.student_id, r.offering_id
             FROM course_change_requests r
             JOIN students st ON st.id = r.student_id
             JOIN programmes p ON p.id = st.programme_id
            WHERE r.id = $1 AND r.status = 'pending'
              AND ($2::bigint IS NULL OR st.department_id = $2 OR p.department_id = $2)"#,
    )
    .bind(request_id)
    .bind(department)
    .fetch_optional(db)
    .await?
    else {
        return Ok(false);
    };
    let (student_id, old_offering_id) = r;

    let mut tx = db.begin().await?;
    sqlx::query(
        r#"UPDATE course_change_requests
              SET status = $2, decision_note = $3, decided_by = $4, decided_at = now()
            WHERE id = $1 AND status = 'pending'"#,
    )
    .bind(request_id)
    .bind(if approve { "approved" } else { "rejected" })
    .bind(note)
    .bind(decided_by)
    .execute(&mut *tx)
    .await?;

    if approve {
        let new_offering_id: i64 =
            sqlx::query_scalar("SELECT new_offering_id FROM course_change_requests WHERE id = $1")
                .bind(request_id)
                .fetch_one(&mut *tx)
                .await?;

        // Move the selection to the new offering.
        sqlx::query(
            r#"UPDATE student_course_selections
                  SET offering_id = $3, state = 'confirmed', confirmed_at = now(),
                      decided_by = $4, decided_note = $5, updated_at = now()
                WHERE student_id = $1 AND offering_id = $2"#,
        )
        .bind(student_id)
        .bind(old_offering_id)
        .bind(new_offering_id)
        .bind(decided_by)
        .bind(note)
        .execute(&mut *tx)
        .await?;

        // Swap the enrollment: the new one in, the old one dropped.
        let old_course: i64 =
            sqlx::query_scalar("SELECT course_id FROM course_offerings WHERE id = $1")
                .bind(old_offering_id)
                .fetch_one(&mut *tx)
                .await?;
        let new_course: i64 =
            sqlx::query_scalar("SELECT course_id FROM course_offerings WHERE id = $1")
                .bind(new_offering_id)
                .fetch_one(&mut *tx)
                .await?;
        sqlx::query(
            r#"INSERT INTO enrollments (student_id, course_id, semester, offering_id)
               VALUES ($1, $2, (SELECT semester FROM students WHERE id = $1), $3)
               ON CONFLICT (student_id, course_id)
               DO UPDATE SET status = 'active', semester = EXCLUDED.semester,
                             offering_id = EXCLUDED.offering_id"#,
        )
        .bind(student_id)
        .bind(new_course)
        .bind(new_offering_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"UPDATE enrollments SET status = 'dropped'
                WHERE student_id = $1 AND course_id = $2 AND $3::bigint <> $2"#,
        )
        .bind(student_id)
        .bind(old_course)
        .bind(new_course)
        .execute(&mut *tx)
        .await?;
    } else {
        // Rejected: put the selection back the way it was.
        sqlx::query(
            r#"UPDATE student_course_selections
                  SET state = 'confirmed', updated_at = now()
                WHERE student_id = $1 AND offering_id = $2
                  AND state = 'change_requested'"#,
        )
        .bind(student_id)
        .bind(old_offering_id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(true)
}

// ---------- Cohort specializations ----------

/// The HOD finalises one offering per choice group for a cohort (the BCA
/// AI/ML-vs-Full-Stack decision, or any programme's later equivalent). Every
/// active student of the cohort is confirmed onto the finalised offering and
/// enrolled — nobody picks individually. A re-finalisation replaces the old
/// decision: selections and enrollments move off the group's other offerings
/// and pending change requests on them are cancelled. Returns how many
/// students were updated.
pub async fn finalize_cohort_specialization(
    db: &PgPool,
    cohort_id: i64,
    choice_group: &str,
    offering_id: i64,
    note: &str,
    decided_by: i64,
) -> Result<u64, AppError> {
    // The offering must be a published COHORT_CHOICE in this group, and it
    // must actually target this cohort.
    let group: Option<String> = sqlx::query_scalar(
        r#"SELECT choice_group FROM course_offerings o
            WHERE o.id = $1 AND o.status = 'published' AND o.selection_mode = 'COHORT_CHOICE'
              AND EXISTS (SELECT 1 FROM course_offering_targets t
                           WHERE t.offering_id = o.id AND t.cohort_id = $2)"#,
    )
    .bind(offering_id)
    .bind(cohort_id)
    .fetch_optional(db)
    .await?;
    let Some(_g) = group.filter(|g| g == choice_group) else {
        return Err(AppError::BadRequest(
            "Pick a published cohort-choice offering in that choice group and cohort."
                .into(),
        ));
    };

    let mut tx = db.begin().await?;
    // Record the decision; a re-finalisation overwrites it in place.
    sqlx::query(
        r#"INSERT INTO cohort_specializations (cohort_id, choice_group, offering_id, note, decided_by, decided_at)
           VALUES ($1, $2, $3, $4, $5, now())
           ON CONFLICT (cohort_id, choice_group)
           DO UPDATE SET offering_id = EXCLUDED.offering_id, note = EXCLUDED.note,
                         decided_by = EXCLUDED.decided_by, decided_at = now()"#,
    )
    .bind(cohort_id)
    .bind(choice_group)
    .bind(offering_id)
    .bind(note)
    .bind(decided_by)
    .execute(&mut *tx)
    .await?;

    let course_id: i64 =
        sqlx::query_scalar("SELECT course_id FROM course_offerings WHERE id = $1")
            .bind(offering_id)
            .fetch_one(&mut *tx)
            .await?;

    // Every active student of the cohort inherits the decision.
    let students: Vec<i64> = sqlx::query_scalar(
        r#"SELECT st.id FROM students st
             JOIN cohorts cb ON cb.programme_id = st.programme_id
                            AND cb.batch_year = st.batch_year
            WHERE cb.id = $1 AND st.is_active"#,
    )
    .bind(cohort_id)
    .fetch_all(&mut *tx)
    .await?;
    if students.is_empty() {
        tx.commit().await?;
        return Ok(0);
    }

    // The group's other offerings in the same academic year: what the cohort
    // had instead, or an earlier decision being replaced.
    let others: Vec<i64> = sqlx::query_scalar(
        r#"SELECT id FROM course_offerings
            WHERE selection_mode = 'COHORT_CHOICE' AND choice_group = $1
              AND id <> $2
              AND academic_year_id = (SELECT academic_year_id FROM course_offerings WHERE id = $2)"#,
    )
    .bind(choice_group)
    .bind(offering_id)
    .fetch_all(&mut *tx)
    .await?;

    if !others.is_empty() {
        let group_offerings: Vec<i64> = others.iter().copied().chain([offering_id]).collect();
        // The cohort decision itself settles any change request on the group.
        sqlx::query(
            r#"DELETE FROM course_change_requests
                WHERE student_id = ANY($1) AND status = 'pending'
                  AND (offering_id = ANY($2) OR new_offering_id = ANY($2))"#,
        )
        .bind(&students)
        .bind(&group_offerings)
        .execute(&mut *tx)
        .await?;
        // Their selections on the group's other offerings go...
        sqlx::query(
            "DELETE FROM student_course_selections WHERE student_id = ANY($1) AND offering_id = ANY($2)",
        )
        .bind(&students)
        .bind(&others)
        .execute(&mut *tx)
        .await?;
        // ...and so do the enrollments of those courses.
        let other_courses: Vec<i64> =
            sqlx::query_scalar("SELECT DISTINCT course_id FROM course_offerings WHERE id = ANY($1)")
                .bind(&others)
                .fetch_all(&mut *tx)
                .await?;
        sqlx::query(
            r#"UPDATE enrollments SET status = 'dropped'
                WHERE student_id = ANY($1) AND course_id = ANY($2) AND status = 'active'"#,
        )
        .bind(&students)
        .bind(&other_courses)
        .execute(&mut *tx)
        .await?;
    }

    // Confirm the whole cohort onto the finalised offering...
    sqlx::query(
        r#"INSERT INTO student_course_selections
               (student_id, offering_id, state, confirmed_at, decided_by, decided_note)
           SELECT u.sid, $2, 'confirmed', now(), $3, 'Cohort finalisation'
             FROM UNNEST($1::bigint[]) AS u(sid)
           ON CONFLICT (student_id, offering_id)
           DO UPDATE SET state = 'confirmed', confirmed_at = now(),
                         decided_by = EXCLUDED.decided_by,
                         decided_note = EXCLUDED.decided_note,
                         updated_at = now()"#,
    )
    .bind(&students)
    .bind(offering_id)
    .bind(decided_by)
    .execute(&mut *tx)
    .await?;
    // ...and enroll them, which is what timetable and attendance follow.
    sqlx::query(
        r#"INSERT INTO enrollments (student_id, course_id, semester, offering_id)
           SELECT u.sid, $2, (SELECT st.semester FROM students st WHERE st.id = u.sid), $3
             FROM UNNEST($1::bigint[]) AS u(sid)
           ON CONFLICT (student_id, course_id)
           DO UPDATE SET status = 'active', semester = EXCLUDED.semester,
                         offering_id = EXCLUDED.offering_id"#,
    )
    .bind(&students)
    .bind(course_id)
    .bind(offering_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(students.len() as u64)
}

/// Cohort decisions this HOD can see (their programme's cohorts, or the admin).
#[derive(Debug, FromRow)]
pub struct CohortDecision {
    #[allow(dead_code)]
    pub id: i64,
    pub cohort_label: String,
    #[allow(dead_code)]
    pub programme: String,
    #[allow(dead_code)]
    pub batch_year: i32,
    pub choice_group: String,
    pub code: String,
    pub title: String,
    pub note: String,
}

pub async fn cohort_decisions(db: &PgPool, department: Option<i64>) -> Res<Vec<CohortDecision>> {
    sqlx::query_as::<_, CohortDecision>(
        r#"SELECT cs.id,
                  p.name || ' ' || cb.batch_year AS cohort_label,
                  p.name AS programme, cb.batch_year,
                  cs.choice_group, c.code, c.title, cs.note
           FROM cohort_specializations cs
           JOIN cohorts cb ON cb.id = cs.cohort_id
           JOIN programmes p ON p.id = cb.programme_id
           JOIN course_offerings o ON o.id = cs.offering_id
           JOIN courses c ON c.id = o.course_id
           WHERE ($1::bigint IS NULL OR p.department_id = $1)
           ORDER BY p.name, cb.batch_year, cs.choice_group"#,
    )
    .bind(department)
    .fetch_all(db)
    .await
}

/// The choice groups that still need a decision, with their alternatives.
#[derive(Debug)]
pub struct CohortChoicePending {
    pub cohort_id: i64,
    pub cohort_label: String,
    pub choice_group: String,
    pub options: Vec<(i64, String, String)>, // offering_id, code, title
}

pub async fn pending_cohort_choices(db: &PgPool, department: Option<i64>) -> Res<Vec<CohortChoicePending>> {
    // Cohorts are created on demand; a choice group with published
    // COHORT_CHOICE offerings targeting a programme implies the cohort exists
    // (targets reference cohorts directly). So we look at cohorts with a
    // targeted group that has no finalisation yet.
    let rows = sqlx::query_as::<_, (i64, String, String)>(
        r#"SELECT DISTINCT cb.id, p.name || ' ' || cb.batch_year, o.choice_group
             FROM cohorts cb
             JOIN programmes p ON p.id = cb.programme_id
             JOIN course_offering_targets t ON t.cohort_id = cb.id
             JOIN course_offerings o ON o.id = t.offering_id
            WHERE o.status = 'published' AND o.selection_mode = 'COHORT_CHOICE'
              AND ($1::bigint IS NULL OR p.department_id = $1)
               AND NOT EXISTS (
                   SELECT 1 FROM cohort_specializations cs
                    WHERE cs.cohort_id = cb.id AND cs.choice_group = o.choice_group)
            ORDER BY 2, 3"#,
    )
    .bind(department)
    .fetch_all(db)
    .await?;

    let mut out = Vec::new();
    for (cohort_id, cohort_label, choice_group) in rows {
        let options = sqlx::query_as::<_, (i64, String, String)>(
            r#"SELECT o.id, c.code, c.title
                 FROM course_offerings o
                 JOIN courses c ON c.id = o.course_id
                WHERE o.status = 'published' AND o.selection_mode = 'COHORT_CHOICE'
                  AND o.choice_group = $2
                  AND EXISTS (SELECT 1 FROM course_offering_targets t
                               WHERE t.offering_id = o.id AND t.cohort_id = $1)
                ORDER BY c.code"#,
        )
        .bind(cohort_id)
        .bind(&choice_group)
        .fetch_all(db)
        .await?;
        out.push(CohortChoicePending {
            cohort_id,
            cohort_label,
            choice_group,
            options,
        });
    }
    Ok(out)
}

// ---------- Applying and assigning without the student choosing ----------

/// FIXED offerings apply themselves: every student the offering targets is
/// confirmed and enrolled with no action from them, which is what "fixed"
/// means for the student. Idempotent, and called whenever an offering's
/// audience can change (targets set, publish, approval decided) plus lazily
/// for one student when they open their course page — so a student imported
/// after the offering was published still gets the course.
///
/// `offering_id` limits it to one offering, `student_id` to one student; both
/// `None` applies everything (not used that way today).
pub async fn auto_apply_fixed(
    db: &PgPool,
    offering_id: Option<i64>,
    student_id: Option<i64>,
) -> Result<u64, AppError> {
    let applied = sqlx::query(&format!(
        r#"INSERT INTO student_course_selections
               (student_id, offering_id, state, confirmed_at, decided_note)
           SELECT st.id, o.id, 'confirmed', now(), 'Fixed course'
             FROM course_offerings o
             JOIN academic_years y ON y.id = o.academic_year_id
             JOIN students st ON st.is_active
            WHERE o.selection_mode = 'FIXED'
              AND ($1::bigint IS NULL OR o.id = $1)
              AND ($2::bigint IS NULL OR st.id = $2)
              AND {ELIGIBLE}
            ON CONFLICT (student_id, offering_id) DO NOTHING"#,
    ))
    .bind(offering_id)
    .bind(student_id)
    .execute(db)
    .await?
    .rows_affected();

    {
        // Bring the enrollments in step: one statement covers both the rows
        // just created and anything applied earlier.
        sqlx::query(
            r#"INSERT INTO enrollments (student_id, course_id, semester, offering_id)
               SELECT sc.student_id, o.course_id, st.semester, sc.offering_id
                 FROM student_course_selections sc
                 JOIN course_offerings o ON o.id = sc.offering_id
                 JOIN students st ON st.id = sc.student_id
                WHERE sc.state = 'confirmed'
                  AND ($1::bigint IS NULL OR sc.offering_id = $1)
                  AND ($2::bigint IS NULL OR sc.student_id = $2)
               ON CONFLICT (student_id, course_id)
               DO UPDATE SET status = 'active', semester = EXCLUDED.semester,
                             offering_id = EXCLUDED.offering_id"#,
        )
        .bind(offering_id)
        .bind(student_id)
        .execute(db)
        .await?;
    }
    Ok(applied)
}

/// Undo FIXED applications that no longer hold: targets edited, the offering
/// unpublished, or a receiving department withdrawing its approval. Only rows
/// this module created (`decided_note = 'Fixed course'`) are touched, and an
/// enrollment is only dropped when the student has no other confirmed
/// selection of that course. Decisions a student or HOD made are never
/// retracted here.
pub async fn retract_fixed(db: &PgPool, offering_id: i64) -> Result<u64, AppError> {
    let removed: Vec<i64> = sqlx::query_scalar(&format!(
        r#"DELETE FROM student_course_selections sc
            WHERE sc.offering_id = $1
              AND sc.state = 'confirmed'
              AND sc.decided_note = 'Fixed course'
              AND NOT EXISTS (
                  SELECT 1
                    FROM course_offerings o
                    JOIN academic_years y ON y.id = o.academic_year_id
                    JOIN students st ON st.id = sc.student_id AND st.is_active
                   WHERE o.id = sc.offering_id
                     AND {ELIGIBLE})
            RETURNING sc.student_id"#,
    ))
    .bind(offering_id)
    .fetch_all(db)
    .await?;
    if removed.is_empty() {
        return Ok(0);
    }

    let course_id: i64 =
        sqlx::query_scalar("SELECT course_id FROM course_offerings WHERE id = $1")
            .bind(offering_id)
            .fetch_one(db)
            .await?;
    sqlx::query(
        r#"UPDATE enrollments e SET status = 'dropped'
            WHERE e.course_id = $1 AND e.student_id = ANY($2) AND e.status = 'active'
              AND NOT EXISTS (
                  SELECT 1 FROM student_course_selections sc
                    JOIN course_offerings o ON o.id = sc.offering_id
                   WHERE sc.student_id = e.student_id
                     AND o.course_id = e.course_id
                     AND sc.state IN ('confirmed', 'locked'))"#,
    )
    .bind(course_id)
    .bind(&removed)
    .execute(db)
    .await?;
    Ok(removed.len() as u64)
}

/// The department assigns a HOD_ASSIGNED offering to one of its students: a
/// confirmed selection recording who decided it, plus the enrollment, in one
/// transaction. Eligibility (targets, approval, semester) is re-checked
/// server-side; the caller only decides which of *its* students may be chosen.
pub async fn assign_selection(
    db: &PgPool,
    offering_id: i64,
    student_id: i64,
    decided_by: i64,
) -> Result<(), AppError> {
    if !eligible(db, student_id, offering_id).await? {
        return Err(AppError::Forbidden);
    }
    let mut tx = db.begin().await?;
    let course_id = offering_for_confirm(&mut tx, offering_id, "HOD_ASSIGNED").await?;

    let updated = sqlx::query(
        r#"INSERT INTO student_course_selections
               (student_id, offering_id, state, confirmed_at, decided_by, decided_note)
           VALUES ($1, $2, 'confirmed', now(), $3, 'Assigned by the department')
           ON CONFLICT (student_id, offering_id)
           DO UPDATE SET state = 'confirmed', confirmed_at = now(),
                         decided_by = $3, decided_note = 'Assigned by the department',
                         updated_at = now()
             WHERE student_course_selections.state <> 'locked'"#,
    )
    .bind(student_id)
    .bind(offering_id)
    .bind(decided_by)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(AppError::BadRequest(
            "That student's selection is locked; unlock it before reassigning.".into(),
        ));
    }

    enroll(&mut tx, student_id, course_id, offering_id).await?;
    tx.commit().await?;
    Ok(())
}

/// The department withdraws an assignment: the selection goes, pending change
/// requests touching it are cancelled, and the enrollment is dropped unless
/// the student still holds another confirmed selection of that course.
/// Returns `false` when there was nothing to withdraw (or it is locked).
pub async fn unassign_selection(
    db: &PgPool,
    offering_id: i64,
    student_id: i64,
) -> Result<bool, AppError> {
    let mode: Option<String> =
        sqlx::query_scalar("SELECT selection_mode FROM course_offerings WHERE id = $1")
            .bind(offering_id)
            .fetch_optional(db)
            .await?;
    if mode.as_deref() != Some("HOD_ASSIGNED") {
        return Err(AppError::BadRequest(
            "Only a department-assigned course can be withdrawn here.".into(),
        ));
    }
    let course_id: i64 =
        sqlx::query_scalar("SELECT course_id FROM course_offerings WHERE id = $1")
            .bind(offering_id)
            .fetch_one(db)
            .await?;

    let mut tx = db.begin().await?;
    let removed = sqlx::query(
        r#"DELETE FROM student_course_selections
            WHERE student_id = $1 AND offering_id = $2
              AND state IN ('draft', 'submitted', 'confirmed', 'change_requested')"#,
    )
    .bind(student_id)
    .bind(offering_id)
    .execute(&mut *tx)
    .await?;
    if removed.rows_affected() == 0 {
        return Ok(false);
    }
    sqlx::query(
        r#"DELETE FROM course_change_requests
            WHERE student_id = $1 AND status = 'pending'
              AND (offering_id = $2 OR new_offering_id = $2)"#,
    )
    .bind(student_id)
    .bind(offering_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"UPDATE enrollments e SET status = 'dropped'
            WHERE e.student_id = $1 AND e.course_id = $2 AND e.status = 'active'
              AND NOT EXISTS (
                  SELECT 1 FROM student_course_selections sc
                    JOIN course_offerings o ON o.id = sc.offering_id
                   WHERE sc.student_id = e.student_id
                     AND o.course_id = e.course_id
                     AND sc.state IN ('confirmed', 'locked'))"#,
    )
    .bind(student_id)
    .bind(course_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

// ---------- The student's own views ----------

/// The student's fixed courses: what they are enrolled in, minus the courses
/// the choices section manages (individual, assigned or cohort-decided
/// selections), so no course is ever listed twice. Courses applied by a FIXED
/// offering belong here — enrollment is all a fixed course means to a
/// student.
#[derive(Debug, FromRow)]
pub struct FixedCourse {
    pub code: String,
    pub title: String,
    pub credits: i32,
    pub teacher: String,
}

pub async fn fixed_courses(db: &PgPool, student_id: i64) -> Res<Vec<FixedCourse>> {
    sqlx::query_as::<_, FixedCourse>(
        r#"SELECT c.code, c.title, c.credits, COALESCE(f.name, '') AS teacher
           FROM enrollments e
           JOIN courses c ON c.id = e.course_id
           LEFT JOIN faculty f ON f.id = c.faculty_id
          WHERE e.student_id = $1 AND e.status = 'active'
            AND NOT EXISTS (
                SELECT 1 FROM student_course_selections sc
                  JOIN course_offerings o ON o.id = sc.offering_id
                 WHERE sc.student_id = e.student_id AND o.course_id = e.course_id
                   AND o.selection_mode <> 'FIXED')
          ORDER BY c.semester, c.code"#,
    )
    .bind(student_id)
    .fetch_all(db)
    .await
}

/// The academic structure the student is studying under, for their dashboard.
#[derive(Debug, FromRow)]
pub struct StudentStructure {
    pub admission_no: String,
    pub programme: String,
    pub department: String,
    pub semester: i32,
    pub batch_year: i32,
}

pub async fn student_structure(db: &PgPool, student_id: i64) -> Res<Option<StudentStructure>> {
    sqlx::query_as::<_, StudentStructure>(
        r#"SELECT st.admission_no, p.name AS programme,
                  COALESCE(d.name, '') AS department,
                  st.semester, st.batch_year
             FROM students st
             JOIN programmes p ON p.id = st.programme_id
             LEFT JOIN departments d ON d.id = COALESCE(st.department_id, p.department_id)
            WHERE st.id = $1 AND st.is_active"#,
    )
    .bind(student_id)
    .fetch_optional(db)
    .await
}

// ---------- The department's assignment tools ----------

/// A published HOD_ASSIGNED offering this department may place on its own
/// students (its own, or one another department published and it approved).
#[derive(Debug, FromRow)]
pub struct AssignableOffering {
    pub id: i64,
    pub code: String,
    pub title: String,
    pub year_label: String,
    pub semester: i32,
    pub department: String,
}

pub async fn assignable_offerings(db: &PgPool, department: Option<i64>) -> Res<Vec<AssignableOffering>> {
    sqlx::query_as::<_, AssignableOffering>(
        r#"SELECT o.id, c.code, c.title, y.label AS year_label, o.semester, d.name AS department
           FROM course_offerings o
           JOIN courses c ON c.id = o.course_id
           JOIN academic_years y ON y.id = o.academic_year_id AND y.is_current
           JOIN departments d ON d.id = o.offering_department_id
          WHERE o.status = 'published'
            AND o.selection_mode = 'HOD_ASSIGNED'
            AND ($1::bigint IS NULL
                 OR d.id = $1
                 OR EXISTS (SELECT 1 FROM course_offering_approvals a
                             WHERE a.offering_id = o.id
                               AND a.department_id = $1 AND a.status = 'approved'))
            AND ($1::bigint IS NULL
                 OR EXISTS (SELECT 1 FROM course_offering_targets t
                              JOIN programmes p ON p.id = t.programme_id
                             WHERE t.offering_id = o.id AND p.department_id = $1))
          ORDER BY c.code"#,
    )
    .bind(department)
    .fetch_all(db)
    .await
}

/// The students of this department, for the assignment form. The server
/// re-checks each student against the offering's targets and approval.
#[derive(Debug, FromRow)]
pub struct AssignableStudent {
    pub id: i64,
    pub name: String,
    pub admission_no: String,
    pub programme: String,
    pub semester: i32,
}

pub async fn assignable_students(db: &PgPool, department: Option<i64>) -> Res<Vec<AssignableStudent>> {
    sqlx::query_as::<_, AssignableStudent>(
        r#"SELECT st.id, st.name, st.admission_no, p.name AS programme, st.semester
           FROM students st
           JOIN programmes p ON p.id = st.programme_id
          WHERE st.is_active
            AND ($1::bigint IS NULL OR st.department_id = $1 OR p.department_id = $1)
          ORDER BY p.name, st.semester, st.name"#,
    )
    .bind(department)
    .fetch_all(db)
    .await
}
