//! The staged result import: upload or paste, review, then commit.
//!
//! The upload never writes a mark. It stages the rows, shows what would happen
//! (per-line problems, duplicates, conflicts with finalized results), and only
//! a second, explicit confirmation files them.

use std::collections::HashMap;

use askama::Template;
use axum::{
    extract::{Multipart, Query, State},
    response::{IntoResponse, Redirect, Response},
    Form,
};
use serde::Deserialize;
use tower_sessions::Session;

use super::{parse_i32, parse_i64};
use crate::{
    auth::{csrf, require_reauth, Manager},
    sections,
    error::{internal, AppError},
    services::{
        exams::PushKind,
        result_import::{self, PreviewCourse, PreviewRow},
        users,
    },
    shell::{self, Shell},
    state::AppState,
};

/// A mark can never exceed `NUMERIC(6,2)`'s ceiling.
const MAX_IMPORT_MARK: f64 = 9999.99;

/// One `name=value` pair from a urlencoded body.
fn field<'a>(pairs: &'a [(String, String)], key: &str) -> &'a str {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .unwrap_or("")
}

/// A path in the workspace this manager is working in: `/admin` for the IT
/// administrator, `/hod` for a head of department, who signs in on the public
/// listener where `/admin` is not served at all.
fn ws(manager: &Manager, path: &str) -> String {
    format!("{}{path}", sections::base_for(&manager.user))
}

#[derive(Deserialize, Default)]
pub struct PasteForm {
    csrf_token: String,
    programme: String,
    semester: String,
    kind: String,
    rows: String,
}

#[derive(Deserialize, Default)]
pub struct BatchQuery {
    batch: Option<String>,
}

#[derive(Deserialize)]
pub struct BatchForm {
    csrf_token: String,
    batch: String,
}

#[derive(Template)]
#[template(path = "admin/results_preview.html")]
pub struct PreviewTemplate {
    shell: Shell,
    batch_id: i64,
    source_name: String,
    semester: i32,
    kind: String,
    valid: usize,
    error: usize,
    duplicate: usize,
    conflict: usize,
    rows: Vec<PreviewRow>,
    /// One maximum box per course, so the head can set what each course is out
    /// of before the rows are filed.
    courses: Vec<PreviewCourse>,
}

fn semester_of(raw: &str) -> i32 {
    parse_i32(raw).filter(|v| (1..=12).contains(v)).unwrap_or(1)
}

/// Stage pasted rows, then send the head to the review page.
pub async fn preview_paste(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Form(f): Form<PasteForm>,
) -> Result<Response, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let programme_id = super::exams::selected_programme(&s, manager.department(), Some(&f.programme)).await?;
    if programme_id == 0 {
        shell::flash(&session, "Choose a programme first.").await?;
        return Ok(Redirect::to(&ws(&manager, "/exams")).into_response());
    }
    if f.rows.trim().is_empty() {
        shell::flash(&session, "Paste at least one result row.").await?;
        return Ok(Redirect::to(&format!("{}/exams?programme={programme_id}", ws(&manager, ""))).into_response());
    }
    let semester = semester_of(&f.semester);
    let kind = PushKind::parse(&f.kind);
    let id = result_import::stage(
        &s.db,
        manager.user.id,
        manager.department(),
        programme_id,
        semester,
        kind.as_str(),
        "",
        "",
        "pasted rows",
        &f.rows,
        &[],
    )
    .await?;
    Ok(Redirect::to(&format!("{}/exams/results/preview?batch={id}", ws(&manager, ""))).into_response())
}

/// Stage an uploaded CSV file.
pub async fn upload(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let form = crate::uploads::read(multipart).await?;
    csrf::verify(&session, form.field("csrf_token")).await?;
    let programme_id = super::exams::selected_programme(&s, manager.department(), Some(form.field("programme"))).await?;
    if programme_id == 0 {
        shell::flash(&session, "Choose a programme first.").await?;
        return Ok(Redirect::to(&ws(&manager, "/exams")).into_response());
    }
    let file = form.file("csv_file").ok_or(AppError::BadRequest("Choose a CSV file to upload.".into()))?;
    if file.bytes.len() > 5 * 1024 * 1024 {
        shell::flash(&session, "That CSV file is larger than 5 MB — split it into smaller batches.").await?;
        return Ok(Redirect::to(&format!("{}/exams?programme={programme_id}", ws(&manager, ""))).into_response());
    }
    // A CSV/TSV file is decoded as before; a workbook is flattened to CSV, so
    // the same staging pipeline accepts `.xlsx`/`.xls`/`.ods`.
    let text = crate::services::import_students::to_csv_text(&file.bytes, &file.filename)
        .map_err(AppError::BadRequest)?;
    let semester = semester_of(form.field("semester"));
    let kind = PushKind::parse(form.field("kind"));
    let source = if file.filename.is_empty() { "upload.csv".to_string() } else { file.filename.clone() };
    let id = result_import::stage(
        &s.db,
        manager.user.id,
        manager.department(),
        programme_id,
        semester,
        kind.as_str(),
        "",
        "",
        &source,
        &text,
        &[],
    )
    .await?;
    Ok(Redirect::to(&format!("{}/exams/results/preview?batch={id}", ws(&manager, ""))).into_response())
}

pub async fn preview_page(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Query(q): Query<BatchQuery>,
) -> Result<PreviewTemplate, AppError> {
    let Some(id) = q.batch.as_deref().and_then(parse_i64) else {
        return Err(AppError::NotFound);
    };
    let batch = result_import::batch(&s.db, id).await?.ok_or(AppError::NotFound)?;
    let rows = result_import::rows(&s.db, id).await?;
    let courses = result_import::courses(&s.db, id).await?;
    let (valid, error, duplicate, conflict) = result_import::counts(&rows);
    Ok(PreviewTemplate {
        shell: Shell::build(&manager.user, &session, &s.db).await?,
        batch_id: batch.id,
        source_name: batch.source_name,
        semester: batch.semester,
        kind: batch.exam_kind,
        valid,
        error,
        duplicate,
        conflict,
        rows,
        courses,
    })
}

/// File the staged rows, each course out of the maximum the review set.
/// Refuses when any row would overwrite a finalized result; the message says so
/// and nothing is written.
pub async fn commit(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    csrf::verify(&session, field(&pairs, "csrf_token")).await?;
    if let Err(redirect) = require_reauth(&session, &ws(&manager, "/marks")).await {
        shell::flash(&session, "Re-enter your password to commit an import.").await?;
        return Ok(redirect.into_response());
    }
    let Some(id) = parse_i64(field(&pairs, "batch")) else {
        return Err(AppError::NotFound);
    };
    let batch = result_import::batch(&s.db, id).await?.ok_or(AppError::NotFound)?;
    let review = format!("{}/exams/results/preview?batch={id}", ws(&manager, ""));

    // One `max_<course_id>` box per course. A box left blank keeps whatever the
    // file carried; anything else must be a usable maximum.
    let mut max_by_course: HashMap<i64, f64> = HashMap::new();
    for (key, raw) in &pairs {
        let Some(course) = key.strip_prefix("max_").and_then(parse_i64) else {
            continue;
        };
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        match raw.parse::<f64>() {
            Ok(m) if m > 0.0 && m <= MAX_IMPORT_MARK => {
                max_by_course.insert(course, m);
            }
            _ => {
                shell::flash(
                    &session,
                    format!(
                        "Each course's maximum must be a number above 0 and at most {MAX_IMPORT_MARK}."
                    ),
                )
                .await?;
                return Ok(Redirect::to(&review).into_response());
            }
        }
    }

    let back = format!(
        "{}/exams?programme={}&semester={}",
        ws(&manager, ""),
        batch.programme_id.unwrap_or(0),
        batch.semester
    );
    match result_import::commit(&s.db, id, manager.department(), &max_by_course).await {
        Ok(done) => {
            users::audit(&s.db, Some(manager.user.id), "results_import_committed", "import_batch", Some(id)).await?;
            shell::flash(&session, format!("Filed {} result row(s).", done.filed)).await?;
            Ok(Redirect::to(&back).into_response())
        }
        Err(sqlx::Error::Protocol(msg)) => {
            shell::flash(&session, msg).await?;
            Ok(Redirect::to(&review).into_response())
        }
        Err(e) => Err(internal(e)),
    }
}

pub async fn discard(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Form(f): Form<BatchForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    if let Some(id) = parse_i64(&f.batch) {
        result_import::discard(&s.db, id).await?;
        users::audit(&s.db, Some(manager.user.id), "result_import_discarded", "batch", Some(id)).await?;
    }
    shell::flash(&session, "Import discarded; nothing was filed.").await?;
    Ok(Redirect::to(&ws(&manager, "/exams")))
}
