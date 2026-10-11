//! The appointments screen: appoint or end a head of department, and grant or
//! revoke a delegation. `AdminOnly` — this is an IT administrator action.
//!
//! The academic appointment is recorded in `role_assignments`/`delegations`,
//! which is what `Manager` reads. The legacy `faculty` booleans are updated in
//! the same transaction so the sidebar stays in step.

use askama::Template;
use axum::{
    extract::{Form, Path, State},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use tower_sessions::Session;

use super::{is_valid_date, parse_i64, TokenForm};
use crate::{
    auth::{csrf, AdminOnly},
    error::{is_unique_violation, AppError},
    services::{appointments, users},
    shell::{flash, Shell},
    state::AppState,
};

const LIST: &str = "/admin/appointments";

#[derive(Template)]
#[template(path = "admin/appointments.html")]
pub struct AppointmentsTemplate {
    shell: Shell,
    heads: Vec<appointments::HeadRow>,
    delegations: Vec<appointments::DelegationRow>,
    faculty: Vec<appointments::FacultyOption>,
    error: Option<String>,
}

async fn render(
    s: &AppState,
    user: &crate::auth::AuthUser,
    session: &Session,
    error: Option<String>,
) -> Result<Response, AppError> {
    Ok(AppointmentsTemplate {
        shell: Shell::build(user, session).await?,
        heads: appointments::list_heads(&s.db).await?,
        delegations: appointments::list_delegations(&s.db).await?,
        faculty: appointments::faculty_options(&s.db).await?,
        error,
    }
    .into_response())
}

pub async fn index(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
) -> Result<Response, AppError> {
    render(&s, &user, &session, None).await
}

#[derive(Deserialize)]
pub struct GrantForm {
    csrf_token: String,
    user_id: String,
    #[serde(default)]
    acting: Option<String>,
    #[serde(default)]
    ends_at: String,
    #[serde(default)]
    reason: String,
}

pub async fn grant(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Form(f): Form<GrantForm>,
) -> Result<Response, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;

    let Some(target) = parse_i64(&f.user_id) else {
        return render(&s, &user, &session, Some("Choose a teacher.".into())).await;
    };
    let Some((Some(department), ..)) = appointments::faculty_flags(&s.db, target).await? else {
        return render(&s, &user, &session, Some("That teacher has no department to head.".into()))
            .await;
    };
    if !f.ends_at.trim().is_empty() && !is_valid_date(&f.ends_at) {
        return render(&s, &user, &session, Some("Give an end date as YYYY-MM-DD, or leave it blank.".into()))
            .await;
    }
    let ends = f.ends_at.trim();
    let acting = f.acting.is_some();

    match appointments::grant_head(
        &s.db,
        target,
        department,
        acting,
        (!ends.is_empty()).then_some(ends),
        user.id,
        f.reason.trim(),
    )
    .await
    {
        Ok(id) => {
            users::audit(&s.db, Some(user.id), "appointment_granted", "role_assignment", Some(id))
                .await?;
            flash(&session, "Appointment recorded. The teacher now heads that department.").await?;
            Ok(Redirect::to(LIST).into_response())
        }
        Err(e) if is_unique_violation(&e) => {
            render(
                &s,
                &user,
                &session,
                Some("That department already has an active head. End that appointment first.".into()),
            )
            .await
        }
        Err(e) => Err(AppError::from(e)),
    }
}

pub async fn revoke(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    appointments::revoke_head(&s.db, id, user.id).await?;
    users::audit(&s.db, Some(user.id), "appointment_revoked", "role_assignment", Some(id)).await?;
    flash(&session, "Appointment ended. A handover task was opened for the successor.").await?;
    Ok(Redirect::to(LIST))
}

#[derive(Deserialize)]
pub struct DelegateForm {
    csrf_token: String,
    user_id: String,
    ends_at: String,
}

pub async fn delegate(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Form(f): Form<DelegateForm>,
) -> Result<Response, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;

    let Some(target) = parse_i64(&f.user_id) else {
        return render(&s, &user, &session, Some("Choose a teacher.".into())).await;
    };
    let Some((Some(department), ..)) = appointments::faculty_flags(&s.db, target).await? else {
        return render(&s, &user, &session, Some("That teacher has no department.".into())).await;
    };
    if !is_valid_date(&f.ends_at) {
        return render(&s, &user, &session, Some("Give an end date as YYYY-MM-DD.".into())).await;
    }

    match appointments::grant_delegation(
        &s.db,
        target,
        department,
        "manage_department",
        f.ends_at.trim(),
        user.id,
    )
    .await
    {
        Ok(id) => {
            users::audit(&s.db, Some(user.id), "delegation_granted", "delegation", Some(id)).await?;
            flash(&session, "Delegation granted until that date.").await?;
            Ok(Redirect::to(LIST).into_response())
        }
        Err(sqlx::Error::RowNotFound) => {
            render(
                &s,
                &user,
                &session,
                Some("That department has no active head to delegate from.".into()),
            )
            .await
        }
        Err(e) => Err(AppError::from(e)),
    }
}

pub async fn revoke_delegation(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    appointments::revoke_delegation(&s.db, id, user.id).await?;
    users::audit(&s.db, Some(user.id), "delegation_revoked", "delegation", Some(id)).await?;
    flash(&session, "Delegation revoked.").await?;
    Ok(Redirect::to(LIST))
}
