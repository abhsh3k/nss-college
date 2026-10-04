//! Notices: list, add, edit, archive/restore and permanently delete.

use askama::Template;
use axum::{
    extract::{Form, Multipart, Path, Query, State},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use tower_sessions::Session;

use super::{audiences, statuses, TokenForm};
use crate::{
    auth::{csrf, AuthUser, OfficeOrAdmin},
    error::AppError,
    services::{
        content_admin::{self, NoticeInput, NoticeRow},
        users,
    },
    shell::{flash, Shell},
    state::AppState,
    uploads::{self, ParsedForm},
};

const FOLDER: &str = "notices";
const LIST: &str = "/admin/notices";

// ---------- List page ----------

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/notices.html")]
pub struct IndexPage {
    shell: Shell,
    notices: Vec<NoticeRow>,
    filter: String,
    statuses: Vec<String>,
}

pub async fn index(
    State(s): State<AppState>,
    session: Session,
    OfficeOrAdmin(user): OfficeOrAdmin,
    Query(q): Query<ListQuery>,
) -> Result<IndexPage, AppError> {
    // An empty filter means "all"; anything else must be a real status.
    let filter = match q.status.as_deref() {
        None | Some("") => String::new(),
        Some(v) => content_admin::clean_status(v).to_string(),
    };
    Ok(IndexPage {
        shell: Shell::build(&user, &session).await?,
        notices: content_admin::notices(&s.db, &filter).await?,
        filter,
        statuses: statuses(),
    })
}

// ---------- Add and edit form ----------

/// The values the form shows, whether it is adding or editing.
pub struct NoticeFormValues {
    pub id: i64,
    pub title: String,
    pub category: String,
    pub body: String,
    pub attachment_path: String,
    pub audience: String,
    pub is_pinned: bool,
    pub status: String,
    pub error: Option<String>,
}

impl NoticeFormValues {
    fn blank() -> NoticeFormValues {
        NoticeFormValues {
            id: 0,
            title: String::new(),
            category: "Notice".into(),
            body: String::new(),
            attachment_path: String::new(),
            audience: "public".into(),
            is_pinned: false,
            status: "draft".into(),
            error: None,
        }
    }

    fn to_input(&self) -> NoticeInput {
        NoticeInput {
            title: self.title.clone(),
            category: self.category.clone(),
            body: self.body.clone(),
            attachment_path: self.attachment_path.clone(),
            audience: self.audience.clone(),
            is_pinned: self.is_pinned,
            status: self.status.clone(),
        }
    }
}

#[derive(Template)]
#[template(path = "admin/notice_form.html")]
pub struct FormPage {
    shell: Shell,
    form: NoticeFormValues,
    statuses: Vec<String>,
    audiences: Vec<String>,
}

fn form_page(form: NoticeFormValues, shell: Shell) -> FormPage {
    FormPage {
        shell,
        form,
        statuses: statuses(),
        audiences: audiences(),
    }
}

pub async fn new_form(
    State(_s): State<AppState>,
    session: Session,
    OfficeOrAdmin(user): OfficeOrAdmin,
) -> Result<FormPage, AppError> {
    Ok(form_page(NoticeFormValues::blank(), Shell::build(&user, &session).await?))
}

async fn edit_page(
    s: &AppState,
    user: &AuthUser,
    session: &Session,
    id: i64,
) -> Result<FormPage, AppError> {
    let row = content_admin::notice_for_edit(&s.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(form_page(
        NoticeFormValues {
            id: row.id,
            title: row.title,
            category: row.category,
            body: row.body,
            attachment_path: row.attachment_path,
            audience: row.audience,
            is_pinned: row.is_pinned,
            status: row.status,
            error: None,
        },
        Shell::build(user, session).await?,
    ))
}

pub async fn edit_form(
    State(s): State<AppState>,
    session: Session,
    OfficeOrAdmin(user): OfficeOrAdmin,
    Path(id): Path<i64>,
) -> Result<FormPage, AppError> {
    edit_page(&s, &user, &session, id).await
}

/// Parse the multipart body into the form values, before anything is saved.
///
/// Returns the values plus the parsed body, so the caller can write the file only
/// once the row itself is about to be written.
async fn read_form(
    multipart: Multipart,
    session: &Session,
    existing: NoticeFormValues,
) -> Result<(NoticeFormValues, ParsedForm), AppError> {
    let body = uploads::read(multipart).await?;

    // The CSRF token rides along as an ordinary field of the multipart body.
    csrf::verify(session, body.field("csrf_token")).await?;

    let mut values = NoticeFormValues {
        id: existing.id,
        title: body.field("title").to_string(),
        category: body.field("category").to_string(),
        body: body.field("body").to_string(),
        // Keep the stored file unless a new one is posted below.
        attachment_path: existing.attachment_path,
        audience: content_admin::clean_audience(body.field("audience")).to_string(),
        is_pinned: body.flag("is_pinned"),
        status: content_admin::clean_status(body.field("status")).to_string(),
        error: None,
    };
    if values.category.is_empty() {
        values.category = "Notice".into();
    }
    if values.title.is_empty() {
        values.error = Some("Give the notice a title.".into());
    }
    Ok((values, body))
}

/// Write the posted attachment, if any, and return the path to store on the row.
async fn store_attachment(
    s: &AppState,
    body: &ParsedForm,
    values: &mut NoticeFormValues,
    user_id: i64,
) -> Result<(), AppError> {
    if body.file("attachment").is_none() {
        return Ok(());
    }
    match uploads::save_optional(
        &s.db,
        &s.upload_dir,
        FOLDER,
        body,
        "attachment",
        "",
        false,
        Some(user_id),
    )
    .await
    {
        Ok(path) => {
            values.attachment_path = path;
            Ok(())
        }
        Err(AppError::BadRequest(msg)) => {
            values.error = Some(msg);
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Re-render the form with the values the user typed and the reason it failed.
async fn with_error(
    user: &AuthUser,
    session: &Session,
    mut values: NoticeFormValues,
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
    OfficeOrAdmin(user): OfficeOrAdmin,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let (mut values, body) = read_form(multipart, &session, NoticeFormValues::blank()).await?;

    store_attachment(&s, &body, &mut values, user.id).await?;
    if let Some(msg) = values.error.clone() {
        values.error = Some(msg);
        return with_error(&user, &session, values).await;
    }

    let id = content_admin::create_notice(&s.db, &values.to_input(), Some(user.id)).await?;
    users::audit(&s.db, Some(user.id), "notice_created", "notice", Some(id)).await?;
    flash(&session, "Notice saved.").await?;
    Ok(Redirect::to(LIST).into_response())
}

pub async fn update(
    State(s): State<AppState>,
    session: Session,
    OfficeOrAdmin(user): OfficeOrAdmin,
    Path(id): Path<i64>,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let current = content_admin::notice_for_edit(&s.db, id)
        .await?
        .ok_or(AppError::NotFound)?;

    let existing = NoticeFormValues {
        id: current.id,
        title: current.title,
        category: current.category,
        body: current.body,
        attachment_path: current.attachment_path,
        audience: current.audience,
        is_pinned: current.is_pinned,
        status: current.status,
        error: None,
    };
    let (mut values, body) = read_form(multipart, &session, existing).await?;
    // Remember what was attached before, so a replaced file can be cleaned up.
    let previous = values.attachment_path.clone();

    store_attachment(&s, &body, &mut values, user.id).await?;
    if values.error.is_some() {
        return with_error(&user, &session, values).await;
    }

    content_admin::update_notice(&s.db, id, &values.to_input()).await?;

    // Only once the row points at the new file is it safe to drop the old one.
    if values.attachment_path != previous {
        content_admin::discard_upload(&s.db, &s.upload_dir, &previous).await;
    }
    users::audit(&s.db, Some(user.id), "notice_updated", "notice", Some(id)).await?;
    flash(&session, "Notice updated.").await?;
    Ok(Redirect::to(LIST).into_response())
}

// ---------- Archive, restore and permanent delete ----------

pub async fn set_status(
    State(s): State<AppState>,
    session: Session,
    OfficeOrAdmin(user): OfficeOrAdmin,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let back = super::safe_back(&f.back, LIST);

    if content_admin::notice_for_edit(&s.db, id).await?.is_none() {
        return Err(AppError::NotFound);
    }

    let status = content_admin::clean_status(&f.status);
    content_admin::set_notice_status(&s.db, id, status).await?;
    users::audit(
        &s.db,
        Some(user.id),
        if status == "archived" {
            "notice_archived"
        } else {
            "notice_published"
        },
        "notice",
        Some(id),
    )
    .await?;
    flash(
        &session,
        if status == "archived" {
            "Notice archived. It is hidden from the site but still recoverable."
        } else {
            "Notice published."
        },
    )
    .await?;
    Ok(Redirect::to(&back))
}

pub async fn destroy(
    State(s): State<AppState>,
    session: Session,
    OfficeOrAdmin(user): OfficeOrAdmin,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let back = super::safe_back(&f.back, LIST);

    if content_admin::notice_for_edit(&s.db, id).await?.is_none() {
        return Err(AppError::NotFound);
    }

    // Read the attachment path first: the row is about to disappear.
    let path = content_admin::upload_path_for_notice(&s.db, id).await.ok().flatten();

    content_admin::delete_notice(&s.db, id).await?;
    content_admin::discard_upload(&s.db, &s.upload_dir, path.as_deref().unwrap_or("")).await;
    users::audit(&s.db, Some(user.id), "notice_deleted", "notice", Some(id)).await?;
    flash(&session, "Notice permanently deleted.").await?;
    Ok(Redirect::to(&back))
}