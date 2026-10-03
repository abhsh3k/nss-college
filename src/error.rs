use askama::Template;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[derive(Template)]
#[template(path = "public/not_found.html")]
pub struct NotFoundTemplate;

#[derive(Template)]
#[template(path = "public/forbidden.html")]
pub struct ForbiddenTemplate;

#[derive(Template)]
#[template(path = "public/server_error.html")]
pub struct ServerErrorTemplate;

#[derive(Debug)]
pub enum AppError {
    NotFound,
    Forbidden,
    Db(sqlx::Error),
    Internal(String),
}

/// Convert any displayable error (sessions, hashing, joins) into a 500.
pub fn internal<E: std::fmt::Display>(e: E) -> AppError {
    AppError::Internal(e.to_string())
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
            AppError::Forbidden => (StatusCode::FORBIDDEN, ForbiddenTemplate).into_response(),
            AppError::Db(e) => {
                tracing::error!(error = ?e, "database error");
                (StatusCode::INTERNAL_SERVER_ERROR, ServerErrorTemplate).into_response()
            }
            AppError::Internal(msg) => {
                tracing::error!(error = %msg, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, ServerErrorTemplate).into_response()
            }
        }
    }
}

/// True when an INSERT/UPDATE broke a UNIQUE constraint (duplicate email, admission number, ...).
pub fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .and_then(|d| d.code())
        .map(|c| c == "23505")
        .unwrap_or(false)
}

/// True when a DELETE was blocked because other rows still refer to it.
pub fn is_foreign_key_violation(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .and_then(|d| d.code())
        .map(|c| c == "23503")
        .unwrap_or(false)
}
