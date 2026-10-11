//! Root authority on the loopback-only management listener.
//!
//! Every handler here takes `SuperAdminOnly`, so no IT admin, Principal or
//! office staff account can reach it — the account roles, the emergency
//! recovery of a locked-out account, the Principal appointment, and the
//! audited reopening of a finalized academic record all belong to root.
//!
//! Nothing here is navigation-only: each action is authorised again in the
//! service layer, and each one writes an audit row, with a structured reason
//! where the operator gave one.

use askama::Template;
use axum::{
    extract::{Form, Path, State},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use tower_sessions::Session;

use super::{is_valid_date, parse_i64};
use crate::{
    auth::{csrf, require_reauth, SuperAdminOnly},
    error::{is_unique_violation, AppError},
    services::{notifications, super_admin, users},
    shell::{flash, Shell},
    state::AppState,
};

const SCREEN: &str = "/admin/roles";

#[derive(Template)]
#[template(path = "admin/roles.html")]
pub struct RolesTemplate {
    shell: Shell,
    accounts: Vec<super_admin::AccountRow>,
    principal: Option<super_admin::PrincipalRow>,
    candidates: Vec<super_admin::PrincipalCandidate>,
    finalized: Vec<super_admin::FinalizedCourseRow>,
    /// The roles the account table may set.
    roles: Vec<RoleOption>,
    error: Option<String>,
}

pub struct RoleOption {
    pub value: &'static str,
    pub label: &'static str,
}

fn technical_roles() -> Vec<RoleOption> {
    super_admin::TECHNICAL_ROLES
        .iter()
        .map(|r| RoleOption {
            value: r,
            label: super_admin::role_label(r),
        })
        .collect()
}

async fn render(
    s: &AppState,
    user: &crate::auth::AuthUser,
    session: &Session,
    error: Option<String>,
) -> Result<Response, AppError> {
    Ok(RolesTemplate {
        shell: Shell::build(user, session).await?,
        accounts: super_admin::list_accounts(&s.db).await?,
        principal: super_admin::current_principal(&s.db).await?,
        candidates: super_admin::principal_candidates(&s.db).await?,
        finalized: super_admin::list_finalized_courses(&s.db).await?,
        roles: technical_roles(),
        error,
    }
    .into_response())
}

pub async fn page(
    State(s): State<AppState>,
    session: Session,
    SuperAdminOnly(user): SuperAdminOnly,
) -> Result<Response, AppError> {
    render(&s, &user, &session, None).await
}

/// A gateway shared by every write on the screen: CSRF, then a recent password
/// re-entry, because each of these changes who can do what.
async fn guard(session: &Session, token: &str) -> Result<Option<Redirect>, AppError> {
    csrf::verify(session, token).await?;
    match require_reauth(session, SCREEN).await {
        Ok(()) => Ok(None),
        Err(redirect) => {
            flash(session, "Re-enter your password to continue.").await?;
            Ok(Some(redirect))
        }
    }
}

#[derive(Deserialize)]
pub struct RoleForm {
    csrf_token: String,
    role: String,
}

pub async fn set_role(
    State(s): State<AppState>,
    session: Session,
    SuperAdminOnly(user): SuperAdminOnly,
    Path(id): Path<i64>,
    Form(f): Form<RoleForm>,
) -> Result<Response, AppError> {
    if let Some(redirect) = guard(&session, &f.csrf_token).await? {
        return Ok(redirect.into_response());
    }
    match super_admin::set_role(&s.db, id, f.role.trim()).await? {
        super_admin::RoleChange::Done => {
            users::audit_detail(
                &s.db,
                Some(user.id),
                "user_role_changed",
                "user",
                Some(id),
                serde_json::json!({ "role": f.role.trim() }),
            )
            .await?;
            flash(&session, "Role updated; any session that account held is now revoked.").await?;
            Ok(Redirect::to(SCREEN).into_response())
        }
        super_admin::RoleChange::NotFound => {
            render(&s, &user, &session, Some("That account no longer exists.".into())).await
        }
        super_admin::RoleChange::UnknownRole => {
            render(
                &s,
                &user,
                &session,
                Some("Only the Super Admin and IT administrator roles are set here.".into()),
            )
            .await
        }
        super_admin::RoleChange::LastSuperAdmin => {
            render(
                &s,
                &user,
                &session,
                Some("This is the last active Super Admin; promote another account first.".into()),
            )
            .await
        }
        super_admin::RoleChange::StillPrincipal => {
            render(
                &s,
                &user,
                &session,
                Some("That account is the Principal. End the Principal appointment instead.".into()),
            )
            .await
        }
    }
}

/// Emergency recovery: clear a lockout so an account can sign in again.
pub async fn unlock(
    State(s): State<AppState>,
    session: Session,
    SuperAdminOnly(user): SuperAdminOnly,
    Path(id): Path<i64>,
    Form(f): Form<super::TokenForm>,
) -> Result<Response, AppError> {
    if let Some(redirect) = guard(&session, &f.csrf_token).await? {
        return Ok(redirect.into_response());
    }
    let unlocked = super_admin::unlock_account(&s.db, id).await?;
    users::audit_detail(
        &s.db,
        Some(user.id),
        "account_unlocked",
        "user",
        Some(id),
        serde_json::json!({ "unlocked": unlocked }),
    )
    .await?;
    flash(
        &session,
        if unlocked {
            "Lockout cleared; the account can sign in again."
        } else {
            "That account was not locked out."
        },
    )
    .await?;
    Ok(Redirect::to(SCREEN).into_response())
}

#[derive(Deserialize)]
pub struct PrincipalForm {
    csrf_token: String,
    #[serde(default)]
    user_id: String,
    #[serde(default)]
    ends_at: String,
    #[serde(default)]
    reason: String,
}

pub async fn appoint_principal(
    State(s): State<AppState>,
    session: Session,
    SuperAdminOnly(user): SuperAdminOnly,
    Form(f): Form<PrincipalForm>,
) -> Result<Response, AppError> {
    if let Some(redirect) = guard(&session, &f.csrf_token).await? {
        return Ok(redirect.into_response());
    }
    let Some(target) = parse_i64(&f.user_id) else {
        return render(&s, &user, &session, Some("Choose an account to appoint.".into())).await;
    };
    if target == user.id {
        return render(
            &s,
            &user,
            &session,
            Some("Root authority cannot also hold the Principal office; choose another account.".into()),
        )
        .await;
    }
    let ends = f.ends_at.trim();
    if !ends.is_empty() && !is_valid_date(ends) {
        return render(&s, &user, &session, Some("Give an end date as YYYY-MM-DD, or leave it blank.".into()))
            .await;
    }
    let reason = f.reason.trim();
    if reason.is_empty() {
        return render(&s, &user, &session, Some("Give a reason for the appointment.".into())).await;
    }
    if super_admin::current_principal(&s.db).await?.is_some() {
        // Defensive: the unique index refuses a second live appointment anyway.
        return render(
            &s,
            &user,
            &session,
            Some("There is already a Principal in office. End that appointment first.".into()),
        )
        .await;
    }

    match super_admin::appoint_principal(
        &s.db,
        target,
        (!ends.is_empty()).then_some(ends),
        user.id,
        reason,
    )
    .await
    {
        Ok(assignment_id) => {
            users::audit_detail(
                &s.db,
                Some(user.id),
                "principal_appointed",
                "role_assignment",
                Some(assignment_id),
                serde_json::json!({
                    "user_id": target,
                    "ends_at": if ends.is_empty() { serde_json::Value::Null } else { serde_json::json!(ends) },
                    "reason": reason,
                }),
            )
            .await?;
            notifications::create(
                &s.db,
                target,
                "appointment",
                "Your Principal appointment is active",
                "The Super Admin appointed you Principal. The academic workspace is in the sidebar.",
                "/admin",
            )
            .await?;
            flash(&session, "Principal appointed and active.").await?;
            Ok(Redirect::to(SCREEN).into_response())
        }
        Err(e) if is_unique_violation(&e) => {
            render(
                &s,
                &user,
                &session,
                Some("There is already a Principal in office. End that appointment first.".into()),
            )
            .await
        }
        Err(sqlx::Error::Protocol(_)) => {
            render(
                &s,
                &user,
                &session,
                Some("Root authority cannot also hold the Principal office; choose another account.".into()),
            )
            .await
        }
        Err(sqlx::Error::RowNotFound) => {
            render(
                &s,
                &user,
                &session,
                Some("That account cannot take the appointment (inactive, or already root).".into()),
            )
            .await
        }
        Err(e) => Err(AppError::from(e)),
    }
}

pub async fn revoke_principal(
    State(s): State<AppState>,
    session: Session,
    SuperAdminOnly(user): SuperAdminOnly,
    Path(id): Path<i64>,
    Form(f): Form<super::TokenForm>,
) -> Result<Response, AppError> {
    if let Some(redirect) = guard(&session, &f.csrf_token).await? {
        return Ok(redirect.into_response());
    }
    match super_admin::revoke_principal(&s.db, id, user.id).await? {
        Some(user_id) => {
            users::audit_detail(
                &s.db,
                Some(user.id),
                "principal_revoked",
                "role_assignment",
                Some(id),
                serde_json::json!({ "user_id": user_id }),
            )
            .await?;
            flash(&session, "Principal appointment ended; the account is back to office staff.").await?;
        }
        None => {
            flash(&session, "That appointment was no longer active.").await?;
        }
    }
    Ok(Redirect::to(SCREEN).into_response())
}

#[derive(Deserialize)]
pub struct ReopenForm {
    csrf_token: String,
    #[serde(default)]
    course: String,
    #[serde(default)]
    reason: String,
}

/// The Super Admin's emergency intervention: reopen one finalized course.
/// Every locked row is preserved in `mark_history` before the live row is
/// returned to `reviewed`, so the value that was in force is never lost.
pub async fn reopen_finalized(
    State(s): State<AppState>,
    session: Session,
    SuperAdminOnly(user): SuperAdminOnly,
    Form(f): Form<ReopenForm>,
) -> Result<Response, AppError> {
    if let Some(redirect) = guard(&session, &f.csrf_token).await? {
        return Ok(redirect.into_response());
    }
    let parts: Vec<&str> = f.course.trim().split(':').collect();
    let target = match parts.as_slice() {
        [programme, semester, course] => (
            parse_i64(programme),
            semester.parse::<i32>().ok(),
            parse_i64(course),
        ),
        _ => (None, None, None),
    };
    let (Some(programme_id), Some(semester), Some(course_id)) = target else {
        return render(&s, &user, &session, Some("Choose a finalized course to reopen.".into())).await;
    };
    let reason = f.reason.trim();
    if reason.len() < 5 {
        return render(
            &s,
            &user,
            &session,
            Some("Give a reason of at least five characters for reopening an official record.".into()),
        )
        .await;
    }

    let reopened = super_admin::reopen_finalized(
        &s.db,
        programme_id,
        semester,
        course_id,
        user.id,
        reason,
    )
    .await?;
    users::audit_detail(
        &s.db,
        Some(user.id),
        "finalized_marks_reopened",
        "course",
        Some(course_id),
        serde_json::json!({
            "programme_id": programme_id,
            "semester": semester,
            "rows": reopened,
            "reason": reason,
        }),
    )
    .await?;
    flash(
        &session,
        &format!("Reopened {reopened} finalized mark(s); the old values are in the mark history."),
    )
    .await?;
    Ok(Redirect::to(SCREEN).into_response())
}
