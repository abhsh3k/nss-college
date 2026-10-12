pub mod csrf;
pub mod password;

use axum::{
    extract::{FromRequestParts, Request},
    http::{request::Parts, StatusCode},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use tower_sessions::Session;

use crate::{error::{internal, AppError}, services::users, state::AppState};

pub const SESSION_USER: &str = "user_id";
/// The role recorded at sign-in, so the listener boundary can be judged without
/// a database round-trip. It is only a fast path: every handler still resolves
/// the caller through [`AuthUser`], and a role change revokes the session.
pub const SESSION_ROLE: &str = "role";
pub const SESSION_VERSION: &str = "session_version";
/// When the user last re-entered their password, as a Unix timestamp.
pub const SESSION_REAUTH: &str = "reauth_at";
/// How long a password re-entry stays good for a critical operation.
pub const REAUTH_WINDOW_SECS: i64 = 15 * 60;

/// A fresh password check, judged on its timestamp alone.
///
/// A re-entry counts for [`REAUTH_WINDOW_SECS`]; a timestamp in the future (a
/// clock skew or a tampered value) never counts.
pub fn reauth_valid(at: Option<i64>, now: i64) -> bool {
    matches!(at, Some(ts) if ts <= now && now - ts <= REAUTH_WINDOW_SECS)
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Whether the session holds a currently-valid password re-entry.
pub async fn reauth_recent(session: &Session) -> bool {
    let at: Option<i64> = session.get(SESSION_REAUTH).await.unwrap_or(None);
    reauth_valid(at, now_unix())
}

/// Redirect target for a critical action when the password check has lapsed.
pub fn reauth_url(next: &str) -> String {
    if safe_next(next).is_some() {
        format!("/account/reauth?next={next}")
    } else {
        "/account/reauth".to_string()
    }
}

/// Gate a critical action on a recent password check. `Ok(())` when the session
/// is fresh; otherwise `Err` is the redirect the handler should return.
pub async fn require_reauth(session: &Session, next: &str) -> Result<(), Redirect> {
    if reauth_recent(session).await {
        Ok(())
    } else {
        Err(Redirect::to(&reauth_url(next)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Root authority. It carries every IT-admin power *and* the role/recovery
    /// powers behind `SuperAdminOnly`, and it is only reachable on the
    /// loopback-only management listener.
    SuperAdmin,
    Admin,
    /// Principal / Academic Admin: institution-wide academic authority, but no
    /// technical administration privileges.
    Principal,
    Staff,
    Faculty,
    Student,
    Alumni,
}

impl Role {
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "super_admin" => Some(Role::SuperAdmin),
            "admin" => Some(Role::Admin),
            "principal" => Some(Role::Principal),
            "staff" => Some(Role::Staff),
            "faculty" => Some(Role::Faculty),
            "student" => Some(Role::Student),
            "alumni" => Some(Role::Alumni),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Role::SuperAdmin => "Super Admin",
            Role::Admin => "IT administrator",
            Role::Principal => "Principal",
            Role::Staff => "Office staff",
            Role::Faculty => "Teacher",
            Role::Student => "Student",
            Role::Alumni => "Alumni",
        }
    }

    /// Whether this role may reach the account-role administration screen.
    pub fn is_super_admin(&self) -> bool {
        matches!(self, Role::SuperAdmin)
    }

    /// Where this role lands after signing in.
    pub fn home(&self) -> &'static str {
        match self {
            Role::SuperAdmin | Role::Admin | Role::Principal => "/admin",
            // Office staff work in their own workspace, on both listeners.
            Role::Staff => crate::sections::STAFF_BASE,
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
    /// Heads their department.
    pub is_hod: bool,
    /// Approved by the IT admin to act for the HOD.
    pub can_manage: bool,
}

impl AuthUser {
    /// Whether this teacher may use the department tools.
    pub fn manages_department(&self) -> bool {
        self.is_hod || self.can_manage
    }
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
            Ok(Some(row)) => {
                // Sessions carry the user's revocation version at sign-in. Any
                // mismatch (including a session that predates the field) means
                // the session was revoked by a password reset, deactivation, or
                // role change, so force a fresh sign-in.
                let session_version: Option<i64> = session
                    .get(SESSION_VERSION)
                    .await
                    .map_err(|e| internal(e).into_response())?;
                if session_version != Some(row.session_version) {
                    let _ = session.flush().await;
                    return Err(to_login(parts));
                }
                match Role::parse(&row.role) {
                    Some(role) => Ok(AuthUser {
                        id: row.id,
                        full_name: row.full_name,
                        role,
                        must_change_password: row.must_change_password,
                        is_hod: row.is_hod,
                        can_manage: row.can_manage,
                    }),
                    None => Err(AppError::Forbidden.into_response()),
                }
            }
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

// IT administrator only (users, academics, timetable, settings). The Super
// Admin is a superset of the IT admin, so it is allowed wherever the IT admin
// is — the difference is the extra `SuperAdminOnly` powers below, not fewer.
#[allow(dead_code)]
mod guards {
    use super::*;
    role_guard!(AdminOnly, &[Role::Admin, Role::SuperAdmin]);
    // The Principal signs off academic appointments, so they share the
    // appointments screen with the IT admin.
    role_guard!(AdminOrPrincipal, &[Role::Admin, Role::SuperAdmin, Role::Principal]);
    // The landing page at `/admin`: everyone who works in the management area,
    // including the Principal (whose home it is). Content authoring stays on
    // `OfficeOrAdmin`, so the Principal gets no publishing powers here.
    role_guard!(AdminDashboard, &[Role::Admin, Role::SuperAdmin, Role::Principal, Role::Staff]);
    role_guard!(PrincipalOnly, &[Role::Principal]);
    role_guard!(OfficeOrAdmin, &[Role::Admin, Role::SuperAdmin, Role::Staff]);
    // The office-staff workspace at `/staff`; the IT admin reaches the same
    // content tools under `/admin` through `OfficeOrAdmin`.
    role_guard!(StaffOnly, &[Role::Staff]);
    role_guard!(TeacherOnly, &[Role::Faculty]);
    role_guard!(StudentOnly, &[Role::Student]);
    // Root authority: account roles, emergency recovery, and the audited
    // reopening of finalized academic records. No other role passes this.
    role_guard!(SuperAdminOnly, &[Role::SuperAdmin]);
}
pub use guards::{
    AdminDashboard, AdminOnly, AdminOrPrincipal, OfficeOrAdmin, PrincipalOnly, StaffOnly,
    StudentOnly, SuperAdminOnly, TeacherOnly,
};

/// How far across the college a manager may reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// IT admin: every department.
    All,
    /// Head of (or approved for) one department. `None` when the teacher has
    /// no department set yet, which grants nothing to manage.
    Department(Option<i64>),
}

/// A user allowed to manage academic records.
///
/// This is the IT admin, or a teacher who heads their department, or a teacher
/// the IT admin has approved to act for the HOD. Everyone else is refused.
///
/// Unlike the role guards this one is not decided by `users.role` alone: the
/// HOD powers live on the `faculty` row, so it is read per request.
pub struct Manager {
    pub user: AuthUser,
    pub scope: Scope,
}

impl Manager {
    /// The department this manager is confined to; `None` for the IT admin.
    pub fn department(&self) -> Option<i64> {
        match self.scope {
            Scope::All => None,
            Scope::Department(id) => id,
        }
    }

    /// True for the IT admin, who is not confined to one department.
    pub fn is_admin(&self) -> bool {
        matches!(self.scope, Scope::All)
    }
}

#[axum::async_trait]
impl FromRequestParts<AppState> for Manager {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Ready(user) = Ready::from_request_parts(parts, state).await?;

        // The IT admin keeps every power and is not confined to a department;
        // the Super Admin, being a superset, likewise. The Principal has
        // institution-wide *academic* authority — the same cross-department
        // scope — but not the technical powers, which stay behind `AdminOnly`.
        if matches!(user.role, Role::Admin | Role::SuperAdmin | Role::Principal) {
            return Ok(Manager {
                user,
                scope: Scope::All,
            });
        }

        if user.role != Role::Faculty {
            return Err(AppError::Forbidden.into_response());
        }

        // Authority is decided by the appointment/delegation model, which keeps
        // the legacy faculty flags in step for accounts created before it and
        // respects effective dates (an expired appointment denies).
        let department_id =
            crate::services::appointments::effective_department(&state.db, user.id)
                .await
                .map_err(|e| AppError::from(e).into_response())?;

        // No effective appointment, or one without a department: nothing to
        // manage. A missing department must deny, never mean "all departments".
        let Some(department_id) = department_id else {
            return Err(AppError::Forbidden.into_response());
        };

        Ok(Manager {
            user,
            scope: Scope::Department(Some(department_id)),
        })
    }
}

/// Refuse root authority (and teachers) on the public listener's management
/// area.
///
/// The Super Admin is loopback-only: root authority must not be reachable from
/// the network, so on the public listener its management requests are refused
/// here. A head of department works in `/hod` (mounted on both listeners), so
/// the `/admin` form of the management area is not offered to a `faculty`
/// account on the public listener either. The IT administrator, the Principal
/// and office staff may use either listener. This layer is mounted only on the
/// public listener's management mount, so the boundary is a property of the
/// listener rather than of a navigation link.
pub async fn management_area_on_public(request: Request, next: Next) -> Response {
    let (mut parts, body) = request.into_parts();
    if let Ok(session) = Session::from_request_parts(&mut parts, &()).await {
        let role: Option<String> = session.get(SESSION_ROLE).await.unwrap_or(None);
        if let Some(role) = role {
            let allowed = matches!(
                Role::parse(&role),
                Some(Role::Admin | Role::Principal | Role::Staff)
            );
            if !allowed {
                return AppError::Forbidden.into_response();
            }
        }
    }
    next.run(Request::from_parts(parts, body)).await
}

/// Accept only same-site relative paths for the post-login redirect.
pub fn safe_next(next: &str) -> Option<&str> {
    if next.starts_with('/') && !next.starts_with("//") && !next.contains('\\') && !next.contains("://") {
        Some(next)
    } else {
        None
    }
}

pub fn safe_next_for_role(next: &str, role: Role) -> Option<&str> {
    let next = safe_next(next)?;
    let allowed = match role {
        Role::SuperAdmin | Role::Admin | Role::Principal => {
            next.starts_with("/admin") || next.starts_with("/account/")
        }
        // Office staff work in `/staff`; `/admin` is kept so a staff member
        // bounced to the sign-in page from there is not sent somewhere else.
        Role::Staff => {
            next.starts_with(crate::sections::STAFF_BASE)
                || next.starts_with("/admin/")
                || next.starts_with("/account/")
        }
        // A teacher's own pages, plus the department workspace an appointed head
        // of department works in (`/hod`). `/admin` is kept so an HOD who was
        // bounced to the sign-in page from there is not sent somewhere else.
        Role::Faculty => {
            next.starts_with("/teacher")
                || next.starts_with("/hod/")
                || next.starts_with("/admin/")
                || next.starts_with("/account/")
        }
        Role::Student => next.starts_with("/hub") || next.starts_with("/account/"),
        Role::Alumni => next == "/" || next.starts_with("/account/"),
    };
    allowed.then_some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_login_redirect_is_limited_to_the_role_workspace() {
        assert_eq!(safe_next_for_role("/hub/results", Role::Student), Some("/hub/results"));
        assert_eq!(safe_next_for_role("/admin", Role::Student), None);
        assert_eq!(safe_next_for_role("/admin/people", Role::Admin), Some("/admin/people"));
        assert_eq!(safe_next_for_role("//evil.example", Role::Admin), None);
        // A head of department signs in on the public listener: their
        // department workspace is `/hod`, and a student may not be sent there.
        assert_eq!(safe_next_for_role("/hod/marks", Role::Faculty), Some("/hod/marks"));
        assert_eq!(safe_next_for_role("/hod/marks", Role::Student), None);
    }

    #[test]
    fn a_reentry_only_counts_within_the_window() {
        let now = 1_000_000;
        assert!(reauth_valid(Some(now), now));
        assert!(reauth_valid(Some(now - REAUTH_WINDOW_SECS + 1), now));
        assert!(!reauth_valid(Some(now - REAUTH_WINDOW_SECS - 1), now));
        assert!(!reauth_valid(None, now));
        // A timestamp in the future is not a fresh check.
        assert!(!reauth_valid(Some(now + 10), now));
    }

    #[test]
    fn the_super_admin_is_the_root_role_on_the_management_listener() {
        assert_eq!(Role::parse("super_admin"), Some(Role::SuperAdmin));
        assert_eq!(Role::SuperAdmin.label(), "Super Admin");
        // Its home is the management area, which is only served on loopback.
        assert_eq!(Role::SuperAdmin.home(), "/admin");
        assert!(Role::SuperAdmin.is_super_admin());
        assert!(!Role::Admin.is_super_admin());
        assert_eq!(
            safe_next_for_role("/admin/roles", Role::SuperAdmin),
            Some("/admin/roles")
        );
    }

    #[test]
    fn office_staff_land_in_their_own_workspace() {
        assert_eq!(Role::Staff.home(), "/staff");
        assert_eq!(
            safe_next_for_role("/staff/notices", Role::Staff),
            Some("/staff/notices")
        );
        // A staff member is not sent into a student's or teacher's pages.
        assert_eq!(safe_next_for_role("/hub", Role::Staff), None);
        assert_eq!(safe_next_for_role("/teacher", Role::Staff), None);
    }

    #[test]
    fn the_principal_is_a_distinct_role_with_an_academic_workspace() {
        assert_eq!(Role::parse("principal"), Some(Role::Principal));
        assert_eq!(Role::Principal.home(), "/admin");
        assert_eq!(Role::Principal.label(), "Principal");
        // The Principal may reach the academic management area, not a student's.
        assert_eq!(
            safe_next_for_role("/admin/appointments", Role::Principal),
            Some("/admin/appointments")
        );
        assert_eq!(safe_next_for_role("/hub", Role::Principal), None);
    }
}
