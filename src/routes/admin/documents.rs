//! Documents: the downloadable files the college publishes (forms, reports, PDFs).

use askama::Template;
use axum::{
    extract::{Form, Multipart, Path, Query, State},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use tower_sessions::Session;

use super::{statuses, TokenForm};
use crate::{
    auth::{csrf, AuthUser, AdminOnly},
    error::AppError,
    services::{
        content_admin,
        site_admin::{self, DocumentInput},
        users,
    },
    shell::{flash, Shell},
    state::AppState,
    uploads::{self, ParsedForm},
};

const FOLDER: &str = "documents";
const LIST: &str = "/admin/documents";

// ---------- List page ----------

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/documents.html")]
pub struct IndexPage {
    shell: Shell,
    documents: Vec<site_admin::DocumentRow>,
    filter: String,
    statuses: Vec<String>,
}

pub async fn index(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Query(q): Query<ListQuery>,
) -> Result<IndexPage, AppError> {
    let filter = match q.status.as_deref() {
        None | Some("") => String::new(),
        Some(v) => content_admin::clean_status(v).to_string(),
    };
    Ok(IndexPage {
        shell: Shell::build(&user, &session).await?,
        documents: site_admin::documents(&s.db, &filter).await?,
        filter,
        statuses: statuses(),
    })
}

// ---------- Add and edit form ----------

pub struct DocumentFormValues {
    pub id: i64,
    pub title: String,
    pub category: String,
    pub file_path: String,
    pub file_name: String,
    pub status: String,
    pub error: Option<String>,
}

impl DocumentFormValues {
    fn blank() -> DocumentFormValues {
        DocumentFormValues {
            id: 0,
            title: String::new(),
            category: "General".into(),
            file_path: String::new(),
            file_name: String::new(),
            status: "draft".into(),
            error: None,
        }
    }

    fn to_input(&self) -> DocumentInput {
        DocumentInput {
            title: self.title.clone(),
            category: self.category.clone(),
            file_path: self.file_path.clone(),
            status: self.status.clone(),
        }
    }
}

#[derive(Template)]
#[template(path = "admin/document_form.html")]
pub struct FormPage {
    shell: Shell,
    form: DocumentFormValues,
    statuses: Vec<String>,
}

fn form_page(form: DocumentFormValues, shell: Shell) -> FormPage {
    FormPage {
        shell,
        form,
        statuses: statuses(),
    }
}

pub async fn new_form(
    State(_s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
) -> Result<FormPage, AppError> {
    Ok(form_page(DocumentFormValues::blank(), Shell::build(&user, &session).await?))
}

pub async fn edit_form(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
) -> Result<FormPage, AppError> {
    let row = site_admin::document_for_edit(&s.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(form_page(
        DocumentFormValues {
            id: row.id,
            title: row.title,
            category: row.category,
            file_name: row.file_path.rsplit('/').next().unwrap_or("").to_string(),
            file_path: row.file_path,
            status: row.status,
            error: None,
        },
        Shell::build(&user, &session).await?,
    ))
}

async fn read_form(
    multipart: Multipart,
    session: &Session,
    existing: DocumentFormValues,
) -> Result<(DocumentFormValues, ParsedForm), AppError> {
    let body = uploads::read(multipart).await?;
    csrf::verify(session, body.field("csrf_token")).await?;

    let mut values = DocumentFormValues {
        id: existing.id,
        title: body.field("title").to_string(),
        category: body.field("category").to_string(),
        // Keep the stored file unless a replacement is posted below.
        file_path: existing.file_path,
        file_name: existing.file_name,
        status: content_admin::clean_status(body.field("status")).to_string(),
        error: None,
    };
    if values.category.is_empty() {
        values.category = "General".into();
    }
    if values.title.is_empty() {
        values.error = Some("Give the document a title.".into());
    }
    Ok((values, body))
}

/// Write the posted file, if any, onto `values`.
async fn store_file(
    s: &AppState,
    body: &ParsedForm,
    values: &mut DocumentFormValues,
    user_id: i64,
) -> Result<(), AppError> {
    if body.file("file").is_none() {
        return Ok(());
    }
    match uploads::save_optional(
        &s.db,
        &s.upload_dir,
        FOLDER,
        body,
        "file",
        "",
        false,
        Some(user_id),
    )
    .await
    {
        Ok(path) => {
            values.file_path = path.clone();
            values.file_name = path.rsplit('/').next().unwrap_or("").to_string();
            Ok(())
        }
        Err(AppError::BadRequest(msg)) => {
            values.error = Some(msg);
            Ok(())
        }
        Err(e) => Err(e),
    }
}

async fn with_error(
    user: &AuthUser,
    session: &Session,
    mut values: DocumentFormValues,
) -> Result<Response, AppError> {
    let msg = values
        .error
        .clone()
        .unwrap_or_else(|| "Check the form and try again.".into());
    values.error = Some(msg);
    Ok(form_page(values, Shell::build(user, session).await?).into_response())
}

pub async fn create(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let (mut values, body) = read_form(multipart, &session, DocumentFormValues::blank()).await?;

    // The file is stored first, because whether one arrived is only known now.
    store_file(&s, &body, &mut values, user.id).await?;
    // A document row with no file behind it is useless, so a new one needs one.
    if values.file_path.is_empty() && values.error.is_none() {
        values.error = Some("Choose a file to upload.".into());
    }
    if values.error.is_some() {
        // Nothing will point at this file, so do not leave it on disk.
        content_admin::discard_upload(&s.db, &s.upload_dir, &values.file_path).await;
        return with_error(&user, &session, values).await;
    }

    let id = site_admin::create_document(&s.db, &values.to_input(), Some(user.id)).await?;
    users::audit(&s.db, Some(user.id), "document_created", "document", Some(id)).await?;
    flash(&session, "Document saved.").await?;
    Ok(Redirect::to(LIST).into_response())
}

pub async fn update(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let current = site_admin::document_for_edit(&s.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    let existing = DocumentFormValues {
        id: current.id,
        title: current.title,
        category: current.category,
        file_path: current.file_path.clone(),
        file_name: String::new(),
        status: current.status,
        error: None,
    };
    let previous = existing.file_path.clone();

    let (mut values, body) = read_form(multipart, &session, existing).await?;
    store_file(&s, &body, &mut values, user.id).await?;
    if values.error.is_some() {
        // The row still points at the old file, so a rejected replacement upload
        // would otherwise be left behind. Only a *new* path is dropped: when no
        // file was posted the path is still the one the row owns.
        if values.file_path != previous {
            content_admin::discard_upload(&s.db, &s.upload_dir, &values.file_path).await;
        }
        return with_error(&user, &session, values).await;
    }

    site_admin::update_document(&s.db, id, &values.to_input()).await?;

    // Only once the row points at the new file is it safe to drop the old one.
    if values.file_path != previous {
        content_admin::discard_upload(&s.db, &s.upload_dir, &previous).await;
    }
    users::audit(&s.db, Some(user.id), "document_updated", "document", Some(id)).await?;
    flash(&session, "Document updated.").await?;
    Ok(Redirect::to(LIST).into_response())
}

// ---------- Archive, restore and permanent delete ----------

pub async fn set_status(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let back = super::safe_back(&f.back, LIST);

    if site_admin::document_for_edit(&s.db, id).await?.is_none() {
        return Err(AppError::NotFound);
    }

    let status = content_admin::clean_status(&f.status);
    site_admin::set_document_status(&s.db, id, status).await?;
    users::audit(
        &s.db,
        Some(user.id),
        if status == "archived" {
            "document_archived"
        } else {
            "document_published"
        },
        "document",
        Some(id),
    )
    .await?;
    flash(
        &session,
        if status == "archived" {
            "Document archived. It is hidden from readers but still recoverable."
        } else {
            "Document published."
        },
    )
    .await?;
    Ok(Redirect::to(&back))
}

pub async fn destroy(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let back = super::safe_back(&f.back, LIST);

    if site_admin::document_for_edit(&s.db, id).await?.is_none() {
        return Err(AppError::NotFound);
    }

    let path = site_admin::upload_path_for_document(&s.db, id).await.ok().flatten();
    site_admin::delete_document(&s.db, id).await?;
    content_admin::discard_upload(&s.db, &s.upload_dir, path.as_deref().unwrap_or("")).await;
    users::audit(&s.db, Some(user.id), "document_deleted", "document", Some(id)).await?;
    flash(&session, "Document permanently deleted.").await?;
    Ok(Redirect::to(&back))
}