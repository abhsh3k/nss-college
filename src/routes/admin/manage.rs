//! The section index page behind a dashboard card: `/manage/:key`, mounted as
//! `/admin/manage/:key` for the management workspace and `/hod/manage/:key` for
//! a head of department's own.
//!
//! It renders one [`Area`] from the catalogue in `crate::sections` — a title,
//! a description and the links to the pages that already implement each
//! operation. The page itself grants nothing: it only picks a list to draw,
//! and that list is chosen by the same rules that guard the links inside it.

use askama::Template;
use axum::extract::{Path, State};
use tower_sessions::Session;

use crate::{
    auth::{AuthUser, Ready, Role},
    error::AppError,
    sections::{self, Area},
    shell::Shell,
    state::AppState,
};

#[derive(Template)]
#[template(path = "admin/manage.html")]
pub struct ManageTemplate {
    shell: Shell,
    area: Area,
    /// Every area this caller may reach, for the "other areas" strip.
    siblings: Vec<Area>,
}

/// The catalogue that matches what this user is already allowed to open.
///
/// This mirrors — it does not replace — the guards on the target pages:
///
/// * Super Admin: everything (each target is `AdminOnly`, `AcademicOnly` or
///   `Manager`);
/// * IT administrator: its own [`sections::areas_for`] view — accounts, the CMS,
///   settings, and the two academic tools whose guards admit it (the timetable
///   and the attendance report);
/// * office staff: content only (their targets are `OfficeOrAdmin`),
/// * the Principal, and a head of department or an approved delegate:
///   department-scoped tools (their targets take `Manager`),
/// * anyone else: refused, exactly as the target pages refuse them.
fn catalogue(user: &AuthUser) -> Option<Vec<Area>> {
    if user.role == Role::Faculty && user.manages_department() {
        return Some(sections::HOD.to_vec());
    }
    let areas = sections::areas_for(user.role);
    (!areas.is_empty()).then_some(areas)
}

/// Which area, if any, this caller may open — and with what error if not.
///
/// * role with no catalogue at all → `403` (the same answer the target pages
///   would give),
/// * a slug that belongs to another role's catalogue → `403`, so a boundary is
///   reported as a boundary rather than as a missing page,
/// * anything else → `404`.
fn lookup(user: &AuthUser, key: &str) -> Result<(Area, Vec<Area>), AppError> {
    let all = catalogue(user).ok_or(AppError::Forbidden)?;
    match all.iter().find(|a| a.key == key) {
        Some(area) => Ok((area.clone(), all)),
        None if sections::exists_any(key) => Err(AppError::Forbidden),
        None => Err(AppError::NotFound),
    }
}

pub async fn page(
    State(s): State<AppState>,
    session: Session,
    Ready(user): Ready,
    Path(key): Path<String>,
) -> Result<ManageTemplate, AppError> {
    let (area, siblings) = lookup(&user, &key)?;

    Ok(ManageTemplate {
        shell: Shell::build(&user, &session, &s.db).await?,
        area,
        siblings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_section_page_lists_its_operations_and_the_other_areas() {
        // The Super Admin keeps the academic areas; the IT admin does not.
        let all = sections::areas_for(Role::SuperAdmin);
        let area = all
            .iter()
            .find(|a| a.key == "exams")
            .cloned()
            .expect("the exams area");
        let html = ManageTemplate {
            shell: crate::sections::test_shell("Super Admin"),
            area,
            siblings: all,
        }
        .render()
        .expect("the section page renders");

        assert!(html.contains("Examination &amp; Results"));
        assert!(html.contains("/admin/exams"));
        assert!(html.contains("/admin/marks"));
        assert!(html.contains("Other management areas"));
    }

    /// The IT administrator's two academic cards list only what its own guards
    /// open: the grid and the report, never the substitution or the register
    /// correction that a head of department owns.
    #[test]
    fn the_it_admins_timetable_section_offers_no_substitutions() {
        let all = sections::areas_for(Role::Admin);
        let area = all
            .iter()
            .find(|a| a.key == "timetable")
            .cloned()
            .expect("the IT admin's timetable area");
        let html = ManageTemplate {
            shell: crate::sections::test_shell("IT administrator"),
            area,
            siblings: all,
        }
        .render()
        .expect("the section page renders");

        assert!(html.contains("href=\"/admin/timetable\""));
        assert!(
            !html.contains("/admin/substitutions"),
            "the IT admin was offered the substitution page, which would refuse it"
        );
    }

    /// The same catalogue rendered for a head of department links into the
    /// department workspace, not into `/admin` — which is not served on the
    /// listener they sign in to.
    #[test]
    fn a_section_page_for_an_hod_links_into_the_department_workspace() {
        let all = sections::HOD.to_vec();
        let area = all
            .iter()
            .find(|a| a.key == "exams")
            .cloned()
            .expect("the exams area");
        let html = ManageTemplate {
            shell: crate::sections::test_shell_in("Head of department", sections::HOD_BASE),
            area,
            siblings: all,
        }
        .render()
        .expect("the section page renders");

        assert!(html.contains("\"/hod/exams\""));
        assert!(html.contains("\"/hod/marks\""));
        assert!(html.contains("\"/hod/manage/academic\""));
        assert!(!html.contains("/admin/"), "an HOD page must not link to /admin");
        // The breadcrumb leads back to the caller's own dashboard.
        assert!(html.contains("href=\"/hod\""));
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
        let super_admin = user(Role::SuperAdmin, false);
        let staff = user(Role::Staff, false);
        let hod = user(Role::Faculty, true);
        let teacher = user(Role::Faculty, false);
        let student = user(Role::Student, false);

        assert!(lookup(&admin, "users").is_ok());
        // Root authority is a superset of the IT admin: it opens the same
        // areas, including the section that holds the Appointments link.
        assert!(lookup(&super_admin, "users").is_ok());
        assert!(lookup(&super_admin, "settings").is_ok());
        assert!(lookup(&staff, "website").is_ok());
        assert!(lookup(&hod, "students").is_ok());

        // A section that exists, but for somebody else: forbidden, not missing.
        assert!(matches!(lookup(&staff, "academic"), Err(AppError::Forbidden)));
        assert!(matches!(lookup(&hod, "users"), Err(AppError::Forbidden)));
        assert!(matches!(lookup(&hod, "settings"), Err(AppError::Forbidden)));
        assert!(matches!(lookup(&teacher, "students"), Err(AppError::Forbidden)));
        // The IT admin is offered the timetable and the attendance report, and
        // none of the rest of the academic set; the Super Admin, as the
        // superset, still is.
        assert!(lookup(&admin, "timetable").is_ok());
        assert!(lookup(&admin, "attendance").is_ok());
        assert!(matches!(lookup(&admin, "exams"), Err(AppError::Forbidden)));
        assert!(matches!(lookup(&admin, "offerings"), Err(AppError::Forbidden)));
        assert!(lookup(&super_admin, "exams").is_ok());

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
