use askama::Template;
use axum::{
    extract::State,
    http::{header, HeaderValue},
    response::IntoResponse,
};

use crate::{error::AppError, models::Notice, services::content, state::AppState};

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
