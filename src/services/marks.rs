//! Marks entry grid and the management of filed results.
//!
//! The paste/CSV push in `exams` files results keyed by PRN; this module
//! backs the roster-first screen instead — pick a course, see the class, type
//! a number per student. Both paths write the same `marks` rows, so whatever
//! the grid files reads back on the student's Results page exactly like a
//! pushed row (and vice versa: anything pushed shows up in the grid).

use sqlx::{FromRow, PgPool};

use crate::services::exams::assessment_label;

type Res<T> = Result<T, sqlx::Error>;

// ---------- Entry grid ----------

/// One student on the marks sheet, with whatever is already filed for this
/// exact assessment key (coursework row, internal exam or university exam).
#[derive(Debug, FromRow)]
pub struct RosterRow {
    pub student_id: i64,
    pub name: String,
    pub admission_no: String,
    pub prn: String,
    /// Enrolled in this course for the semester on the sheet.
    pub enrolled: bool,
    /// Existing marks for (student, course, assessment, exam_kind, exam_name).
    /// There is at most one such row — the unique key has no semester — so a
    /// single LEFT JOIN is enough and `m.semester` is deliberately not part
    /// of the join: re-saving under another semester moves the row, exactly
    /// as the paste push does.
    pub obtained: Option<f64>,
    pub published: Option<bool>,
}

/// The roster for one sheet: the programme's active students who are either
/// enrolled in this course for this semester, currently sitting in this
/// semester, or already carry a mark under this key. The last two groups
/// keep the sheet usable before enrollments exist and keep a filed row
/// reachable after a promotion.
pub async fn roster(
    db: &PgPool,
    programme_id: i64,
    semester: i32,
    course_id: i64,
    assessment: &str,
    exam_kind: &str,
    exam_name: &str,
) -> Res<Vec<RosterRow>> {
    sqlx::query_as::<_, RosterRow>(
        r#"SELECT st.id                    AS student_id,
                  st.name,
                  COALESCE(st.admission_no, '') AS admission_no,
                  COALESCE(st.prn, '')          AS prn,
                  (e.id IS NOT NULL)            AS enrolled,
                  m.marks_obtained::float8      AS obtained,
                  m.published
             FROM students st
             LEFT JOIN enrollments e
                    ON e.student_id = st.id
                   AND e.course_id = $3
                   AND e.semester = $2
                   AND e.status = 'active'
             LEFT JOIN marks m
                    ON m.student_id = st.id
                   AND m.course_id = $3
                   AND m.assessment = $4
                   AND m.exam_kind = $5
                   AND m.exam_name = $6
            WHERE st.programme_id = $1
              AND st.is_active
              AND (st.semester = $2 OR e.id IS NOT NULL OR m.student_id IS NOT NULL)
            ORDER BY (e.id IS NULL), st.name, st.id"#,
    )
    .bind(programme_id)
    .bind(semester)
    .bind(course_id)
    .bind(assessment)
    .bind(exam_kind)
    .bind(exam_name)
    .fetch_all(db)
    .await
}

/// One cell of the sheet that was typed in.
pub struct Cell {
    pub student_id: i64,
    pub obtained: f64,
}

/// File one sheet: upsert every typed cell under the semester picked on the
/// page, published immediately (the management list can unpublish). Returns
/// how many rows were written. An internal exam's row attaches to the
/// college's published exam for that course and semester when one exists,
/// so its date shows beside the score — the same rule the push follows.
#[allow(clippy::too_many_arguments)]
pub async fn save_sheet(
    db: &PgPool,
    programme_id: i64,
    semester: i32,
    course_id: i64,
    assessment: &str,
    exam_kind: &str,
    exam_name: &str,
    max_marks: f64,
    cells: &[Cell],
) -> Res<u64> {
    if cells.is_empty() {
        return Ok(0);
    }
    let mut tx = db.begin().await?;
    let exam_id: Option<i64> = if exam_kind == "internal" {
        sqlx::query_scalar(
            r#"SELECT id FROM exams
                WHERE course_id = $1 AND programme_id = $2 AND semester = $3
                  AND status = 'published'
                ORDER BY exam_date LIMIT 1"#,
        )
        .bind(course_id)
        .bind(programme_id)
        .bind(semester)
        .fetch_optional(&mut *tx)
        .await?
    } else {
        None
    };

    let mut filed = 0u64;
    for cell in cells {
        sqlx::query(
            r#"INSERT INTO marks (student_id, course_id, exam_id, assessment,
                                  marks_obtained, max_marks, published,
                                  exam_kind, exam_name, semester)
               VALUES ($1, $2, $3, $4, $5, $6, true, $7, $8, $9)
               ON CONFLICT (student_id, course_id, assessment, exam_kind, exam_name)
               DO UPDATE SET marks_obtained = EXCLUDED.marks_obtained,
                             max_marks      = EXCLUDED.max_marks,
                             exam_id        = COALESCE(EXCLUDED.exam_id, marks.exam_id),
                             published      = true,
                             semester       = EXCLUDED.semester,
                             updated_at     = now()"#,
        )
        .bind(cell.student_id)
        .bind(course_id)
        .bind(exam_id)
        .bind(assessment)
        .bind(cell.obtained)
        .bind(max_marks)
        .bind(exam_kind)
        .bind(exam_name)
        .bind(semester)
        .execute(&mut *tx)
        .await?;
        filed += 1;
    }
    tx.commit().await?;
    Ok(filed)
}

// ---------- Management list ----------

/// One filed mark row, ready for the management table.
#[derive(Debug)]
pub struct FiledRow {
    pub id: i64,
    pub student_name: String,
    pub admission_no: String,
    pub course_code: String,
    pub course_title: String,
    /// "University exam", "Internal exam — Internal 1", "Assignment", ...
    pub kind_label: String,
    pub obtained_label: String,
    pub max_label: String,
    pub published: bool,
    pub updated_label: String,
}

fn num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v:.2}")
    }
}

/// Every mark filed for a programme under the given semester, newest schema
/// rows only (`semester IS NULL` is unreachable since migration 0019
/// back-filled it). Capped at `limit`; `filed_more` reports the truncation.
pub async fn filed(
    db: &PgPool,
    programme_id: i64,
    semester: i32,
    limit: i64,
) -> Res<(Vec<FiledRow>, bool)> {
    let rows: Vec<(i64, String, String, String, String, String, String, String, f64, f64, bool, String)> =
        sqlx::query_as(
            r#"SELECT m.id,
                      st.name,
                      COALESCE(st.admission_no, ''),
                      c.code,
                      c.title,
                      m.assessment,
                      m.exam_kind,
                      m.exam_name,
                      m.marks_obtained::float8,
                      m.max_marks::float8,
                      m.published,
                      to_char(m.updated_at, 'Dy DD Mon YYYY HH24:MI')
                 FROM marks m
                 JOIN students st ON st.id = m.student_id
                 JOIN courses  c  ON c.id  = m.course_id
                WHERE st.programme_id = $1
                  AND m.semester = $2
                ORDER BY c.code, st.name, m.id
                LIMIT $3"#,
        )
        .bind(programme_id)
        .bind(semester)
        .bind(limit + 1)
        .fetch_all(db)
        .await?;

    let more = rows.len() as i64 > limit;
    let out = rows
        .into_iter()
        .take(limit as usize)
        .map(
            |(
                id,
                student_name,
                admission_no,
                course_code,
                course_title,
                assessment,
                exam_kind,
                exam_name,
                obtained,
                maximum,
                published,
                updated_label,
            )| FiledRow {
                id,
                student_name,
                admission_no,
                course_code,
                course_title,
                kind_label: assessment_label(&assessment, &exam_kind, &exam_name),
                obtained_label: num(obtained),
                max_label: num(maximum),
                published,
                updated_label,
            },
        )
        .collect();
    Ok((out, more))
}

/// The programme of the student behind a filed mark, for authorising a
/// toggle or delete: `None` when the row is already gone.
pub async fn mark_programme(db: &PgPool, mark_id: i64) -> Res<Option<i64>> {
    sqlx::query_scalar(
        r#"SELECT st.programme_id
             FROM marks m JOIN students st ON st.id = m.student_id
            WHERE m.id = $1"#,
    )
    .bind(mark_id)
    .fetch_optional(db)
    .await
}

/// Flip a row's published flag; `None` when the row no longer exists.
pub async fn toggle_published(db: &PgPool, mark_id: i64) -> Res<Option<bool>> {
    sqlx::query_scalar(
        r#"UPDATE marks SET published = NOT published, updated_at = now()
            WHERE id = $1 RETURNING published"#,
    )
    .bind(mark_id)
    .fetch_optional(db)
    .await
}

/// Remove one filed mark row. `None` when it was already gone.
pub async fn delete_mark(db: &PgPool, mark_id: i64) -> Res<Option<()>> {
    let done = sqlx::query("DELETE FROM marks WHERE id = $1")
        .bind(mark_id)
        .execute(db)
        .await?;
    Ok(if done.rows_affected() > 0 { Some(()) } else { None })
}
