//! The self-service profile page.
//!
//! A teacher, the Principal or a member of office staff signs in and fills in
//! the profile that appears on `/about/staff`: a display name, designation,
//! qualification, contact address, a short biography and an optional photo.
//!
//! A teacher's profile *is* their `faculty` row — the same record the timetable
//! and courses use and the IT admin manages — so this page edits it in place,
//! touching only the fields a teacher owns. The Principal and office staff have
//! no `faculty` row, so their profile is kept in `staff_profiles`, keyed by
//! their account. Both paths then render through the one staff page.

use askama::Template;
use axum::{
    extract::{Multipart, State},
    response::{IntoResponse, Redirect},
};
use tower_sessions::Session;

use crate::{
    auth::{csrf, AuthUser, Role},
    error::AppError,
    services::{content_admin, faculty, profiles, users},
    shell::{flash, Shell},
    state::AppState,
    uploads,
};

const PATH: &str = "/profile";

/// Profile photos live in their own folder under the upload directory.
const FOLDER: &str = "staff-profiles";

#[derive(Template)]
#[template(path = "profile/edit.html")]
pub struct ProfilePage {
    shell: Shell,
    /// True for a teacher, so the page can say where the card is published.
    is_faculty: bool,
    name: String,
    designation: String,
    qualification: String,
    email: String,
    bio: String,
    photo: Option<String>,
    error: Option<String>,
}

/// Whether this role keeps a public profile on the staff page.
fn has_profile(role: Role) -> bool {
    matches!(role, Role::Faculty | Role::Principal | Role::Staff)
}

/// The photo path a caller's profile currently points at (empty when none).
async fn current_photo(s: &AppState, user: &AuthUser) -> Result<String, AppError> {
    let photo = if user.role == Role::Faculty {
        faculty::photo_for_user(&s.db, user.id).await?
    } else {
        profiles::photo_for_user(&s.db, user.id).await?
    };
    Ok(photo.unwrap_or_default())
}

/// Fold the stored values into the form, defaulting the display name for a user
/// who has never filled a profile in.
async fn page_for(s: &AppState, user: &AuthUser, session: &Session) -> Result<ProfilePage, AppError> {
    // The account email starts the contact field, so a first-time profile does
    // not begin blank. `AuthUser` does not carry it, so it is read here.
    let account_email = users::email(&s.db, user.id).await?.unwrap_or_default();
    let (name, designation, qualification, email, bio, photo) = if user.role == Role::Faculty {
        match faculty::profile_for_user(&s.db, user.id).await? {
            Some(p) => (
                p.name,
                p.designation,
                p.qualification,
                p.email.unwrap_or_default(),
                p.bio,
                p.photo_path,
            ),
            None => (
                user.full_name.clone(),
                String::new(),
                String::new(),
                account_email,
                String::new(),
                None,
            ),
        }
    } else {
        match profiles::for_user(&s.db, user.id).await? {
            Some(p) => (
                p.display_name,
                p.designation,
                p.qualification,
                p.email.unwrap_or_default(),
                p.bio,
                p.photo_path,
            ),
            None => (
                user.full_name.clone(),
                String::new(),
                String::new(),
                account_email,
                String::new(),
                None,
            ),
        }
    };

    Ok(ProfilePage {
        is_faculty: user.role == Role::Faculty,
        name,
        designation,
        qualification,
        email,
        bio,
        photo,
        error: None,
        shell: Shell::build(user, session, &s.db).await?,
    })
}

pub async fn show(
    State(s): State<AppState>,
    session: Session,
    user: AuthUser,
) -> Result<ProfilePage, AppError> {
    if !has_profile(user.role) {
        return Err(AppError::Forbidden);
    }
    page_for(&s, &user, &session).await
}

/// Save the posted profile. The photo follows the same rule as every other
/// upload: an empty file input keeps the current picture, and the only way to
/// take one off is the explicit "remove" box.
pub async fn save(
    State(s): State<AppState>,
    session: Session,
    user: AuthUser,
    multipart: Multipart,
) -> Result<axum::response::Response, AppError> {
    if !has_profile(user.role) {
        return Err(AppError::Forbidden);
    }
    let body = uploads::read(multipart).await?;
    csrf::verify(&session, body.field("csrf_token")).await?;

    let previous = current_photo(&s, &user).await?;
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
                // A replacement was just stored but the caller asked for no
                // photo at all, so it must not be left on disk.
                content_admin::discard_upload(&s.db, &s.upload_dir, &path).await;
            }
            path
        }
        Err(AppError::BadRequest(msg)) => {
            flash(&session, &msg).await?;
            return Ok(Redirect::to(PATH).into_response());
        }
        Err(e) => return Err(e),
    };
    let photo = if removing { String::new() } else { photo };

    let name = body.field("name").trim();
    if name.is_empty() {
        // Nothing points at a replacement yet, so do not leave it behind.
        if photo != previous {
            content_admin::discard_upload(&s.db, &s.upload_dir, &photo).await;
        }
        flash(&session, "Give your profile a name.").await?;
        return Ok(Redirect::to(PATH).into_response());
    }

    if user.role == Role::Faculty {
        let saved = faculty::save_profile(
            &s.db,
            user.id,
            name,
            body.field("designation"),
            body.field("qualification"),
            body.field("email"),
            body.field("bio"),
            &photo,
        )
        .await?;
        // A teacher account with no `faculty` row keeps its profile in the same
        // table as the Principal and office staff, so it still reaches `/about/staff`.
        if !saved {
            profiles::save(
                &s.db,
                user.id,
                name,
                body.field("designation"),
                body.field("qualification"),
                body.field("email"),
                body.field("bio"),
                &photo,
            )
            .await?;
        }
    } else {
        profiles::save(
            &s.db,
            user.id,
            name,
            body.field("designation"),
            body.field("qualification"),
            body.field("email"),
            body.field("bio"),
            &photo,
        )
        .await?;
    }

    // Only once the row points at the new photo is it safe to drop the old one.
    if photo != previous {
        content_admin::discard_upload(&s.db, &s.upload_dir, &previous).await;
    }
    users::audit(&s.db, Some(user.id), "profile_updated", "staff_profile", Some(user.id)).await?;
    flash(&session, "Profile saved. It is now on the staff page.").await?;
    Ok(Redirect::to(PATH).into_response())
}
