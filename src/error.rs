use askama::Template;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::{models::SiteInfo, site};

/// The error pages have no database handle of their own, so their copy comes
/// from the settings cache rather than a query per failure.
macro_rules! error_template {
    ($name:ident, $path:literal) => {
        #[derive(Template)]
        #[template(path = $path)]
        pub struct $name {
            site: SiteInfo,
        }

        impl $name {
            pub fn new() -> Self {
                Self {
                    site: site::cached(),
                }
            }
        }
    };
}

error_template!(NotFoundTemplate, "public/not_found.html");
error_template!(ForbiddenTemplate, "public/forbidden.html");
error_template!(ServerErrorTemplate, "public/server_error.html");
error_template!(BadRequestTemplate, "public/bad_request.html");

#[derive(Debug)]
pub enum AppError {
    NotFound,
    Forbidden,
    /// The form itself was invalid (bad file type, oversized upload, ...).
    BadRequest(String),
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
            AppError::NotFound => (StatusCode::NOT_FOUND, NotFoundTemplate::new()).into_response(),
            AppError::Forbidden => (StatusCode::FORBIDDEN, ForbiddenTemplate::new()).into_response(),
            AppError::BadRequest(msg) => {
                tracing::info!(message = %msg, "rejected an invalid form");
                (StatusCode::BAD_REQUEST, BadRequestTemplate::new()).into_response()
            }
            AppError::Db(e) => {
                tracing::error!(error = ?e, "database error");
                (StatusCode::INTERNAL_SERVER_ERROR, ServerErrorTemplate::new()).into_response()
            }
            AppError::Internal(msg) => {
                tracing::error!(error = %msg, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, ServerErrorTemplate::new()).into_response()
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
