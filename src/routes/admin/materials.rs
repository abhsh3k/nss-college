//! Study materials: the files a head of department shares with the students of
//! a course.
//!
//! Every handler takes `Manager`, so the IT admin manages every course and an
//! HOD only the courses owned by their own department. The course id that
//! arrives in the form is never trusted on its own: the owning department is
//! looked up server-side and compared with the manager's scope.

use askama::Template;
use axum::{
    extract::{Form, Multipart, Path, State},
    response::{IntoResponse, Redirect, Response},
};
use tower_sessions::Session;

use super::TokenForm;
use crate::{
    auth::{csrf, Manager},
    error::AppError,
    sections,
    services::{content_admin, materials, users},
    shell::{flash, Shell},
    state::AppState,
    uploads,
};

const FOLDER: &str = "materials";

/// The materials list in the caller's own workspace: `/admin/materials` for the
/// IT administrator, `/hod/materials` for a head of department.
fn list(manager: &Manager) -> String {
    format!("{}/materials", sections::base_for(&manager.user))
}

#[derive(Template)]
#[template(path = "admin/materials.html")]
pub struct MaterialsTemplate {
    shell: Shell,
    materials: Vec<materials::MaterialRow>,
    courses: Vec<materials::CourseChoice>,
    error: Option<String>,
}

async fn render_index(
    s: &AppState,
    user: &crate::auth::AuthUser,
    session: &Session,
    department: Option<i64>,
    error: Option<String>,
) -> Result<Response, AppError> {
    Ok(MaterialsTemplate {
        shell: Shell::build(user, session, &s.db).await?,
        materials: materials::for_manager(&s.db, department).await?,
        courses: materials::courses_for_manager(&s.db, department).await?,
        error,
    }
    .into_response())
}

pub async fn index(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
) -> Result<Response, AppError> {
    render_index(&s, &manager.user, &session, manager.department(), None).await
}

pub async fn create(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let body = uploads::read(multipart).await?;
    csrf::verify(&session, body.field("csrf_token")).await?;

    let title = body.field("title").trim().to_string();
    let the_course = super::parse_i64(body.field("course_id"));
    let status = content_admin::clean_status(body.field("status")).to_string();

    let scope = manager.department();
    let Some(course_id) = the_course else {
        return render_index(&s, &manager.user, &session, scope, Some("Choose a course.".into())).await;
    };
    // Re-check the course against the manager's scope, whatever the form sent.
    if !materials::scope_allows(scope, materials::course_department(&s.db, course_id).await?) {
        return Err(AppError::Forbidden);
    }
    if title.is_empty() {
        return render_index(&s, &manager.user, &session, scope, Some("Give the material a title.".into())).await;
    }
    if !crate::validate::fits(&title, crate::validate::MAX_TITLE) {
        let msg = crate::validate::too_long_message("The title", crate::validate::MAX_TITLE);
        return render_index(&s, &manager.user, &session, scope, Some(msg)).await;
    }
    let Some(file) = body.file("file") else {
        return render_index(&s, &manager.user, &session, scope, Some("Choose a file to upload.".into())).await;
    };

    // Write the file only once the metadata is known good, then point a row at
    // it. A rejected upload never leaves bytes behind.
    let path = uploads::save(&s.db, &s.upload_dir, FOLDER, file, false, Some(manager.user.id)).await?;
    let id = materials::create(
        &s.db,
        course_id,
        &title,
        &path,
        manager.user.id,
        &status,
    )
    .await?;
    users::audit(&s.db, Some(manager.user.id), "material_uploaded", "study_material", Some(id)).await?;
    flash(&session, "Study material uploaded.").await?;
    Ok(Redirect::to(&list(&manager)).into_response())
}

pub async fn set_status(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let Some(owner) = materials::material_department(&s.db, id).await? else {
        return Err(AppError::NotFound);
    };
    if !materials::scope_allows(manager.department(), owner) {
        return Err(AppError::Forbidden);
    }
    let status = content_admin::clean_status(&f.status);
    materials::set_status(&s.db, id, status).await?;
    users::audit(
        &s.db,
        Some(manager.user.id),
        if status == "published" { "material_published" } else { "material_archived" },
        "study_material",
        Some(id),
    )
    .await?;
    flash(&session, "Study material updated.").await?;
    Ok(Redirect::to(&list(&manager)))
}

pub async fn destroy(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let Some(owner) = materials::material_department(&s.db, id).await? else {
        return Err(AppError::NotFound);
    };
    if !materials::scope_allows(manager.department(), owner) {
        return Err(AppError::Forbidden);
    }
    let path = materials::file_path(&s.db, id).await?;
    materials::delete(&s.db, id).await?;
    content_admin::discard_upload(&s.db, &s.upload_dir, path.as_deref().unwrap_or("")).await;
    users::audit(&s.db, Some(manager.user.id), "material_deleted", "study_material", Some(id)).await?;
    flash(&session, "Study material deleted.").await?;
    Ok(Redirect::to(&list(&manager)))
}
