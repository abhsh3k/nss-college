pub mod csrf;
pub mod password;

use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use tower_sessions::Session;

use crate::{error::{internal, AppError}, services::users, state::AppState};

pub const SESSION_USER: &str = "user_id";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Admin,
    Staff,
    Faculty,
    Student,
    Alumni,
}

impl Role {
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "admin" => Some(Role::Admin),
            "staff" => Some(Role::Staff),
            "faculty" => Some(Role::Faculty),
            "student" => Some(Role::Student),
            "alumni" => Some(Role::Alumni),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Role::Admin => "IT administrator",
            Role::Staff => "Office staff",
            Role::Faculty => "Teacher",
            Role::Student => "Student",
            Role::Alumni => "Alumni",
        }
    }

    /// Where this role lands after signing in.
    pub fn home(&self) -> &'static str {
        match self {
            Role::Admin | Role::Staff => "/admin",
            Role::Faculty => "/teacher",
            Role::Student => "/hub",
            Role::Alumni => "/",
        }
    }
}

#[derive(Clone, Debug)]
pub struct AuthUser {
    pub id: i64,
    pub full_name: String,
    pub role: Role,
    pub must_change_password: bool,
}

/// Send the browser to the sign-in page. HTMX requests need a header instead of a 303.
fn to_login(parts: &Parts) -> Response {
    let path = parts.uri.path();
    let plain = path.len() > 1
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_'));
    let target = if plain {
        format!("/login?next={path}")
    } else {
        "/login".to_string()
    };
    if parts.headers.contains_key("hx-request") {
        return ([("HX-Redirect", target)], StatusCode::OK).into_response();
    }
    Redirect::to(&target).into_response()
}

#[axum::async_trait]
impl FromRequestParts<AppState> for AuthUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let session = Session::from_request_parts(parts, state)
            .await
            .map_err(|e| e.into_response())?;

        let user_id: Option<i64> = session
            .get(SESSION_USER)
            .await
            .map_err(|e| internal(e).into_response())?;
        let Some(user_id) = user_id else {
            return Err(to_login(parts));
        };

        match users::find_active(&state.db, user_id).await {
            Ok(Some(row)) => match Role::parse(&row.role) {
                Some(role) => Ok(AuthUser {
                    id: row.id,
                    full_name: row.full_name,
                    role,
                    must_change_password: row.must_change_password,
                }),
                None => Err(AppError::Forbidden.into_response()),
            },
            Ok(None) => {
                // Account removed or deactivated while signed in.
                let _ = session.flush().await;
                Err(to_login(parts))
            }
            Err(e) => Err(AppError::from(e).into_response()),
        }
    }
}

/// A signed-in user who has already replaced their temporary password.
pub struct Ready(pub AuthUser);

#[axum::async_trait]
impl FromRequestParts<AppState> for Ready {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if user.must_change_password {
            if parts.headers.contains_key("hx-request") {
                return Err(([("HX-Redirect", "/account/password")], StatusCode::OK).into_response());
            }
            return Err(Redirect::to("/account/password").into_response());
        }
        Ok(Ready(user))
    }
}

macro_rules! role_guard {
    ($name:ident, $allowed:expr) => {
        pub struct $name(pub AuthUser);

        #[axum::async_trait]
        impl FromRequestParts<AppState> for $name {
            type Rejection = Response;

            async fn from_request_parts(
                parts: &mut Parts,
                state: &AppState,
            ) -> Result<Self, Self::Rejection> {
                let Ready(user) = Ready::from_request_parts(parts, state).await?;
                let allowed: &[Role] = $allowed;
                if allowed.contains(&user.role) {
                    Ok($name(user))
                } else {
                    Err(AppError::Forbidden.into_response())
                }
            }
        }
    };
}

// IT administrator only (users, academics, timetable, settings).
#[allow(dead_code)]
mod guards {
    use super::*;
    role_guard!(AdminOnly, &[Role::Admin]);
    role_guard!(OfficeOrAdmin, &[Role::Admin, Role::Staff]);
    role_guard!(TeacherOnly, &[Role::Faculty]);
    role_guard!(StudentOnly, &[Role::Student]);
}
pub use guards::{AdminOnly, OfficeOrAdmin, StudentOnly, TeacherOnly};

/// Accept only same-site relative paths for the post-login redirect.
pub fn safe_next(next: &str) -> Option<&str> {
    if next.starts_with('/') && !next.starts_with("//") && !next.contains('\\') && !next.contains("://") {
        Some(next)
    } else {
        None
    }
}
