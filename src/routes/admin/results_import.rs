//! The staged result import: upload or paste, review, then commit.
//!
//! The upload never writes a mark. It stages the rows, shows what would happen
//! (per-line problems, duplicates, conflicts with finalized results), and only
//! a second, explicit confirmation files them.

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
    error::{internal, AppError},
    services::{
        exams::PushKind,
        result_import::{self, PreviewRow},
        users,
    },
    shell::{self, Shell},
    state::AppState,
};

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
        return Ok(Redirect::to("/admin/exams").into_response());
    }
    if f.rows.trim().is_empty() {
        shell::flash(&session, "Paste at least one result row.").await?;
        return Ok(Redirect::to(&format!("/admin/exams?programme={programme_id}")).into_response());
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
    )
    .await?;
    Ok(Redirect::to(&format!("/admin/exams/results/preview?batch={id}")).into_response())
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
        return Ok(Redirect::to("/admin/exams").into_response());
    }
    let file = form.file("csv_file").ok_or(AppError::BadRequest("Choose a CSV file to upload.".into()))?;
    if file.bytes.len() > 5 * 1024 * 1024 {
        shell::flash(&session, "That CSV file is larger than 5 MB — split it into smaller batches.").await?;
        return Ok(Redirect::to(&format!("/admin/exams?programme={programme_id}")).into_response());
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
    )
    .await?;
    Ok(Redirect::to(&format!("/admin/exams/results/preview?batch={id}")).into_response())
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
    let (valid, error, duplicate, conflict) = result_import::counts(&rows);
    Ok(PreviewTemplate {
        shell: Shell::build(&manager.user, &session).await?,
        batch_id: batch.id,
        source_name: batch.source_name,
        semester: batch.semester,
        kind: batch.exam_kind,
        valid,
        error,
        duplicate,
        conflict,
        rows,
    })
}

/// File the staged rows. Refuses when any row would overwrite a finalized
/// result; the message says so and nothing is written.
pub async fn commit(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Form(f): Form<BatchForm>,
) -> Result<Response, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    if let Err(redirect) = require_reauth(&session, "/admin/marks").await {
        shell::flash(&session, "Re-enter your password to commit an import.").await?;
        return Ok(redirect.into_response());
    }
    let Some(id) = parse_i64(&f.batch) else {
        return Err(AppError::NotFound);
    };
    let batch = result_import::batch(&s.db, id).await?.ok_or(AppError::NotFound)?;
    let back = format!(
        "/admin/exams?programme={}&semester={}",
        batch.programme_id.unwrap_or(0),
        batch.semester
    );
    match result_import::commit(&s.db, id, manager.department()).await {
        Ok(done) => {
            users::audit(&s.db, Some(manager.user.id), "results_import_committed", "import_batch", Some(id)).await?;
            shell::flash(&session, format!("Filed {} result row(s).", done.filed)).await?;
            Ok(Redirect::to(&back).into_response())
        }
        Err(sqlx::Error::Protocol(msg)) => {
            shell::flash(&session, msg).await?;
            Ok(Redirect::to(&format!("/admin/exams/results/preview?batch={id}")).into_response())
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
    Ok(Redirect::to("/admin/exams"))
}
