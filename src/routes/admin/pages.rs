//! Pages: the informational pages served by path (`/about`, `/iqac`, ...),
//! plus the sections that make up each page's body.

use askama::Template;
use axum::{
    extract::{Multipart, Path, Query, State},
    Form,
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use tower_sessions::Session;

use super::{statuses, TokenForm};
use crate::{
    auth::{csrf, AdminOnly},
    error::AppError,
    layout::{self, DisplayOptions},
    services::{
        content_admin,
        site_admin::{self, PageInput, SectionRow},
        users,
    },
    shell::{flash, Shell},
    state::AppState,
    uploads::{self, ParsedForm},
};

const LIST: &str = "/admin/pages";

/// Page section photos live in their own folder.
const FOLDER: &str = "sections";

/// A one-line message shown on the list page after an inline section action.
fn back_to(page_id: i64) -> String {
    format!("/admin/pages/{page_id}")
}

// ---------- List page ----------

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/pages.html")]
pub struct IndexPage {
    shell: Shell,
    pages: Vec<site_admin::PageRow>,
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
        pages: site_admin::pages(&s.db, &filter).await?,
        filter,
        statuses: statuses(),
    })
}

// ---------- Add and edit form ----------

pub struct PageFormValues {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub lede: String,
    pub status: String,
    /// Which header menu the page sits in (see `site_admin::NAV_GROUPS`).
    pub nav_group: String,
    /// The words the menu shows; empty means "use the title".
    pub nav_label: String,
    /// Position inside the group, lowest first.
    pub nav_sort: String,
    pub error: Option<String>,
}

impl PageFormValues {
    fn blank() -> PageFormValues {
        PageFormValues {
            id: 0,
            path: String::new(),
            title: String::new(),
            lede: String::new(),
            status: "published".into(),
            nav_group: "none".into(),
            nav_label: String::new(),
            nav_sort: "0".into(),
            error: None,
        }
    }

    fn to_input(&self) -> PageInput {
        PageInput {
            path: self.path.clone(),
            title: self.title.clone(),
            lede: self.lede.clone(),
            status: self.status.clone(),
            nav_group: self.nav_group.clone(),
            nav_label: self.nav_label.clone(),
            nav_sort: site_admin::clean_nav_sort(&self.nav_sort),
        }
    }

    /// Fold a stored row into the form, for both the edit screen and the re-read
    /// after a failed save.
    fn from_edit(row: site_admin::PageEdit) -> PageFormValues {
        PageFormValues {
            id: row.id,
            path: row.path,
            title: row.title,
            lede: row.lede,
            status: row.status,
            nav_group: row.nav_group,
            nav_label: row.nav_label,
            nav_sort: row.nav_sort.to_string(),
            error: None,
        }
    }
}

#[derive(Template)]
#[template(path = "admin/page_form.html")]
pub struct FormPage {
    shell: Shell,
    form: PageFormValues,
    statuses: Vec<String>,
    /// The choices of the "position" select.
    nav_groups: Vec<NavGroupOpt>,
}

fn form_page(form: PageFormValues, shell: Shell) -> FormPage {
    FormPage {
        shell,
        form,
        statuses: statuses(),
        nav_groups: nav_groups(),
    }
}

/// One option of the "position" select on the page forms: the value stored in
/// `pages.nav_group` and the words an admin reads for it.
pub struct NavGroupOpt {
    pub value: String,
    pub label: &'static str,
}

/// The menu groups with the words an admin reads, ready for a select.
fn nav_groups() -> Vec<NavGroupOpt> {
    site_admin::NAV_GROUPS
        .iter()
        .map(|g| NavGroupOpt {
            value: (*g).to_string(),
            label: site_admin::nav_group_label(g),
        })
        .collect()
}

pub async fn new_form(
    State(_s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
) -> Result<FormPage, AppError> {
    Ok(form_page(PageFormValues::blank(), Shell::build(&user, &session).await?))
}

/// The edit screen: the page's own fields plus every section, each editable in place.
#[derive(Template)]
#[template(path = "admin/page_edit.html")]
pub struct EditPage {
    shell: Shell,
    form: PageFormValues,
    sections: Vec<SectionRow>,
    statuses: Vec<String>,
    /// The choices of the "position" select.
    nav_groups: Vec<NavGroupOpt>,
    /// What a newly added section starts from, so the add form does not open with
    /// blank choices when the admin expects the house style.
    defaults: DisplayOptions,
}

pub async fn edit_form(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
) -> Result<EditPage, AppError> {
    let row = site_admin::page_for_edit(&s.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(EditPage {
        form: PageFormValues::from_edit(row),
        sections: site_admin::sections(&s.db, id).await?,
        defaults: layout::defaults(&s.db).await?,
        statuses: statuses(),
        nav_groups: nav_groups(),
        shell: Shell::build(&user, &session).await?,
    })
}

/// Re-read both halves of the edit screen after an inline section change, so the
/// flash message and the section list are shown together.
async fn edit_page(
    s: &AppState,
    user: &crate::auth::AuthUser,
    session: &Session,
    id: i64,
) -> Result<EditPage, AppError> {
    let row = site_admin::page_for_edit(&s.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(EditPage {
        form: PageFormValues::from_edit(row),
        sections: site_admin::sections(&s.db, id).await?,
        defaults: layout::defaults(&s.db).await?,
        statuses: statuses(),
        nav_groups: nav_groups(),
        shell: Shell::build(user, session).await?,
    })
}

/// The submitted page fields, with the path and title checked before any write.
fn read_form(f: &PageFormSubmit, existing_id: i64) -> PageFormValues {
    let mut form = PageFormValues {
        id: existing_id,
        path: f.path.trim().to_string(),
        title: f.title.trim().to_string(),
        lede: f.lede.trim().to_string(),
        status: content_admin::clean_status(&f.status).to_string(),
        nav_group: site_admin::clean_nav_group(&f.nav_group),
        nav_label: f.nav_label.trim().to_string(),
        nav_sort: site_admin::clean_nav_sort(&f.nav_sort).to_string(),
        error: None,
    };
    if form.title.is_empty() {
        form.error = Some("Give the page a title.".into());
    } else if form.path.is_empty() {
        form.error = Some("Give the page a path, for example /about.".into());
    } else if !form.path.starts_with('/') || form.path == "/" || form.path.contains(' ') {
        form.error = Some("A page path starts with a slash and has no spaces.".into());
    }
    form
}

pub async fn create(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Form(f): Form<PageFormSubmit>,
) -> Result<Response, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;

    let values = read_form(&f, 0);
    if values.error.is_some() {
        return Ok(form_page(values, Shell::build(&user, &session).await?).into_response());
    }

    let id = match site_admin::create_page(&s.db, &values.to_input()).await {
        Ok(id) => id,
        Err(sqlx::Error::Protocol(msg)) => {
            let mut values = values;
            values.error = Some(msg);
            return Ok(form_page(values, Shell::build(&user, &session).await?).into_response());
        }
        Err(e) => return Err(e.into()),
    };
    users::audit(&s.db, Some(user.id), "page_created", "page", Some(id)).await?;
    flash(&session, "Page saved. Add its sections below.").await?;
    Ok(Redirect::to(&back_to(id)).into_response())
}

#[derive(Deserialize)]
pub struct PageFormSubmit {
    csrf_token: String,
    path: String,
    title: String,
    #[serde(default)]
    lede: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    nav_group: String,
    #[serde(default)]
    nav_label: String,
    #[serde(default)]
    nav_sort: String,
}

pub async fn update(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
    Form(f): Form<PageFormSubmit>,
) -> Result<Response, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;

    let values = read_form(&f, id);
    if values.error.is_some() {
        return Ok(edit_page(&s, &user, &session, id).await?.into_response());
    }

    if let Err(e) = site_admin::update_page(&s.db, id, &values.to_input()).await {
        if let sqlx::Error::Database(db) = &e {
            if db.code().as_deref() == Some("23505") {
                let mut page = edit_page(&s, &user, &session, id).await?;
                page.form.error = Some("Another page already uses that path.".into());
                return Ok(page.into_response());
            }
        }
        return Err(e.into());
    }
    users::audit(&s.db, Some(user.id), "page_updated", "page", Some(id)).await?;
    flash(&session, "Page updated.").await?;
    Ok(Redirect::to(&back_to(id)).into_response())
}

// ---------- Sections ----------

/// Store a posted section photo, or fall back to the one the row already owns.
///
/// An empty file input must not blank out an existing photo, so `keep` is what
/// the row has today. Ticking "remove" is the only way to take one off. A
/// rejected upload becomes a form message rather than a failed request.
async fn store_photo(
    s: &AppState,
    body: &ParsedForm,
    keep: &str,
    user_id: i64,
) -> Result<Result<String, String>, AppError> {
    match uploads::save_optional(
        &s.db,
        &s.upload_dir,
        FOLDER,
        body,
        "photo",
        keep,
        true,
        Some(user_id),
    )
    .await
    {
        Ok(path) => {
            if body.flag("remove_photo") {
                // A replacement was just stored but the admin asked for no photo
                // at all, so it must not be left on disk.
                content_admin::discard_upload(&s.db, &s.upload_dir, &path).await;
                return Ok(Ok(String::new()));
            }
            Ok(Ok(path))
        }
        Err(AppError::BadRequest(msg)) => Ok(Err(msg)),
        Err(e) => Err(e),
    }
}

pub async fn add_section(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
    multipart: Multipart,
) -> Result<Redirect, AppError> {
    let body = uploads::read(multipart).await?;
    csrf::verify(&session, body.field("csrf_token")).await?;
    if site_admin::page_for_edit(&s.db, id).await?.is_none() {
        return Err(AppError::NotFound);
    }

    let heading = body.field("heading");
    let photo = match store_photo(&s, &body, "", user.id).await? {
        Ok(path) => path,
        Err(msg) => {
            // Nothing points at the photo yet, so do not leave it on disk.
            content_admin::discard_upload(&s.db, &s.upload_dir, "").await;
            flash(&session, &msg).await?;
            return Ok(Redirect::to(&back_to(id)));
        }
    };

    if heading.is_empty() {
        flash(&session, "Give the section a heading.").await?;
        return Ok(Redirect::to(&back_to(id)));
    }

    let opts = layout::from_form(&body);
    site_admin::create_section(
        &s.db,
        id,
        heading,
        body.field("body"),
        &photo,
        body.field("photo_caption"),
        body.flag("published"),
        &opts,
    )
    .await?;
    users::audit(&s.db, Some(user.id), "page_section_added", "page", Some(id)).await?;
    flash(&session, "Section added.").await?;
    Ok(Redirect::to(&back_to(id)))
}

pub async fn update_section(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path((page_id, section_id)): Path<(i64, i64)>,
    multipart: Multipart,
) -> Result<Redirect, AppError> {
    let body = uploads::read(multipart).await?;
    csrf::verify(&session, body.field("csrf_token")).await?;
    if site_admin::page_for_edit(&s.db, page_id).await?.is_none() {
        return Err(AppError::NotFound);
    }

    let previous = site_admin::section_photo(&s.db, section_id)
        .await?
        .unwrap_or_default();
    let photo = match store_photo(&s, &body, &previous, user.id).await? {
        Ok(path) => path,
        Err(msg) => {
            flash(&session, &msg).await?;
            return Ok(Redirect::to(&back_to(page_id)));
        }
    };

    let heading = body.field("heading");
    if heading.is_empty() {
        // The row still points at the old photo, so a replacement stored above
        // must not be left behind.
        if photo != previous {
            content_admin::discard_upload(&s.db, &s.upload_dir, &photo).await;
        }
        flash(&session, "Give the section a heading.").await?;
        return Ok(Redirect::to(&back_to(page_id)));
    }

    let opts = layout::from_form(&body);
    site_admin::update_section(
        &s.db,
        section_id,
        heading,
        body.field("body"),
        &photo,
        body.field("photo_caption"),
        body.flag("published"),
        &opts,
    )
    .await?;

    // Only once the row points at the new photo is it safe to drop the old one.
    if photo != previous {
        content_admin::discard_upload(&s.db, &s.upload_dir, &previous).await;
    }
    users::audit(&s.db, Some(user.id), "page_section_updated", "page", Some(page_id)).await?;
    flash(&session, "Section updated.").await?;
    Ok(Redirect::to(&back_to(page_id)))
}

pub async fn delete_section(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path((page_id, section_id)): Path<(i64, i64)>,
    Form(f): Form<TokenForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    if site_admin::page_for_edit(&s.db, page_id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    site_admin::delete_section(&s.db, section_id).await?;
    users::audit(&s.db, Some(user.id), "page_section_deleted", "page", Some(page_id)).await?;
    flash(&session, "Section removed.").await?;
    Ok(Redirect::to(&back_to(page_id)))
}

#[derive(Deserialize)]
pub struct MoveSubmit {
    csrf_token: String,
    #[serde(default)]
    up: String,
}

pub async fn move_section(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path((page_id, section_id)): Path<(i64, i64)>,
    Form(f): Form<MoveSubmit>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    if site_admin::page_for_edit(&s.db, page_id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    // The button carries "up" when ticked, so its presence means move up.
    site_admin::move_section(&s.db, section_id, !f.up.is_empty()).await?;
    users::audit(&s.db, Some(user.id), "page_section_moved", "page", Some(page_id)).await?;
    Ok(Redirect::to(&back_to(page_id)))
}

// ---------- Archive and delete ----------

pub async fn set_status(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let back = super::safe_back(&f.back, LIST);
    if site_admin::page_for_edit(&s.db, id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    let status = content_admin::clean_status(&f.status);
    site_admin::set_page_status(&s.db, id, status).await?;
    users::audit(&s.db, Some(user.id), "page_status_changed", "page", Some(id)).await?;
    flash(
        &session,
        if status == "draft" {
            "Page unpublished. Visitors now get 404 for it."
        } else {
            "Page published."
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
    if site_admin::page_for_edit(&s.db, id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    // The page's sections go with it, through the foreign key.
    site_admin::delete_page(&s.db, id).await?;
    users::audit(&s.db, Some(user.id), "page_deleted", "page", Some(id)).await?;
    flash(&session, "Page and its sections permanently deleted.").await?;
    Ok(Redirect::to(&back))
}