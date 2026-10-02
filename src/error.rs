use askama::Template;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[derive(Template)]
#[template(path = "public/not_found.html")]
pub struct NotFoundTemplate;

#[derive(Template)]
#[template(path = "public/server_error.html")]
pub struct ServerErrorTemplate;

#[derive(Debug)]
pub enum AppError {
    NotFound,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => AppError::NotFound,
            other => AppError::Db(other),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::NotFound => (StatusCode::NOT_FOUND, NotFoundTemplate).into_response(),
            AppError::Db(e) => {
                tracing::error!(error = ?e, "database error");
                (StatusCode::INTERNAL_SERVER_ERROR, ServerErrorTemplate).into_response()
            }
        }
    }
}
