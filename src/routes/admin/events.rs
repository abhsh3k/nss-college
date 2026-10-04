//! Events: list, add, edit, archive/restore and permanently delete.

use askama::Template;
use axum::{
    extract::{Form, Multipart, Path, Query, State},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use tower_sessions::Session;

use super::{audiences, normalise_datetime, statuses, TokenForm};
use crate::{
    auth::{csrf, AuthUser, OfficeOrAdmin},
    error::AppError,
    services::{
        content_admin::{self, EventInput, EventRow},
        users,
    },
    shell::{flash, Shell},
    state::AppState,
    uploads::{self, ParsedForm},
};

const FOLDER: &str = "events";
const LIST: &str = "/admin/events";

// ---------- List page ----------

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/events.html")]
pub struct IndexPage {
    shell: Shell,
    events: Vec<EventRow>,
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
        events: content_admin::events(&s.db, &filter).await?,
        filter,
        statuses: statuses(),
    })
}

// ---------- Add and edit form ----------

pub struct EventFormValues {
    pub id: i64,
    pub title: String,
    pub description: String,
    pub location: String,
    /// Raw `datetime-local` values, so a failed save can be shown back unchanged.
    pub starts_value: String,
    pub ends_value: String,
    pub image_path: String,
    pub audience: String,
    pub status: String,
    pub error: Option<String>,
}

impl EventFormValues {
    fn blank() -> EventFormValues {
        EventFormValues {
            id: 0,
            title: String::new(),
            description: String::new(),
            location: String::new(),
            starts_value: String::new(),
            ends_value: String::new(),
            image_path: String::new(),
            audience: "public".into(),
            status: "draft".into(),
            error: None,
        }
    }
}

#[derive(Template)]
#[template(path = "admin/event_form.html")]
pub struct FormPage {
    shell: Shell,
    form: EventFormValues,
    statuses: Vec<String>,
    audiences: Vec<String>,
}

fn form_page(form: EventFormValues, shell: Shell) -> FormPage {
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
    Ok(form_page(EventFormValues::blank(), Shell::build(&user, &session).await?))
}

pub async fn edit_form(
    State(s): State<AppState>,
    session: Session,
    OfficeOrAdmin(user): OfficeOrAdmin,
    Path(id): Path<i64>,
) -> Result<FormPage, AppError> {
    let row = content_admin::event_for_edit(&s.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(form_page(
        EventFormValues {
            id: row.id,
            title: row.title,
            description: row.description,
            location: row.location,
            starts_value: row.starts_value,
            ends_value: row.ends_value,
            image_path: row.image_path,
            audience: row.audience,
            status: row.status,
            error: None,
        },
        Shell::build(&user, &session).await?,
    ))
}

async fn read_form(
    multipart: Multipart,
    session: &Session,
    existing: EventFormValues,
) -> Result<(EventFormValues, ParsedForm), AppError> {
    let body = uploads::read(multipart).await?;
    csrf::verify(session, body.field("csrf_token")).await?;

    let mut values = EventFormValues {
        id: existing.id,
        title: body.field("title").to_string(),
        description: body.field("description").to_string(),
        location: body.field("location").to_string(),
        starts_value: body.field("starts_at").to_string(),
        ends_value: body.field("ends_at").to_string(),
        image_path: existing.image_path,
        audience: content_admin::clean_audience(body.field("audience")).to_string(),
        status: content_admin::clean_status(body.field("status")).to_string(),
        error: None,
    };

    if values.title.is_empty() {
        values.error = Some("Give the event a title.".into());
        return Ok((values, body));
    }
    if normalise_datetime(&values.starts_value).is_none() {
        values.error = Some("Enter a valid start date and time.".into());
        return Ok((values, body));
    }
    if !values.ends_value.is_empty() && normalise_datetime(&values.ends_value).is_none() {
        values.error = Some("Enter a valid end date and time, or leave it empty.".into());
    }
    Ok((values, body))
}

async fn store_image(
    s: &AppState,
    body: &ParsedForm,
    values: &mut EventFormValues,
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

/// Convert the validated datetime-local values into what the service stores.
fn to_input(values: &EventFormValues) -> Result<EventInput, AppError> {
    let starts_at = normalise_datetime(&values.starts_value)
        .ok_or_else(|| AppError::BadRequest("Enter a valid start date and time.".into()))?;
    let ends_at = if values.ends_value.trim().is_empty() {
        String::new()
    } else {
        normalise_datetime(&values.ends_value).ok_or_else(|| {
            AppError::BadRequest("Enter a valid end date and time.".into())
        })?
    };
    Ok(EventInput {
        title: values.title.clone(),
        description: values.description.clone(),
        location: values.location.clone(),
        starts_at,
        ends_at,
        image_path: values.image_path.clone(),
        audience: values.audience.clone(),
        status: values.status.clone(),
    })
}

async fn with_error(
    user: &AuthUser,
    session: &Session,
    mut values: EventFormValues,
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
    let (mut values, body) = read_form(multipart, &session, EventFormValues::blank()).await?;

    store_image(&s, &body, &mut values, user.id).await?;
    if values.error.is_some() {
        return with_error(&user, &session, values).await;
    }

    let input = to_input(&values)?;
    let id = content_admin::create_event(&s.db, &input, Some(user.id)).await?;
    users::audit(&s.db, Some(user.id), "event_created", "event", Some(id)).await?;
    flash(&session, "Event saved.").await?;
    Ok(Redirect::to(LIST).into_response())
}

pub async fn update(
    State(s): State<AppState>,
    session: Session,
    OfficeOrAdmin(user): OfficeOrAdmin,
    Path(id): Path<i64>,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let current = content_admin::event_for_edit(&s.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    let existing = EventFormValues {
        id: current.id,
        title: current.title,
        description: current.description,
        location: current.location,
        starts_value: current.starts_value,
        ends_value: current.ends_value,
        image_path: current.image_path,
        audience: current.audience,
        status: current.status,
        error: None,
    };
    let (mut values, body) = read_form(multipart, &session, existing).await?;
    // Remember the current poster so a replaced file can be cleaned up.
    let previous = values.image_path.clone();

    store_image(&s, &body, &mut values, user.id).await?;
    if values.error.is_some() {
        return with_error(&user, &session, values).await;
    }

    let input = to_input(&values)?;
    content_admin::update_event(&s.db, id, &input).await?;

    // Only once the row points at the new poster is it safe to drop the old one.
    if values.image_path != previous {
        content_admin::discard_upload(&s.db, &s.upload_dir, &previous).await;
    }
    users::audit(&s.db, Some(user.id), "event_updated", "event", Some(id)).await?;
    flash(&session, "Event updated.").await?;
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

    if content_admin::event_for_edit(&s.db, id).await?.is_none() {
        return Err(AppError::NotFound);
    }

    let status = content_admin::clean_status(&f.status);
    content_admin::set_event_status(&s.db, id, status).await?;
    users::audit(
        &s.db,
        Some(user.id),
        if status == "archived" {
            "event_archived"
        } else {
            "event_published"
        },
        "event",
        Some(id),
    )
    .await?;
    flash(
        &session,
        if status == "archived" {
            "Event archived. It is hidden from the site but still recoverable."
        } else {
            "Event published."
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

    if content_admin::event_for_edit(&s.db, id).await?.is_none() {
        return Err(AppError::NotFound);
    }

    let path = content_admin::upload_path_for_event(&s.db, id).await.ok().flatten();
    content_admin::delete_event(&s.db, id).await?;
    content_admin::discard_upload(&s.db, &s.upload_dir, path.as_deref().unwrap_or("")).await;
    users::audit(&s.db, Some(user.id), "event_deleted", "event", Some(id)).await?;
    flash(&session, "Event permanently deleted.").await?;
    Ok(Redirect::to(&back))
}