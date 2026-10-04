use askama::Template;
use axum::{
    extract::{Query, State},
    response::{IntoResponse, Redirect, Response},
    Form,
};
use serde::Deserialize;
use tower_sessions::Session;

use crate::{
    auth::{csrf, password, safe_next, AuthUser, SESSION_USER},
    error::{internal, AppError},
    services::users,
    shell::Shell,
    state::AppState,
};

// ---------- Sign in ----------

#[derive(Template)]
#[template(path = "auth/login.html")]
pub struct LoginTemplate {
    site: crate::models::SiteInfo,
    csrf_token: String,
    next: String,
    identifier: String,
    error: Option<&'static str>,
}

#[derive(Deserialize)]
pub struct LoginQuery {
    next: Option<String>,
}

pub async fn login_form(
    State(s): State<AppState>,
    session: Session,
    user: Option<AuthUser>,
    Query(q): Query<LoginQuery>,
) -> Result<Response, AppError> {
    if let Some(user) = user {
        return Ok(Redirect::to(user.role.home()).into_response());
    }
    let next = q.next.unwrap_or_default();
    Ok(LoginTemplate {
        site: crate::site::get(&s.db).await?,
        csrf_token: csrf::token(&session).await?,
        next,
        identifier: String::new(),
        error: None,
    }
    .into_response())
}

#[derive(Deserialize)]
pub struct LoginForm {
    identifier: String,
    password: String,
    csrf_token: String,
    next: Option<String>,
}

const BAD_LOGIN: &str = "That email, admission number or password is not right.";
const LOCKED: &str = "Too many wrong attempts. Try again in 15 minutes, or ask the IT admin to help.";

pub async fn login_submit(
    State(s): State<AppState>,
    session: Session,
    Form(f): Form<LoginForm>,
) -> Result<Response, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;

    let identifier = f.identifier.trim().to_string();
    let next = f.next.clone().unwrap_or_default();

    let again = |error: &'static str, token: String| {
        LoginTemplate {
            site: crate::site::cached(),
            csrf_token: token,
            next: next.clone(),
            identifier: identifier.clone(),
            error: Some(error),
        }
        .into_response()
    };

    let row = users::find_for_login(&s.db, &identifier).await?;
    let Some(row) = row else {
        password::verify_dummy(f.password).await?;
        return Ok(again(BAD_LOGIN, csrf::token(&session).await?));
    };

    if !row.is_active {
        password::verify_dummy(f.password).await?;
        return Ok(again(BAD_LOGIN, csrf::token(&session).await?));
    }
    if row.locked {
        return Ok(again(LOCKED, csrf::token(&session).await?));
    }

    let ok = password::verify_blocking(f.password, row.password_hash).await?;
    if !ok {
        users::record_failed_login(&s.db, row.id).await?;
        tracing::warn!(user_id = row.id, "failed sign-in");
        return Ok(again(BAD_LOGIN, csrf::token(&session).await?));
    }

    users::record_login(&s.db, row.id).await?;
    // New session id on sign-in prevents session fixation.
    session.cycle_id().await.map_err(internal)?;
    csrf::rotate(&session).await?;
    session.insert(SESSION_USER, row.id).await.map_err(internal)?;
    tracing::info!(user_id = row.id, "signed in");

    let user = users::find_active(&s.db, row.id).await?.ok_or(AppError::Forbidden)?;
    let home = crate::auth::Role::parse(&user.role)
        .map(|r| r.home())
        .unwrap_or("/");
    let target = if user.must_change_password {
        "/account/password"
    } else {
        safe_next(&next).unwrap_or(home)
    };
    Ok(Redirect::to(target).into_response())
}

// ---------- Sign out ----------

#[derive(Deserialize)]
pub struct LogoutForm {
    csrf_token: String,
}

pub async fn logout(session: Session, Form(f): Form<LogoutForm>) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    session.flush().await.map_err(internal)?;
    Ok(Redirect::to("/login"))
}

// ---------- Change password ----------

#[derive(Template)]
#[template(path = "account/password.html")]
pub struct PasswordTemplate {
    shell: Shell,
    forced: bool,
    error: Option<String>,
}

pub async fn password_form(user: AuthUser, session: Session) -> Result<PasswordTemplate, AppError> {
    Ok(PasswordTemplate {
        shell: Shell::build(&user, &session).await?,
        forced: user.must_change_password,
        error: None,
    })
}

#[derive(Deserialize)]
pub struct PasswordForm {
    current_password: String,
    new_password: String,
    confirm_password: String,
    csrf_token: String,
}

pub async fn password_submit(
    State(s): State<AppState>,
    user: AuthUser,
    session: Session,
    Form(f): Form<PasswordForm>,
) -> Result<Response, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;

    let problem = if f.new_password != f.confirm_password {
        Some("The two new passwords do not match.".to_string())
    } else if f.new_password.chars().count() < password::MIN_LENGTH {
        Some(format!("Use at least {} characters.", password::MIN_LENGTH))
    } else if f.new_password == f.current_password {
        Some("Choose a password you have not used here before.".to_string())
    } else {
        let stored = users::password_hash(&s.db, user.id).await?;
        if password::verify_blocking(f.current_password.clone(), stored).await? {
            None
        } else {
            Some("Your current password is not right.".to_string())
        }
    };

    if let Some(error) = problem {
        return Ok(PasswordTemplate {
            shell: Shell::build(&user, &session).await?,
            forced: user.must_change_password,
            error: Some(error),
        }
        .into_response());
    }

    let new_hash = password::hash_blocking(f.new_password).await?;
    users::set_password(&s.db, user.id, &new_hash).await?;
    users::audit(&s.db, Some(user.id), "password_changed", "user", Some(user.id)).await?;
    session.cycle_id().await.map_err(internal)?;
    Ok(Redirect::to(user.role.home()).into_response())
}
