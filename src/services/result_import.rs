//! Staged result imports: parse and validate an upload into a batch, show it,
//! then commit it on an explicit confirmation.
//!
//! The old flow validated and filed in the same request, so a mistake was only
//! discovered after the marks were already written. Here the raw text is kept
//! on the batch, the preview is built from the same parse/validate the commit
//! uses, and the commit is a separate, audited step. A row that would overwrite
//! a finalized result is surfaced as a conflict and blocks the commit rather
//! than erroring halfway through.

use sqlx::{FromRow, PgPool};

use super::exams::{self, PushKind, PushRow};

type Res<T> = Result<T, sqlx::Error>;

/// The `exam_kind` stored on a batch row from the push kind and assessment.
fn exam_kind_of(kind: PushKind, assessment: &str) -> &'static str {
    match kind {
        PushKind::Internal => "internal",
        PushKind::University if assessment == "exam" => "university",
        PushKind::University => "coursework",
    }
}

/// An internal exam carries its own name; every other kind is unnamed.
fn exam_name_of(kind: PushKind, row: &PushRow) -> &str {
    if matches!(kind, PushKind::Internal) {
        row.exam_name.as_str()
    } else {
        ""
    }
}

#[derive(Debug, FromRow)]
pub struct BatchRow {
    pub id: i64,
    pub programme_id: Option<i64>,
    pub semester: i32,
    pub exam_kind: String,
    pub assessment: String,
    pub exam_name: String,
    pub source_name: String,
    pub state: String,
    pub payload: String,
}

pub async fn batch(db: &PgPool, id: i64) -> Res<Option<BatchRow>> {
    sqlx::query_as::<_, BatchRow>(
        r#"SELECT id, programme_id, semester, exam_kind, assessment, exam_name,
                  source_name, state, payload
             FROM result_import_batches WHERE id = $1"#,
    )
    .bind(id)
    .fetch_optional(db)
    .await
}

#[derive(Debug, FromRow)]
pub struct PreviewRow {
    pub line_no: i32,
    pub identity_key: String,
    pub course_code: String,
    pub marks_label: String,
    pub state: String,
    pub error_message: String,
}

pub async fn rows(db: &PgPool, batch_id: i64) -> Res<Vec<PreviewRow>> {
    sqlx::query_as::<_, PreviewRow>(
        r#"SELECT line_no, identity_key, course_code,
                  CASE WHEN obtained IS NULL OR maximum IS NULL THEN ''
                       ELSE obtained::text || ' / ' || maximum::text END AS marks_label,
                  state, error_message
             FROM result_import_rows
            WHERE batch_id = $1
            ORDER BY line_no, id"#,
    )
    .bind(batch_id)
    .fetch_all(db)
    .await
}

/// How many rows are in each state.
pub fn counts(rows: &[PreviewRow]) -> (usize, usize, usize, usize) {
    let n = |want: &str| rows.iter().filter(|r| r.state == want).count();
    (n("valid"), n("error"), n("duplicate"), n("conflict"))
}

/// Whether an existing marks row for this candidate is finalized (or already
/// superseded), which a routine import must never overwrite.
async fn has_finalized(
    db: &PgPool,
    student_id: i64,
    course_id: i64,
    assessment: &str,
    exam_kind: &str,
    exam_name: &str,
) -> Res<bool> {
    sqlx::query_scalar(
        r#"SELECT EXISTS (
               SELECT 1 FROM marks
                WHERE student_id = $1 AND course_id = $2 AND assessment = $3
                  AND exam_kind = $4 AND exam_name = $5
                  AND workflow_state IN ('finalized', 'superseded'))"#,
    )
    .bind(student_id)
    .bind(course_id)
    .bind(assessment)
    .bind(exam_kind)
    .bind(exam_name)
    .fetch_one(db)
    .await
}

/// The `(assessment, exam_kind, exam_name)` a row files under. In guided mode
/// these are fixed by the sheet; otherwise they come from the row and the push
/// kind.
fn row_key(
    guided: bool,
    kind: PushKind,
    batch_assessment: &str,
    batch_exam_kind: &str,
    batch_exam_name: &str,
    row: &PushRow,
) -> (String, String, String) {
    if guided {
        (
            batch_assessment.to_string(),
            batch_exam_kind.to_string(),
            batch_exam_name.to_string(),
        )
    } else {
        (
            row.assessment.clone(),
            exam_kind_of(kind, &row.assessment).to_string(),
            exam_name_of(kind, row).to_string(),
        )
    }
}

/// Parse and validate `text` into a new batch, recording one row per parsed
/// line and per problem. Returns the batch id.
/// `exam_kind` is the value stored on the batch and decides how rows are parsed.
/// When `assessment` is non-empty the import is *guided*: every row files under
/// those fixed values (the marks-grid import) instead of the row's own.
#[allow(clippy::too_many_arguments)]
pub async fn stage(
    db: &PgPool,
    created_by: i64,
    department: Option<i64>,
    programme_id: i64,
    semester: i32,
    exam_kind: &str,
    assessment: &str,
    exam_name: &str,
    source_name: &str,
    text: &str,
) -> Res<i64> {
    let kind = if exam_kind == "internal" { PushKind::Internal } else { PushKind::University };
    let guided = !assessment.is_empty();
    let mut tx = db.begin().await?;
    let batch_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO result_import_batches
               (created_by, programme_id, semester, exam_kind, assessment, exam_name,
                source_name, payload, state)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'preview')
           RETURNING id"#,
    )
    .bind(created_by)
    .bind(programme_id)
    .bind(semester)
    .bind(exam_kind)
    .bind(assessment)
    .bind(exam_name)
    .bind(source_name)
    .bind(text)
    .fetch_one(&mut *tx)
    .await?;

    let (parsed, parse_problems) = exams::parse_push(text, kind);
    let (good, validate_problems) = exams::validate_push(db, department, kind, &parsed).await?;

    // Rows that are good but repeat a (key, course) already seen are duplicates.
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for (row, cand) in &good {
        let key = (row.key.clone(), row.course.to_uppercase());
        let (a, ek, en) = row_key(guided, kind, assessment, exam_kind, exam_name, row);
        let (state, note) = if !seen.insert(key) {
            ("duplicate", "duplicate row in this batch")
        } else if has_finalized(db, cand.student_id, cand.course_id, &a, &ek, &en).await? {
            ("conflict", "an existing finalized result — use the correction workflow")
        } else {
            ("valid", "")
        };
        sqlx::query(
            r#"INSERT INTO result_import_rows
                   (batch_id, line_no, identity_key, course_code, course_id, student_id,
                    obtained, maximum, state, error_message)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
        )
        .bind(batch_id)
        .bind(if row.line > 0 { row.line as i32 } else { 0 })
        .bind(&row.key)
        .bind(&row.course)
        .bind(cand.course_id)
        .bind(cand.student_id)
        .bind(row.obtained)
        .bind(row.max)
        .bind(state)
        .bind(note)
        .execute(&mut *tx)
        .await?;
    }

    for p in parse_problems.iter().chain(validate_problems.iter()) {
        sqlx::query(
            r#"INSERT INTO result_import_rows
                   (batch_id, line_no, identity_key, course_code, state, error_message)
               VALUES ($1, $2, '', '', 'error', $3)"#,
        )
        .bind(batch_id)
        .bind(p.line as i32)
        .bind(&p.problem)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(batch_id)
}

/// What a commit did.
pub struct Committed {
    pub filed: u64,
    pub batch_id: i64,
}

/// Commit a staged batch. Re-parses and re-validates the stored text, refuses
/// if any row would overwrite a finalized result, and files the rest in one
/// transaction.
pub async fn commit(
    db: &PgPool,
    batch_id: i64,
    department: Option<i64>,
) -> Res<Committed> {
    let batch = batch(db, batch_id).await?.ok_or(sqlx::Error::RowNotFound)?;
    if batch.state != "preview" {
        return Err(sqlx::Error::Protocol(
            "this import has already been committed or discarded".into(),
        ));
    }
    let kind = if batch.exam_kind == "internal" {
        PushKind::Internal
    } else {
        PushKind::University
    };
    let guided = !batch.assessment.is_empty();

    let (parsed, _) = exams::parse_push(&batch.payload, kind);
    let (good, _) = exams::validate_push(db, department, kind, &parsed).await?;

    // Any conflict blocks the whole commit, so a routine import can never
    // silently overwrite a finalized result.
    for (row, cand) in &good {
        let (a, ek, en) = row_key(
            guided, kind, &batch.assessment, &batch.exam_kind, &batch.exam_name, row,
        );
        if has_finalized(db, cand.student_id, cand.course_id, &a, &ek, &en).await? {
            return Err(sqlx::Error::Protocol(
                "one or more rows would overwrite a finalized result".into(),
            ));
        }
    }

    let filed = if guided {
        exams::push_marks_as(
            db,
            batch.semester,
            &batch.assessment,
            &batch.exam_kind,
            &batch.exam_name,
            &good,
        )
        .await?
    } else {
        exams::push_marks(db, kind, batch.semester, &good).await?
    };

    let mut tx = db.begin().await?;
    sqlx::query(
        "UPDATE result_import_rows SET state = 'committed' WHERE batch_id = $1 AND state = 'valid'",
    )
    .bind(batch_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE result_import_batches SET state = 'committed', committed_at = now() WHERE id = $1",
    )
    .bind(batch_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(Committed { filed, batch_id })
}

/// Throw a staged batch away.
pub async fn discard(db: &PgPool, batch_id: i64) -> Res<()> {
    sqlx::query("UPDATE result_import_batches SET state = 'cancelled' WHERE id = $1 AND state = 'preview'")
        .bind(batch_id)
        .execute(db)
        .await?;
    Ok(())
}
