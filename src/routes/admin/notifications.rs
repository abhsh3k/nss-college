//! The in-app notification feed.
//!
//! Any signed-in user may read their own feed. The feed only ever points at the
//! pages that own each decision, and following a link re-authorises there, so a
//! notification is a nudge, not a grant of access.

use askama::Template;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
    Form,
};
use tower_sessions::Session;

use super::TokenForm;
use crate::{
    auth::{csrf, Ready},
    error::AppError,
    services::notifications,
    shell::Shell,
    state::AppState,
};

const FEED: &str = "/admin/notifications";

#[derive(Template)]
#[template(path = "admin/notifications.html")]
pub struct NotificationsTemplate {
    shell: Shell,
    items: Vec<notifications::Notification>,
    unread: i64,
}

pub async fn page(
    State(s): State<AppState>,
    session: Session,
    Ready(user): Ready,
) -> Result<NotificationsTemplate, AppError> {
    Ok(NotificationsTemplate {
        shell: Shell::build(&user, &session).await?,
        items: notifications::for_user(&s.db, user.id, 100).await?,
        unread: notifications::unread_count(&s.db, user.id).await?,
    })
}

pub async fn mark_read(
    State(s): State<AppState>,
    session: Session,
    Ready(user): Ready,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Response, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    notifications::mark_read(&s.db, id, user.id).await?;
    Ok(Redirect::to(FEED).into_response())
}

pub async fn mark_all(
    State(s): State<AppState>,
    session: Session,
    Ready(user): Ready,
    Form(f): Form<TokenForm>,
) -> Result<Response, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    notifications::mark_all_read(&s.db, user.id).await?;
    Ok(Redirect::to(FEED).into_response())
}
