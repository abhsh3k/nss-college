//! News items: list, add, edit, archive/restore and permanently delete.

use askama::Template;
use axum::{
    extract::{Form, Multipart, Path, Query, State},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use tower_sessions::Session;

use super::{statuses, TokenForm};
use crate::{
    auth::{csrf, AuthUser, OfficeOrAdmin},
    error::AppError,
    services::{
        content_admin::{self, NewsInput, NewsRow},
        users,
    },
    shell::{flash, Shell},
    state::AppState,
    uploads::{self, ParsedForm},
};

const FOLDER: &str = "news";
const LIST: &str = "/admin/news";

// ---------- List page ----------

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/news.html")]
pub struct IndexPage {
    shell: Shell,
    news: Vec<NewsRow>,
    filter: String,
    statuses: Vec<String>,
}

pub async fn index(
    State(s): State<AppState>,
    session: Session,
    OfficeOrAdmin(user): OfficeOrAdmin,
    Query(q): Query<ListQuery>,
) -> Result<IndexPage, AppError> {
    let filter = match q.status.as_deref() {
        None | Some("") => String::new(),
        Some(v) => content_admin::clean_status(v).to_string(),
    };
    Ok(IndexPage {
        shell: Shell::build(&user, &session).await?,
        news: content_admin::news(&s.db, &filter).await?,
        filter,
        statuses: statuses(),
    })
}

// ---------- Add and edit form ----------

pub struct NewsFormValues {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub image_path: String,
    pub status: String,
    pub error: Option<String>,
}

impl NewsFormValues {
    fn blank() -> NewsFormValues {
        NewsFormValues {
            id: 0,
            title: String::new(),
            body: String::new(),
            image_path: String::new(),
            status: "draft".into(),
            error: None,
        }
    }

    fn to_input(&self) -> NewsInput {
        NewsInput {
            title: self.title.clone(),
            body: self.body.clone(),
            image_path: self.image_path.clone(),
            status: self.status.clone(),
        }
    }
}

#[derive(Template)]
#[template(path = "admin/news_form.html")]
pub struct FormPage {
    shell: Shell,
    form: NewsFormValues,
    statuses: Vec<String>,
}

fn form_page(form: NewsFormValues, shell: Shell) -> FormPage {
    FormPage {
        shell,
        form,
        statuses: statuses(),
    }
}

pub async fn new_form(
    State(_s): State<AppState>,
    session: Session,
    OfficeOrAdmin(user): OfficeOrAdmin,
) -> Result<FormPage, AppError> {
    Ok(form_page(NewsFormValues::blank(), Shell::build(&user, &session).await?))
}

pub async fn edit_form(
    State(s): State<AppState>,
    session: Session,
    OfficeOrAdmin(user): OfficeOrAdmin,
    Path(id): Path<i64>,
) -> Result<FormPage, AppError> {
    let row = content_admin::news_for_edit(&s.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(form_page(
        NewsFormValues {
            id: row.id,
            title: row.title,
            body: row.body,
            image_path: row.image_path,
            status: row.status,
            error: None,
        },
        Shell::build(&user, &session).await?,
    ))
}

async fn read_form(
    multipart: Multipart,
    session: &Session,
    existing: NewsFormValues,
) -> Result<(NewsFormValues, ParsedForm), AppError> {
    let body = uploads::read(multipart).await?;
    csrf::verify(session, body.field("csrf_token")).await?;

    let values = NewsFormValues {
        id: existing.id,
        title: body.field("title").to_string(),
        body: body.field("body").to_string(),
        image_path: existing.image_path,
        status: content_admin::clean_status(body.field("status")).to_string(),
        error: None,
    };
    let mut values = values;
    if values.title.is_empty() {
        values.error = Some("Give the news item a title.".into());
    }
    Ok((values, body))
}

async fn store_image(
    s: &AppState,
    body: &ParsedForm,
    values: &mut NewsFormValues,
    user_id: i64,
) -> Result<(), AppError> {
    if body.file("image").is_none() {
        return Ok(());
    }
    match uploads::save_optional(
        &s.db,
        &s.upload_dir,
        FOLDER,
        body,
        "image",
        "",
        true,
        Some(user_id),
    )
    .await
    {
        Ok(path) => {
            values.image_path = path;
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
    mut values: NewsFormValues,
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
    let (mut values, body) = read_form(multipart, &session, NewsFormValues::blank()).await?;

    store_image(&s, &body, &mut values, user.id).await?;
    if values.error.is_some() {
        return with_error(&user, &session, values).await;
    }

    let id = content_admin::create_news(&s.db, &values.to_input(), Some(user.id)).await?;
    users::audit(&s.db, Some(user.id), "news_created", "news", Some(id)).await?;
    flash(&session, "News item saved.").await?;
    Ok(Redirect::to(LIST).into_response())
}

pub async fn update(
    State(s): State<AppState>,
    session: Session,
    OfficeOrAdmin(user): OfficeOrAdmin,
    Path(id): Path<i64>,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let current = content_admin::news_for_edit(&s.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    let existing = NewsFormValues {
        id: current.id,
        title: current.title,
        body: current.body,
        image_path: current.image_path,
        status: current.status,
        error: None,
    };
    let (mut values, body) = read_form(multipart, &session, existing).await?;
    // Remember the current photo so a replaced file can be cleaned up.
    let previous = values.image_path.clone();

    store_image(&s, &body, &mut values, user.id).await?;
    if values.error.is_some() {
        return with_error(&user, &session, values).await;
    }

    content_admin::update_news(&s.db, id, &values.to_input()).await?;

    // Only once the row points at the new photo is it safe to drop the old one.
    if values.image_path != previous {
        content_admin::discard_upload(&s.db, &s.upload_dir, &previous).await;
    }
    users::audit(&s.db, Some(user.id), "news_updated", "news", Some(id)).await?;
    flash(&session, "News item updated.").await?;
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

    if content_admin::news_for_edit(&s.db, id).await?.is_none() {
        return Err(AppError::NotFound);
    }

    let status = content_admin::clean_status(&f.status);
    content_admin::set_news_status(&s.db, id, status).await?;
    users::audit(
        &s.db,
        Some(user.id),
        if status == "archived" {
            "news_archived"
        } else {
            "news_published"
        },
        "news",
        Some(id),
    )
    .await?;
    flash(
        &session,
        if status == "archived" {
            "News item archived. It is hidden from the site but still recoverable."
        } else {
            "News item published."
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

    if content_admin::news_for_edit(&s.db, id).await?.is_none() {
        return Err(AppError::NotFound);
    }

    let path = content_admin::upload_path_for_news(&s.db, id).await.ok().flatten();
    content_admin::delete_news(&s.db, id).await?;
    content_admin::discard_upload(&s.db, &s.upload_dir, path.as_deref().unwrap_or("")).await;
    users::audit(&s.db, Some(user.id), "news_deleted", "news", Some(id)).await?;
    flash(&session, "News item permanently deleted.").await?;
    Ok(Redirect::to(&back))
}