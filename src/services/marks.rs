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
        let state: Option<String> = sqlx::query_scalar(
            r#"SELECT workflow_state FROM marks
                WHERE student_id = $1 AND course_id = $2 AND assessment = $3
                  AND exam_kind = $4 AND exam_name = $5
                FOR UPDATE"#,
        )
        .bind(cell.student_id)
        .bind(course_id)
        .bind(assessment)
        .bind(exam_kind)
        .bind(exam_name)
        .fetch_optional(&mut *tx)
        .await?;
        if matches!(state.as_deref(), Some("finalized" | "superseded")) {
            return Err(sqlx::Error::Protocol(
                "finalized marks require the correction workflow".into(),
            ));
        }
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
    /// `draft`/`reviewed`/`published`/`finalized`/`correction_pending`/
    /// `superseded` — the lifecycle marker, independent of the published flag.
    pub workflow_state: String,
    pub updated_label: String,
}

impl FiledRow {
    /// A locked row: only the correction workflow may change it.
    pub fn is_finalized(&self) -> bool {
        self.workflow_state == "finalized"
    }

    /// A correction was proposed and is waiting for a second manager.
    pub fn is_correction_pending(&self) -> bool {
        self.workflow_state == "correction_pending"
    }
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
    let rows: Vec<(i64, String, String, String, String, String, String, String, f64, f64, bool, String, String)> =
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
                      m.workflow_state,
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
                workflow_state,
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
                workflow_state,
                updated_label,
            },
        )
        .collect();
    Ok((out, more))
}

/// The lifecycle state of a filed mark, for refusing a toggle or delete on a
/// locked row: `None` when the row is already gone.
pub async fn mark_state(db: &PgPool, mark_id: i64) -> Res<Option<String>> {
    sqlx::query_scalar("SELECT workflow_state FROM marks WHERE id = $1")
        .bind(mark_id)
        .fetch_optional(db)
        .await
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

// ---------- Finalization and corrections ----------

/// What a finalize or correction action did, so the route can say so without
/// guessing from error strings.
#[derive(Debug, PartialEq, Eq)]
pub enum CorrectionOutcome {
    /// The finalize/correction wrote what was asked.
    Done(u64),
    /// No such mark row.
    NotFound,
    /// The row is not finalized, so the correction workflow does not apply.
    NotFinalized,
    /// A correction is already awaiting a decision.
    AlreadyPending,
    /// There is no pending correction to decide.
    NotPending,
    /// The approver is the person who requested the correction.
    SameActor,
    /// The proposed mark is outside `0..=max`.
    OutOfRange,
}

/// Lock every published mark for one course in a programme and semester,
/// moving it to `finalized`. Drafts and already-pending rows are left alone;
/// returns how many rows were locked. The caller has already authorised the
/// programme.
pub async fn finalize_course(
    db: &PgPool,
    programme_id: i64,
    semester: i32,
    course_id: i64,
    actor_id: i64,
) -> Res<u64> {
    let done = sqlx::query(
        r#"UPDATE marks m
              SET workflow_state = 'finalized',
                  finalized_at   = now(),
                  finalized_by   = $4,
                  updated_at     = now()
             FROM students st
            WHERE st.id = m.student_id
              AND st.programme_id = $1
              AND m.semester = $2
              AND m.course_id = $3
              AND m.published
              AND m.workflow_state IN ('draft', 'reviewed', 'published')"#,
    )
    .bind(programme_id)
    .bind(semester)
    .bind(course_id)
    .bind(actor_id)
    .execute(db)
    .await?;
    Ok(done.rows_affected())
}

/// Propose a correction to one finalized mark. `max` is optional: when left
/// empty the correction keeps the row's current maximum. The row is locked
/// while the proposed value is written, so two requests cannot race.
pub async fn request_correction(
    db: &PgPool,
    mark_id: i64,
    actor_id: i64,
    obtained: f64,
    max: Option<f64>,
    reason: &str,
) -> Res<CorrectionOutcome> {
    let mut tx = db.begin().await?;
    let row: Option<(String, f64, f64)> = sqlx::query_as(
        r#"SELECT m.workflow_state, m.marks_obtained::float8, m.max_marks::float8
             FROM marks m
            WHERE m.id = $1
            FOR UPDATE OF m"#,
    )
    .bind(mark_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((state, _old_obtained, old_max)) = row else {
        return Ok(CorrectionOutcome::NotFound);
    };
    match state.as_str() {
        "correction_pending" => return Ok(CorrectionOutcome::AlreadyPending),
        "finalized" => {}
        _ => return Ok(CorrectionOutcome::NotFinalized),
    }
    let limit = max.unwrap_or(old_max);
    if !(0.0..=limit).contains(&obtained) {
        return Ok(CorrectionOutcome::OutOfRange);
    }

    sqlx::query(
        r#"UPDATE marks
              SET workflow_state              = 'correction_pending',
                  correction_proposed_obtained = $2,
                  correction_proposed_max      = $3,
                  correction_reason            = $4,
                  correction_requested_by      = $5,
                  correction_requested_at      = now(),
                  correction_decided_by        = NULL,
                  correction_decided_at        = NULL,
                  updated_at                   = now()
            WHERE id = $1"#,
    )
    .bind(mark_id)
    .bind(obtained)
    .bind(max)
    .bind(reason)
    .bind(actor_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(CorrectionOutcome::Done(1))
}

/// Approve or reject the pending correction on one mark.
///
/// Approving snapshots the current (finalized) values into `mark_history` as a
/// `superseded` row, then applies the proposed value to the live row and locks
/// it again. Rejecting only clears the pending fields. A correction must be
/// approved by someone other than the person who requested it.
pub async fn decide_correction(
    db: &PgPool,
    mark_id: i64,
    approver_id: i64,
    approve: bool,
) -> Res<CorrectionOutcome> {
    let mut tx = db.begin().await?;
    #[allow(clippy::type_complexity)]
    let row: Option<(String, Option<i64>, String, i64, i64, String, String, String, Option<i32>, f64, f64, bool)> = sqlx::query_as(
        r#"SELECT m.workflow_state,
                  m.correction_requested_by,
                  m.correction_reason,
                  m.student_id,
                  m.course_id,
                  m.assessment,
                  m.exam_kind,
                  m.exam_name,
                  m.semester,
                  m.marks_obtained::float8,
                  m.max_marks::float8,
                  m.published
             FROM marks m
            WHERE m.id = $1
            FOR UPDATE OF m"#,
    )
    .bind(mark_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((
        state,
        requested_by,
        reason,
        student_id,
        course_id,
        assessment,
        exam_kind,
        exam_name,
        semester,
        old_obtained,
        old_max,
        old_published,
    )) = row
    else {
        return Ok(CorrectionOutcome::NotFound);
    };
    if state != "correction_pending" {
        return Ok(CorrectionOutcome::NotPending);
    }
    if requested_by == Some(approver_id) {
        return Ok(CorrectionOutcome::SameActor);
    }

    if approve {
        // Keep the value being replaced, then overwrite the live row.
        sqlx::query(
            r#"INSERT INTO mark_history
                   (mark_id, student_id, course_id, assessment, exam_kind, exam_name,
                    semester, marks_obtained, max_marks, published, workflow_state,
                    changed_by, change_reason)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'superseded', $11, $12)"#,
        )
        .bind(mark_id)
        .bind(student_id)
        .bind(course_id)
        .bind(&assessment)
        .bind(&exam_kind)
        .bind(&exam_name)
        .bind(semester)
        .bind(old_obtained)
        .bind(old_max)
        .bind(old_published)
        .bind(approver_id)
        .bind(&reason)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"UPDATE marks
                  SET marks_obtained               = correction_proposed_obtained,
                      max_marks                    = COALESCE(correction_proposed_max, max_marks),
                      workflow_state               = 'finalized',
                      finalized_at                 = now(),
                      finalized_by                 = $2,
                      correction_proposed_obtained = NULL,
                      correction_proposed_max      = NULL,
                      correction_requested_by      = NULL,
                      correction_requested_at      = NULL,
                      correction_decided_by        = $2,
                      correction_decided_at        = now(),
                      updated_at                   = now()
                WHERE id = $1"#,
        )
        .bind(mark_id)
        .bind(approver_id)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query(
            r#"UPDATE marks
                  SET workflow_state               = 'finalized',
                      correction_proposed_obtained = NULL,
                      correction_proposed_max      = NULL,
                      correction_reason            = '',
                      correction_requested_by      = NULL,
                      correction_requested_at      = NULL,
                      correction_decided_by        = $2,
                      correction_decided_at        = now(),
                      updated_at                   = now()
                WHERE id = $1"#,
        )
        .bind(mark_id)
        .bind(approver_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(CorrectionOutcome::Done(1))
}

/// One correction waiting for a decision, for the review panel.
#[derive(Debug)]
pub struct PendingCorrection {
    pub id: i64,
    pub student_name: String,
    pub course_code: String,
    pub kind_label: String,
    pub current_label: String,
    pub proposed_label: String,
    pub reason: String,
    pub requester: String,
    pub requested_label: String,
}

/// Every pending correction across a programme's semesters, oldest first.
pub async fn pending_corrections(db: &PgPool, programme_id: i64) -> Res<Vec<PendingCorrection>> {
    #[allow(clippy::type_complexity)]
    let rows: Vec<(i64, String, String, String, String, String, f64, f64, f64, Option<f64>, String, String, String)> =
        sqlx::query_as(
            r#"SELECT m.id,
                      st.name,
                      c.code,
                      m.assessment,
                      m.exam_kind,
                      m.exam_name,
                      m.marks_obtained::float8,
                      m.max_marks::float8,
                      m.correction_proposed_obtained::float8,
                      m.correction_proposed_max::float8,
                      m.correction_reason,
                      COALESCE(u.full_name, 'a manager'),
                      to_char(m.correction_requested_at, 'Dy DD Mon YYYY HH24:MI')
                 FROM marks m
                 JOIN students st ON st.id = m.student_id
                 JOIN courses  c  ON c.id  = m.course_id
                 LEFT JOIN users u ON u.id = m.correction_requested_by
                WHERE st.programme_id = $1
                  AND m.workflow_state = 'correction_pending'
                ORDER BY m.correction_requested_at"#,
        )
        .bind(programme_id)
        .fetch_all(db)
        .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                student_name,
                course_code,
                assessment,
                exam_kind,
                exam_name,
                obtained,
                maximum,
                proposed,
                proposed_max,
                reason,
                requester,
                requested_label,
            )| PendingCorrection {
                id,
                student_name,
                course_code,
                kind_label: assessment_label(&assessment, &exam_kind, &exam_name),
                current_label: format!("{} / {}", num(obtained), num(maximum)),
                proposed_label: format!(
                    "{} / {}",
                    num(proposed),
                    num(proposed_max.unwrap_or(maximum))
                ),
                reason,
                requester,
                requested_label,
            },
        )
        .collect())
}

/// One applied correction, for the history panel: what the value used to be
/// and who changed it, with the reason.
pub struct HistoryRow {
    pub student_name: String,
    pub course_code: String,
    pub kind_label: String,
    pub semester: i32,
    pub value_label: String,
    pub reason: String,
    pub changed_by: String,
    pub changed_label: String,
}

/// The most recent corrections for a programme, newest first.
pub async fn correction_history(
    db: &PgPool,
    programme_id: i64,
    limit: i64,
) -> Res<Vec<HistoryRow>> {
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, String, String, String, Option<i32>, f64, f64, String, String, String)> =
        sqlx::query_as(
            r#"SELECT st.name,
                      c.code,
                      h.assessment,
                      h.exam_kind,
                      h.exam_name,
                      h.semester,
                      h.marks_obtained::float8,
                      h.max_marks::float8,
                      h.change_reason,
                      COALESCE(u.full_name, 'a manager'),
                      to_char(h.changed_at, 'Dy DD Mon YYYY HH24:MI')
                 FROM mark_history h
                 JOIN students st ON st.id = h.student_id
                 JOIN courses  c  ON c.id  = h.course_id
                 LEFT JOIN users u ON u.id = h.changed_by
                WHERE st.programme_id = $1
                ORDER BY h.changed_at DESC, h.id DESC
                LIMIT $2"#,
        )
        .bind(programme_id)
        .bind(limit)
        .fetch_all(db)
        .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                student_name,
                course_code,
                assessment,
                exam_kind,
                exam_name,
                semester,
                obtained,
                maximum,
                reason,
                changed_by,
                changed_label,
            )| HistoryRow {
                student_name,
                course_code,
                kind_label: assessment_label(&assessment, &exam_kind, &exam_name),
                semester: semester.unwrap_or(0),
                value_label: format!("{} / {}", num(obtained), num(maximum)),
                reason,
                changed_by,
                changed_label,
            },
        )
        .collect())
}
