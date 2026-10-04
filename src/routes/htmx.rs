use askama::Template;
use axum::{
    extract::State,
    http::{header, HeaderValue},
    response::IntoResponse,
};

use crate::{
    auth::{AuthUser, Role},
    error::AppError,
    models::Notice,
    services::content,
    state::AppState,
};

#[derive(Template)]
#[template(path = "partials/notice_list.html")]
pub struct NoticeListTemplate {
    notices: Vec<Notice>,
}

/// Small fragment polled by the notice board (`hx-trigger="every 60s"`).
/// Layer 5 adds ETag / 304 handling so unchanged polls cost almost nothing.
pub async fn notices(State(s): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let body = NoticeListTemplate {
        notices: content::notices(&s.db, 8).await?,
    };
    Ok((
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"))],
        body,
    ))
}

// ---------- The navbar's sign-in / dashboard button ----------

#[derive(Template)]
#[template(path = "partials/account_link.html")]
pub struct AccountLinkTemplate {
    href: &'static str,
    label: &'static str,
}

/// The one navbar button that points somewhere different depending on who is
/// browsing: the sign-in link when signed out, the Student Hub for a student,
/// and the holder's own dashboard for a teacher or member of staff.
pub async fn account_link(user: Option<AuthUser>) -> Result<impl IntoResponse, AppError> {
    let (href, label) = match user {
        // Students only ever get the Student Hub.
        Some(u) if u.role == Role::Student => ("/hub", "Student Hub"),
        Some(u) if u.role == Role::Alumni => ("/", "Home"),
        Some(u) => (u.role.home(), "Dashboard"),
        None => ("/login", "Log in"),
    };
    Ok((
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"))],
        AccountLinkTemplate { href, label },
    ))
}
