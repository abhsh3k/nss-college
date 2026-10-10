//! The section index page behind a dashboard card: `/admin/manage/:key`.
//!
//! It renders one [`Area`] from the catalogue in `crate::sections` — a title,
//! a description and the links to the pages that already implement each
//! operation. The page itself grants nothing: it only picks a list to draw,
//! and that list is chosen by the same rules that guard the links inside it.

use askama::Template;
use axum::extract::Path;
use tower_sessions::Session;

use crate::{
    auth::{AuthUser, Ready, Role},
    error::AppError,
    sections::{self, Area},
    shell::Shell,
};

#[derive(Template)]
#[template(path = "admin/manage.html")]
pub struct ManageTemplate {
    shell: Shell,
    area: &'static Area,
    /// Every area this caller may reach, for the "other areas" strip.
    siblings: &'static [Area],
}

/// The catalogue that matches what this user is already allowed to open.
///
/// This mirrors — it does not replace — the guards on the target pages:
///
/// * IT administrator: everything (each target is `AdminOnly` or `Manager`),
/// * office staff: content only (their targets are `OfficeOrAdmin`),
/// * a head of department or an approved delegate: department-scoped tools
///   (their targets take `Manager`, which confines every query to
///   `faculty.department_id`),
/// * anyone else: refused, exactly as the target pages refuse them.
fn catalogue(user: &AuthUser) -> Option<&'static [Area]> {
    match user.role {
        Role::Admin => Some(sections::ADMIN),
        Role::Staff => Some(sections::STAFF),
        Role::Faculty if user.manages_department() => Some(sections::HOD),
        _ => None,
    }
}

/// Which area, if any, this caller may open — and with what error if not.
///
/// * role with no catalogue at all → `403` (the same answer the target pages
///   would give),
/// * a slug that belongs to another role's catalogue → `403`, so a boundary is
///   reported as a boundary rather than as a missing page,
/// * anything else → `404`.
fn lookup(user: &AuthUser, key: &str) -> Result<(&'static Area, &'static [Area]), AppError> {
    let all = catalogue(user).ok_or(AppError::Forbidden)?;
    match sections::find(all, key) {
        Some(area) => Ok((area, all)),
        None if sections::exists_any(key) => Err(AppError::Forbidden),
        None => Err(AppError::NotFound),
    }
}

pub async fn page(
    session: Session,
    Ready(user): Ready,
    Path(key): Path<String>,
) -> Result<ManageTemplate, AppError> {
    let (area, siblings) = lookup(&user, &key)?;

    Ok(ManageTemplate {
        shell: Shell::build(&user, &session).await?,
        area,
        siblings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_section_page_lists_its_operations_and_the_other_areas() {
        let area = sections::find(sections::ADMIN, "exams").expect("the exams area");
        let html = ManageTemplate {
            shell: crate::sections::test_shell("IT administrator"),
            area,
            siblings: sections::ADMIN,
        }
        .render()
        .expect("the section page renders");

        assert!(html.contains("Examination &amp; Results"));
        assert!(html.contains("/admin/exams"));
        assert!(html.contains("/admin/marks"));
        assert!(html.contains("Other management areas"));
    }

    #[test]
    fn an_unknown_area_key_is_a_404() {
        assert!(sections::find(sections::ADMIN, "does-not-exist").is_none());
        assert!(matches!(
            lookup(&user(Role::Admin, false), "does-not-exist"),
            Err(AppError::NotFound)
        ));
    }

    /// The three rules `lookup` implements, exercised on real roles.
    #[test]
    fn a_role_gets_its_own_area_and_nothing_else() {
        let admin = user(Role::Admin, false);
        let staff = user(Role::Staff, false);
        let hod = user(Role::Faculty, true);
        let teacher = user(Role::Faculty, false);
        let student = user(Role::Student, false);

        assert!(lookup(&admin, "users").is_ok());
        assert!(lookup(&staff, "website").is_ok());
        assert!(lookup(&hod, "students").is_ok());

        // A section that exists, but for somebody else: forbidden, not missing.
        assert!(matches!(lookup(&staff, "academic"), Err(AppError::Forbidden)));
        assert!(matches!(lookup(&hod, "users"), Err(AppError::Forbidden)));
        assert!(matches!(lookup(&hod, "settings"), Err(AppError::Forbidden)));
        assert!(matches!(lookup(&teacher, "students"), Err(AppError::Forbidden)));

        // Roles with no catalogue at all are refused outright, and a typo is a
        // 404 for everyone.
        assert!(matches!(lookup(&student, "users"), Err(AppError::Forbidden)));
        assert!(matches!(lookup(&admin, "nope"), Err(AppError::NotFound)));
        assert!(matches!(lookup(&hod, "nope"), Err(AppError::NotFound)));
    }

    fn user(role: Role, manages: bool) -> AuthUser {
        AuthUser {
            id: 1,
            full_name: "Test User".into(),
            role,
            must_change_password: false,
            is_hod: manages,
            can_manage: false,
        }
    }
}
