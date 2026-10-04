//! Rank holders: the students who placed among the top ranks in the university
//! examinations, and how that list is presented.
//!
//! The list screen and the form follow the same shape as the documents screen
//! next door, so an admin who has used one knows the other. Two extras belong
//! here because they only make sense for a list of people: reordering within an
//! exam year, and the display options that choose between cards and paragraphs.

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
    layout::{self, DisplayOptions},
    services::{
        content,
        content_admin,
        rank_holders as svc,
        rank_holders::{RankHolderInput, RankHolderRow},
        site_admin,
        users,
    },
    shell::{flash, Shell},
    state::AppState,
    uploads::{self, ParsedForm},
};

const LIST: &str = "/admin/rank-holders";

/// Photos live in their own folder so a rank holder's picture is easy to find on
/// disk and cannot be mistaken for an attachment.
const FOLDER: &str = "rank-holders";

// ---------- List page ----------

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/rank_holders.html")]
pub struct IndexPage {
    shell: Shell,
    holders: Vec<RankHolderRow>,
    filter: String,
    statuses: Vec<String>,
    /// The display options for the public list, shown on this screen so an admin
    /// can reach the layout without hunting for it on the settings page.
    opts: DisplayOptions,
    /// The heading and intro the public page shows above the list.
    heading: String,
    body: String,
    photo: Option<String>,
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

    // The heading and intro come from the `rank_holders` home section, the same
    // row the layout is read from, so both are edited in one place.
    let row: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT heading, body, photo_path FROM home_sections WHERE section_key = $1",
    )
    .bind(layout::RANK_HOLDERS_KEY)
    .fetch_optional(&s.db)
    .await?;
    let (heading, body, photo) =
        row.unwrap_or_else(|| ("University rank holders".into(), String::new(), None));

    Ok(IndexPage {
        holders: svc::rank_holders(&s.db, &filter).await?,
        opts: svc::rank_holders_display(&s.db).await?,
        heading,
        body,
        photo,
        filter,
        statuses: statuses(),
        shell: Shell::build(&user, &session).await?,
    })
}

// ---------- Add and edit form ----------

/// The form's fields, held in one struct so a rejected save can be redisplayed
/// with what the admin typed rather than an empty form.
pub struct HolderForm {
    pub id: i64,
    pub name: String,
    pub rank: String,
    pub year: String,
    /// Kept as the submitted string so a bad choice can be redisplayed and
    /// corrected, while a valid one is written as an id.
    pub department_id: String,
    pub photo: String,
    pub status: String,
    pub error: Option<String>,
}

impl HolderForm {
    fn blank() -> HolderForm {
        HolderForm {
            id: 0,
            name: String::new(),
            rank: "1".into(),
            year: String::new(),
            department_id: String::new(),
            photo: String::new(),
            status: "published".into(),
            error: None,
        }
    }

    /// The values ready for the database, or the message explaining why not.
    fn to_input(&self) -> Result<RankHolderInput, String> {
        if self.name.trim().is_empty() {
            return Err("Give the student a name.".into());
        }
        let rank = self.rank.trim().parse::<i32>().map_err(|_| {
            "The rank must be a whole number, such as 1.".to_string()
        })?;
        if rank < 1 {
            return Err("A rank starts at 1.".into());
        }
        // The exam year is a year, so anything outside a plausible range is a
        // typo rather than a real value.
        let year = self.year.trim().parse::<i32>().map_err(|_| {
            "The exam year must be a whole year, such as 2025.".to_string()
        })?;
        if !(1990..=2100).contains(&year) {
            return Err("The exam year should be between 1990 and 2100.".into());
        }
        let department_id = match self.department_id.trim().parse::<i64>() {
            Ok(id) if id > 0 => Some(id),
            _ => None,
        };
        Ok(RankHolderInput {
            name: self.name.trim().to_string(),
            rank,
            year,
            department_id,
            photo: self.photo.clone(),
            status: self.status.clone(),
        })
    }
}

#[derive(Template)]
#[template(path = "admin/rank_holder_form.html")]
pub struct FormPage {
    shell: Shell,
    form: HolderForm,
    departments: Vec<svc::DepartmentOption>,
    statuses: Vec<String>,
}

async fn form_page(
    s: &AppState,
    user: &AuthUser,
    session: &Session,
    form: HolderForm,
) -> Result<FormPage, AppError> {
    Ok(FormPage {
        departments: svc::departments(&s.db).await?,
        statuses: statuses(),
        shell: Shell::build(user, session).await?,
        form,
    })
}

pub async fn new_form(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
) -> Result<FormPage, AppError> {
    let mut form = HolderForm::blank();
    // A new holder defaults to the year the admin is working in, which is almost
    // always the one they mean.
    form.year = content::current_year().to_string();
    form_page(&s, &user, &session, form).await
}

pub async fn edit_form(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
) -> Result<FormPage, AppError> {
    let row = svc::rank_holder_for_edit(&s.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    form_page(
        &s,
        &user,
        &session,
        HolderForm {
            id: row.id,
            name: row.name,
            rank: row.rank.to_string(),
            year: row.year.to_string(),
            department_id: row.department_id.map(|d| d.to_string()).unwrap_or_default(),
            photo: row.photo.unwrap_or_default(),
            status: row.status,
            error: None,
        },
    )
    .await
}

/// Turn the form into database values, or set the error and report why not.
///
/// Both `create` and `update` go through this so the two screens cannot drift
/// apart in what they accept.
fn validate(form: &HolderForm) -> Result<RankHolderInput, String> {
    form.to_input()
}

/// Read the posted fields, keeping the stored photo unless a new one arrives.
fn read_form(body: &ParsedForm, existing: &HolderForm) -> HolderForm {
    HolderForm {
        id: existing.id,
        name: body.field("name").to_string(),
        rank: body.field("rank").to_string(),
        year: body.field("exam_year").to_string(),
        department_id: body.field("department_id").to_string(),
        // An empty file input must not wipe the photo the row already owns.
        photo: existing.photo.clone(),
        status: content_admin::clean_status(body.field("status")).to_string(),
        error: None,
    }
}

/// Store a posted photo onto the form values, turning a rejected upload into a
/// form error rather than a failed request.
///
/// Ticking "remove" clears the photo, which is the only way to take one off:
/// an empty file input means "keep what is there".
async fn store_photo(
    s: &AppState,
    body: &ParsedForm,
    form: &mut HolderForm,
    user_id: i64,
) -> Result<(), AppError> {
    if body.flag("remove_photo") {
        form.photo = String::new();
        return Ok(());
    }
    if body.file("photo").is_none() {
        return Ok(());
    }
    match uploads::save_optional(
        &s.db,
        &s.upload_dir,
        FOLDER,
        body,
        "photo",
        "",
        true,
        Some(user_id),
    )
    .await
    {
        Ok(path) => {
            form.photo = path;
            Ok(())
        }
        Err(AppError::BadRequest(msg)) => {
            form.error = Some(msg);
            Ok(())
        }
        Err(e) => Err(e),
    }
}

pub async fn create(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let body = uploads::read(multipart).await?;
    csrf::verify(&session, body.field("csrf_token")).await?;

    let mut form = read_form(&body, &HolderForm::blank());
    store_photo(&s, &body, &mut form, user.id).await?;

    // The photo is stored first, because whether one arrived is only known now,
    // so any validation failure below must undo it.
    let input = match validate(&mut form) {
        Ok(input) => input,
        Err(msg) => {
            form.error = Some(msg);
            // Nothing points at this photo yet, so it must not be left on disk.
            content_admin::discard_upload(&s.db, &s.upload_dir, &form.photo).await;
            return Ok(form_page(&s, &user, &session, form).await?.into_response());
        }
    };

    let id = svc::create_rank_holder(&s.db, &input).await?;
    users::audit(&s.db, Some(user.id), "rank_holder_created", "rank_holder", Some(id)).await?;
    flash(&session, "Rank holder added.").await?;
    Ok(Redirect::to(LIST).into_response())
}

pub async fn update(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
    multipart: Multipart,
) -> Result<Response, AppError> {
    if svc::rank_holder_for_edit(&s.db, id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    let body = uploads::read(multipart).await?;
    csrf::verify(&session, body.field("csrf_token")).await?;

    let previous = svc::photo_for_rank_holder(&s.db, id).await?.unwrap_or_default();
    let existing = HolderForm {
        id,
        photo: previous.clone(),
        ..HolderForm::blank()
    };

    let mut form = read_form(&body, &existing);
    store_photo(&s, &body, &mut form, user.id).await?;

    let input = match validate(&mut form) {
        Ok(input) => input,
        Err(msg) => {
            form.error = Some(msg);
            // The row still points at the old photo, so a rejected replacement
            // must not be left on disk. Only a genuinely new path is dropped.
            if form.photo != previous {
                content_admin::discard_upload(&s.db, &s.upload_dir, &form.photo).await;
            }
            return Ok(form_page(&s, &user, &session, form).await?.into_response());
        }
    };

    svc::update_rank_holder(&s.db, id, &input).await?;
    // Only once the row points at the new photo is it safe to drop the old one.
    if form.photo != previous {
        content_admin::discard_upload(&s.db, &s.upload_dir, &previous).await;
    }
    users::audit(&s.db, Some(user.id), "rank_holder_updated", "rank_holder", Some(id)).await?;
    flash(&session, "Rank holder updated.").await?;
    Ok(Redirect::to(LIST).into_response())
}

// ---------- Archive, delete and reorder ----------

pub async fn set_status(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let back = super::safe_back(&f.back, LIST);
    if svc::rank_holder_for_edit(&s.db, id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    let status = content_admin::clean_status(&f.status);
    svc::set_rank_holder_status(&s.db, id, status).await?;
    users::audit(&s.db, Some(user.id), "rank_holder_status_changed", "rank_holder", Some(id))
        .await?;
    flash(
        &session,
        if status == "archived" {
            "Rank holder archived. Hidden from the public list but recoverable."
        } else {
            "Rank holder published."
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
    if svc::rank_holder_for_edit(&s.db, id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    let photo = svc::photo_for_rank_holder(&s.db, id).await?;
    svc::delete_rank_holder(&s.db, id).await?;
    content_admin::discard_upload(&s.db, &s.upload_dir, photo.as_deref().unwrap_or("")).await;
    users::audit(&s.db, Some(user.id), "rank_holder_deleted", "rank_holder", Some(id)).await?;
    flash(&session, "Rank holder permanently deleted.").await?;
    Ok(Redirect::to(&back))
}

#[derive(Deserialize)]
pub struct MoveSubmit {
    csrf_token: String,
    #[serde(default)]
    up: String,
}

pub async fn reorder(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
    Form(f): Form<MoveSubmit>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    // The button carries "up" when ticked, so its presence means move up.
    svc::move_rank_holder(&s.db, id, !f.up.is_empty()).await?;
    users::audit(&s.db, Some(user.id), "rank_holder_moved", "rank_holder", Some(id)).await?;
    Ok(Redirect::to(LIST))
}

// ---------- How the public list is displayed ----------

/// The heading, intro and layout of the public rank holders list.
///
/// This lives with the list rather than on the settings screen because the
/// options only mean something next to the people they are shown on.
///
/// The body is read through `uploads::read` rather than `Form` because the
/// shared options fragment carries a photo upload, and a urlencoded body cannot
/// carry a file.
pub async fn save_display(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let body = uploads::read(multipart).await?;
    csrf::verify(&session, body.field("csrf_token")).await?;

    let opts = layout::from_form(&body);

    // An empty file input must not wipe the photo the row already owns, so the
    // stored path stands in when nothing was posted.
    let previous = site_admin::home_section_photo(&s.db, layout::RANK_HOLDERS_KEY)
        .await?
        .unwrap_or_default();
    let photo = match uploads::save_optional(
        &s.db,
        &s.upload_dir,
        FOLDER,
        &body,
        "photo",
        &previous,
        true,
        Some(user.id),
    )
    .await
    {
        Ok(path) => {
            if body.flag("remove_photo") {
                // A replacement was just stored but the admin asked for no photo
                // at all, so it must not be left on disk.
                content_admin::discard_upload(&s.db, &s.upload_dir, &path).await;
            }
            path
        }
        Err(AppError::BadRequest(msg)) => {
            flash(&session, &msg).await?;
            return Ok(Redirect::to(LIST).into_response());
        }
        Err(e) => return Err(e),
    };
    // `save_optional` returns whatever was already stored when no file arrives,
    // so the stored path is cleared explicitly when the box was ticked.
    let photo = if body.flag("remove_photo") {
        String::new()
    } else {
        photo
    };

    // A database predating the layout columns has no `rank_holders` row to write
    // to, and creating one would invent content the templates never seeded.
    let saved = svc::save_rank_holders_display(
        &s.db,
        body.field("heading"),
        body.field("body"),
        &photo,
        &opts,
    )
    .await?;

    if !saved {
        // Nothing will point at the photo, so do not leave it on disk.
        content_admin::discard_upload(&s.db, &s.upload_dir, &photo).await;
        flash(&session, "That list could not be found.").await?;
        return Ok(Redirect::to(LIST).into_response());
    }

    // Only once the row points at the new photo is it safe to drop the old one.
    if photo != previous {
        content_admin::discard_upload(&s.db, &s.upload_dir, &previous).await;
    }

    users::audit(&s.db, Some(user.id), "rank_holders_display_updated", "home_section", None)
        .await?;
    flash(&session, "Rank holders list display saved.").await?;
    Ok(Redirect::to(LIST).into_response())
}