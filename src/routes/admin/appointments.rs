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
    auth::{csrf, require_reauth, AdminOnly, AdminOrPrincipal, PrincipalOnly, Role},
    error::{is_unique_violation, AppError},
    services::{appointments, notifications, users},
    shell::{flash, Shell},
    state::AppState,
};

const LIST: &str = "/admin/appointments";

#[derive(Template)]
#[template(path = "admin/appointments.html")]
pub struct AppointmentsTemplate {
    shell: Shell,
    heads: Vec<appointments::HeadRow>,
    pending: Vec<appointments::PendingHeadRow>,
    delegations: Vec<appointments::DelegationRow>,
    faculty: Vec<appointments::FacultyOption>,
    /// The Principal approves requests; the IT admin initiates them.
    is_principal: bool,
    error: Option<String>,
}

async fn render(
    s: &AppState,
    user: &crate::auth::AuthUser,
    session: &Session,
    error: Option<String>,
) -> Result<Response, AppError> {
    Ok(AppointmentsTemplate {
        shell: Shell::build(user, session, &s.db).await?,
        heads: appointments::list_heads(&s.db).await?,
        pending: appointments::list_pending_heads(&s.db).await?,
        delegations: appointments::list_delegations(&s.db).await?,
        faculty: appointments::faculty_options(&s.db).await?,
        is_principal: user.role == Role::Principal,
        error,
    }
    .into_response())
}

pub async fn index(
    State(s): State<AppState>,
    session: Session,
    user: AdminOrPrincipal,
) -> Result<Response, AppError> {
    render(&s, &user.0, &session, None).await
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
    if let Err(redirect) = require_reauth(&session, LIST).await {
        flash(&session, "Re-enter your password to continue.").await?;
        return Ok(redirect.into_response());
    }

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

    match appointments::request_head(
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
            users::audit_detail(
                &s.db,
                Some(user.id),
                "appointment_requested",
                "role_assignment",
                Some(id),
                serde_json::json!({
                    "department_id": department,
                    "acting": acting,
                    "ends_at": if ends.is_empty() { serde_json::Value::Null } else { serde_json::json!(ends) },
                    "reason": f.reason.trim(),
                }),
            )
            .await?;
            let kind = if acting { "acting head" } else { "head" };
            notifications::notify_roles(
                &s.db,
                &["principal", "admin"],
                "appointment",
                "Head appointment request",
                &format!("A request to appoint the {kind} of a department needs a decision."),
                "/admin/appointments",
            )
            .await?;
            flash(
                &session,
                "Appointment requested. The Principal must approve it before it takes effect.",
            )
            .await?;
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

/// The Principal approves a pending head appointment, activating it.
pub async fn approve(
    State(s): State<AppState>,
    session: Session,
    PrincipalOnly(user): PrincipalOnly,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Response, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    if let Err(redirect) = require_reauth(&session, LIST).await {
        flash(&session, "Re-enter your password to continue.").await?;
        return Ok(redirect.into_response());
    }
    match appointments::approve_head(&s.db, id, user.id).await {
        Ok(Some(appointed)) => {
            users::audit_detail(
                &s.db,
                Some(user.id),
                "appointment_approved",
                "role_assignment",
                Some(id),
                serde_json::json!({ "appointed_user_id": appointed }),
            )
            .await?;
            notifications::create(
                &s.db,
                appointed,
                "appointment",
                "Your head appointment was approved",
                "It is now active; department tools are available from the sidebar.",
                "/admin/work-queue",
            )
            .await?;
            flash(&session, "Appointment approved and now active.").await?;
            Ok(Redirect::to(LIST).into_response())
        }
        Ok(None) => {
            flash(&session, "That request is no longer pending.").await?;
            Ok(Redirect::to(LIST).into_response())
        }
        Err(e) if is_unique_violation(&e) => render(
            &s,
            &user,
            &session,
            Some("That department already has an active head.".into()),
        )
        .await,
        Err(sqlx::Error::Protocol(_)) => {
            flash(&session, "You cannot approve your own appointment.").await?;
            Ok(Redirect::to(LIST).into_response())
        }
        Err(e) => Err(AppError::from(e)),
    }
}

/// The Principal rejects a pending head appointment; it never becomes active.
pub async fn reject(
    State(s): State<AppState>,
    session: Session,
    PrincipalOnly(user): PrincipalOnly,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Response, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    if let Err(redirect) = require_reauth(&session, LIST).await {
        flash(&session, "Re-enter your password to continue.").await?;
        return Ok(redirect.into_response());
    }
    if appointments::reject_head(&s.db, id, user.id).await? {
        users::audit_detail(
            &s.db,
            Some(user.id),
            "appointment_rejected",
            "role_assignment",
            Some(id),
            serde_json::json!({ "rejected": true }),
        )
        .await?;
        flash(&session, "Appointment request rejected.").await?;
    } else {
        flash(&session, "That request is no longer pending.").await?;
    }
    Ok(Redirect::to(LIST).into_response())
}

pub async fn revoke(
    State(s): State<AppState>,
    session: Session,
    user: AdminOrPrincipal,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Redirect, AppError> {
    let user = user.0;
    csrf::verify(&session, &f.csrf_token).await?;
    if let Err(redirect) = require_reauth(&session, LIST).await {
        flash(&session, "Re-enter your password to continue.").await?;
        return Ok(redirect);
    }
    appointments::revoke_head(&s.db, id, user.id).await?;
    users::audit_detail(
        &s.db,
        Some(user.id),
        "appointment_revoked",
        "role_assignment",
        Some(id),
        // Ending a headship has a side effect (the handover task); record it as
        // part of the action rather than leaving it implicit.
        serde_json::json!({ "handover_task_opened": true }),
    )
    .await?;
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
    if let Err(redirect) = require_reauth(&session, LIST).await {
        flash(&session, "Re-enter your password to continue.").await?;
        return Ok(redirect.into_response());
    }

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
            users::audit_detail(
                &s.db,
                Some(user.id),
                "delegation_granted",
                "delegation",
                Some(id),
                serde_json::json!({
                    "delegate_user_id": target,
                    "department_id": department,
                    "permission": "manage_department",
                    "ends_at": f.ends_at.trim(),
                }),
            )
            .await?;
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
    if let Err(redirect) = require_reauth(&session, LIST).await {
        flash(&session, "Re-enter your password to continue.").await?;
        return Ok(redirect);
    }
    appointments::revoke_delegation(&s.db, id, user.id).await?;
    users::audit_detail(
        &s.db,
        Some(user.id),
        "delegation_revoked",
        "delegation",
        Some(id),
        serde_json::json!({ "revoked": true }),
    )
    .await?;
    flash(&session, "Delegation revoked.").await?;
    Ok(Redirect::to(LIST))
}
