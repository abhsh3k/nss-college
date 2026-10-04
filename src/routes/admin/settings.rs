//! Settings: the college's own facts and the copy on the home page.
//!
//! Both screens here are `AdminOnly`, because they change what every visitor to
//! the public site reads.

use askama::Template;
use axum::{
    extract::{Multipart, Path, State},
    response::{IntoResponse, Redirect, Response},
};
use tower_sessions::Session;

use crate::{
    auth::{csrf, AdminOnly},
    error::AppError,
    layout::{self, DisplayOptions},
    services::{content_admin, site_admin, site_admin::HomeSectionRow, users},
    shell::{flash, Shell},
    site, state::AppState, uploads,
};

const LIST: &str = "/admin/settings";

/// Home page block photos live in their own folder, apart from the documents and
/// the rank holder pictures.
const FOLDER: &str = "sections";

// ---------- The screen ----------

#[derive(Template)]
#[template(path = "admin/settings.html")]
pub struct IndexPage {
    shell: Shell,
    settings: Vec<site_admin::SettingRow>,
    home_sections: Vec<HomeSectionRow>,
    /// The choices a newly added section starts from.
    defaults: DisplayOptions,
}

pub async fn index(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
) -> Result<IndexPage, AppError> {
    // Read fresh rather than from the settings cache: this row changes rarely,
    // and the screen should show what is stored now.
    let defaults = layout::defaults(&s.db).await?;
    Ok(IndexPage {
        settings: site_admin::settings(&s.db).await?,
        home_sections: site_admin::home_sections(&s.db).await?,
        defaults,
        shell: Shell::build(&user, &session).await?,
    })
}

// ---------- Save the college's facts ----------

/// Every value is posted under its own key, and the service refuses any key the
/// screen does not offer, so the form itself carries the list of keys.
///
/// Only the keys actually present in the body are written. A browser always
/// submits the whole form, but a hand-crafted or partially failed post must not
/// be able to blank out settings it never mentioned.
pub async fn save(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let body = uploads::read(multipart).await?;
    csrf::verify(&session, body.field("csrf_token")).await?;

    let mut written = 0;
    for row in site_admin::settings(&s.db).await? {
        let Some(value) = body.field_opt(&row.key) else {
            continue;
        };
        if site_admin::set_setting(&s.db, &row.key, value).await? {
            written += 1;
        }
    }

    users::audit(&s.db, Some(user.id), "settings_saved", "site_settings", None).await?;
    // Refresh the cache now rather than waiting for the next timer tick, so the
    // change is on the site as soon as this returns.
    if let Err(e) = site::get(&s.db).await {
        tracing::warn!(error = ?e, "could not refresh site settings after saving");
    }
    flash(
        &session,
        &format!("{written} setting(s) saved and applied to the public site."),
    )
    .await?;
    Ok(Redirect::to(LIST).into_response())
}

// ---------- Save one block of home page copy ----------

/// One small form per block, so a long page does not post every block at once.
///
/// The body is read through `uploads::read` rather than `Form` because the block
/// form now carries a photo upload, and a urlencoded body cannot carry a file.
pub async fn save_home_section(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(key): Path<String>,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let body = uploads::read(multipart).await?;
    csrf::verify(&session, body.field("csrf_token")).await?;

    // A home block keeps the photo it already has unless a new file is posted,
    // so an empty file input cannot blank the image out. Ticking "remove" is the
    // only way to take a photo off a block that has one.
    let previous = site_admin::home_section_photo(&s.db, &key)
        .await?
        .unwrap_or_default();
    let removing = body.flag("remove_photo");
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
            if removing {
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
    let photo = if removing { String::new() } else { photo };

    let opts = layout::from_form(&body);

    let saved = site_admin::update_home_section(
        &s.db,
        &key,
        body.field("heading"),
        body.field("body"),
        &photo,
        &opts,
    )
    .await?;
    if !saved {
        // Nothing points at the photo, so do not leave it on disk.
        content_admin::discard_upload(&s.db, &s.upload_dir, &photo).await;
        return Err(AppError::NotFound);
    }

    // Only once the row points at the new photo is it safe to drop the old one.
    if photo != previous {
        content_admin::discard_upload(&s.db, &s.upload_dir, &previous).await;
    }

    users::audit(
        &s.db,
        Some(user.id),
        "home_section_updated",
        "home_section",
        None,
    )
    .await?;
    flash(&session, "Home page block saved.").await?;
    Ok(Redirect::to(LIST).into_response())
}

// ---------- Save the default a new section starts from ----------

/// The site-wide starting point for a new section.
///
/// Only the six display choices are stored here; there is no photo, because a
/// default image shared by every section would be meaningless.
pub async fn save_display_defaults(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let body = uploads::read(multipart).await?;
    csrf::verify(&session, body.field("csrf_token")).await?;

    layout::save_defaults(&s.db, &layout::from_form(&body)).await?;

    users::audit(
        &s.db,
        Some(user.id),
        "section_display_defaults_updated",
        "section_display_defaults",
        None,
    )
    .await?;
    flash(
        &session,
        "Default section display saved. New sections will start from it.",
    )
    .await?;
    Ok(Redirect::to(LIST).into_response())
}