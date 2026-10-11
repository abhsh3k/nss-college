//! Shared pieces for every signed-in page: sidebar navigation and the CSRF token.

use tower_sessions::Session;

use crate::{
    auth::{csrf, AuthUser, Role},
    error::{internal, AppError},
    models::SiteInfo,
    site,
};

pub struct NavItem {
    pub label: &'static str,
    pub href: &'static str,
}

/// A run of nav items under one small heading.
///
/// The sidebar used to be a flat list of twenty-odd links; it now mirrors the
/// management areas the dashboards group things into, so a page is found by
/// first knowing which area it belongs to. Only the grouping changed — every
/// `href` below is a route that already existed.
pub struct NavGroup {
    /// The small heading above the group. `None` for the ungrouped items at
    /// the top (the page the role lands on, plus its landing page).
    pub title: Option<&'static str>,
    pub items: Vec<NavItem>,
}

pub struct Shell {
    pub user_name: String,
    pub role_label: &'static str,
    pub nav: Vec<NavGroup>,
    pub csrf_token: String,
    pub flash: Option<String>,
    /// The college name and other chrome, from the settings cache.
    pub site: SiteInfo,
}

fn item(label: &'static str, href: &'static str) -> NavItem {
    NavItem { label, href }
}

fn group(title: Option<&'static str>, items: Vec<NavItem>) -> NavGroup {
    NavGroup { title, items }
}

/// The IT administrator's navigation: every management area it may reach.
///
/// The Super Admin is a superset of the IT admin, so it sees this list too (see
/// [`nav_for`]); the difference between the two is the extra root-authority
/// powers, not a narrower area.
fn admin_groups() -> Vec<NavGroup> {
    vec![
            group(
                None,
                vec![
                    item("Overview", "/admin", ),
                    item("Work queue", "/admin/work-queue", ),
                    item("Notifications", "/admin/notifications", ),
                ],
            ),
            group(
                Some("User & accounts"),
                vec![
                    item("People", "/admin/people", ),
                    item("Import students", "/admin/people/import", ),
                    item("Appointments", "/admin/appointments", ),
                ],
            ),
            group(
                Some("Academic management"),
                vec![
                    item("Departments", "/admin/departments", ),
                    item("Courses and programmes", "/admin/academics", ),
                    item("Course catalogue", "/admin/courses", ),
                    item("Study materials", "/admin/materials", ),
                ],
            ),
            group(
                Some("Course operations"),
                vec![
                    item("Course offerings", "/admin/courses/offerings", ),
                    item("External offerings", "/admin/courses/external", ),
                    item("Student selections", "/admin/courses/selections", ),
                ],
            ),
            group(
                Some("Timetable & scheduling"),
                vec![
                    item("Timetable", "/admin/timetable", ),
                    item("Substitutions", "/admin/substitutions", ),
                ],
            ),
            group(
                Some("Attendance"),
                vec![
                    item("Attendance reports", "/admin/attendance", ),
                    item("Department attendance", "/admin/departments/attendance", ),
                ],
            ),
            group(
                Some("Examinations & results"),
                vec![
                    item("Exam timetable", "/admin/exams", ),
                    item("Marks entry", "/admin/marks", ),
                ],
            ),
            group(
                Some("Website & content"),
                vec![
                    item("Notices", "/admin/notices", ),
                    item("News", "/admin/news", ),
                    item("Events", "/admin/events", ),
                    item("Documents", "/admin/documents", ),
                    item("Pages", "/admin/pages", ),
                    item("Rank holders", "/admin/rank-holders", ),
                ],
            ),
            group(
                Some("Settings"),
                vec![item("Site settings", "/admin/settings", )],
            ),
    ]
}

fn nav_for(user: &AuthUser) -> Vec<NavGroup> {
    // Root authority: the management areas above, plus the screen where roles,
    // recovery and the emergency reopening of results live.
    if user.role == Role::SuperAdmin {
        let mut nav = admin_groups();
        nav.push(group(
            Some("Root authority"),
            vec![item("Roles and recovery", "/admin/roles", )],
        ));
        return nav;
    }
    let nav = match user.role {
        Role::Admin => admin_groups(),
        // Returned above; listed so the match stays exhaustive.
        Role::SuperAdmin => admin_groups(),
        Role::Principal => vec![
            group(
                None,
                vec![
                    item("Overview", "/admin", ),
                    item("Work queue", "/admin/work-queue", ),
                    item("Notifications", "/admin/notifications", ),
                    // Where the Principal signs off the academic appointments.
                    item("Appointments", "/admin/appointments", ),
                ],
            ),
            group(
                Some("Academic management"),
                vec![
                    item("Departments", "/admin/departments", ),
                    item("Courses and programmes", "/admin/academics", ),
                    item("Course catalogue", "/admin/courses", ),
                ],
            ),
            group(
                Some("Course operations"),
                vec![
                    item("Course offerings", "/admin/courses/offerings", ),
                    item("External offerings", "/admin/courses/external", ),
                    item("Student selections", "/admin/courses/selections", ),
                ],
            ),
            group(
                Some("Timetable & scheduling"),
                vec![
                    item("Timetable", "/admin/timetable", ),
                    item("Substitutions", "/admin/substitutions", ),
                ],
            ),
            group(
                Some("Attendance"),
                vec![item("Attendance reports", "/admin/attendance", )],
            ),
            group(
                Some("Examinations & results"),
                vec![
                    item("Exam timetable", "/admin/exams", ),
                    item("Marks entry", "/admin/marks", ),
                ],
            ),
        ],
        Role::Staff => vec![
            group(
                None,
                vec![
                    item("Overview", "/admin", ),
                    item("Notifications", "/admin/notifications", ),
                ],
            ),
            // Office staff are `OfficeOrAdmin`: content and nothing else.
            group(
                Some("Website & content"),
                vec![
                    item("Notices", "/admin/notices", ),
                    item("News", "/admin/news", ),
                    item("Events", "/admin/events", ),
                ],
            ),
        ],
        Role::Faculty => {
            let mut groups = vec![group(
                None,
                vec![
                    item("Today", "/teacher", ),
                    item("My timetable", "/teacher/timetable", ),
                    item("Mark attendance", "/teacher/attendance", ),
                    item("Study materials", "/teacher/materials", ),
                    item("Reports", "/teacher/reports", ),
                    item("Attendance sheet", "/teacher/sheet", ),
                ],
            )];
            if user.manages_department() {
                // Department tools. Every one of these handlers takes `Manager`,
                // which confines the query to the HOD's own department.
                groups.push(group(
                    Some("Your department"),
                    vec![
                        item("Work queue", "/admin/work-queue", ),
                        item("Notifications", "/admin/notifications", ),
                    ],
                ));
                groups.push(group(
                    Some("Department & students"),
                    vec![item("Students", "/admin/departments", )],
                ));
                groups.push(group(
                    Some("Academic management"),
                    vec![
                        item("Courses and programmes", "/admin/academics", ),
                        item("Course catalogue", "/admin/courses", ),
                        item("Study materials", "/admin/materials", ),
                    ],
                ));
                groups.push(group(
                    Some("Course operations"),
                    vec![
                        item("My offerings", "/admin/courses/offerings", ),
                        item("External offerings", "/admin/courses/external", ),
                        item("Student selections", "/admin/courses/selections", ),
                    ],
                ));
                groups.push(group(
                    Some("Timetable & scheduling"),
                    vec![
                        item("Timetable", "/admin/timetable", ),
                        item("Substitutions", "/admin/substitutions", ),
                    ],
                ));
                groups.push(group(
                    Some("Attendance"),
                    vec![item("Department attendance", "/admin/departments/attendance", )],
                ));
                groups.push(group(
                    Some("Examinations & results"),
                    vec![
                        item("Exam timetable", "/admin/exams", ),
                        item("Marks entry", "/admin/marks", ),
                    ],
                ));
            }
            groups
        }
        Role::Student => vec![group(
            None,
            vec![
                item("Overview", "/hub", ),
                item("Timetable", "/hub/timetable", ),
                item("My courses", "/hub/courses", ),
                item("Results", "/hub/results", ),
                item("Exam timetable", "/hub/exams", ),
                item("Study materials", "/hub/materials", ),
            ],
        )],
        Role::Alumni => vec![],
    };
    nav
}

/// Exposed only to tests, so the grouping can be asserted without a session.
#[cfg(test)]
pub(crate) fn test_nav_for(user: &AuthUser) -> Vec<NavGroup> {
    nav_for(user)
}

impl Shell {
    pub async fn build(user: &AuthUser, session: &Session) -> Result<Shell, AppError> {
        Ok(Shell {
            user_name: if user.full_name.is_empty() {
                user.role.label().to_string()
            } else {
                user.full_name.clone()
            },
            role_label: user.role.label(),
            nav: nav_for(user),
            csrf_token: csrf::token(session).await?,
            flash: session.remove::<String>("flash").await.map_err(internal)?,
            site: site::cached(),
        })
    }
}

/// Queue a one-time message shown at the top of the next dashboard page.
pub async fn flash(session: &Session, message: impl Into<String>) -> Result<(), AppError> {
    session
        .insert("flash", message.into())
        .await
        .map_err(internal)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn hrefs(nav: &[NavGroup]) -> Vec<&'static str> {
        nav.iter().flat_map(|g| g.items.iter().map(|i| i.href)).collect()
    }

    #[test]
    fn the_admin_nav_keeps_every_link_it_had_before_the_regrouping() {
        let links = hrefs(&nav_for(&user(Role::Admin, false)));
        for old in [
            "/admin",
            "/admin/work-queue",
            "/admin/people",
            "/admin/academics",
            "/admin/courses",
            "/admin/courses/offerings",
            "/admin/courses/external",
            "/admin/courses/selections",
            "/admin/timetable",
            "/admin/substitutions",
            "/admin/exams",
            "/admin/marks",
            "/admin/departments",
            "/admin/attendance",
            "/admin/notices",
            "/admin/news",
            "/admin/events",
            "/admin/documents",
            "/admin/pages",
            "/admin/rank-holders",
            "/admin/settings",
        ] {
            assert!(links.contains(&old), "the admin nav lost {old}");
        }
    }

    #[test]
    fn the_super_admin_gets_the_admin_areas_plus_root_authority() {
        let links = hrefs(&nav_for(&user(Role::SuperAdmin, false)));
        // Root authority is a superset of the IT admin, not a narrower area.
        for area in ["/admin/people", "/admin/academics", "/admin/settings"] {
            assert!(links.contains(&area), "the Super Admin nav lost {area}");
        }
        assert!(links.contains(&"/admin/roles"));
        // The root-only screen is never offered to anyone else.
        assert!(!hrefs(&nav_for(&user(Role::Admin, false))).contains(&"/admin/roles"));
    }

    #[test]
    fn only_the_first_group_is_unlabelled() {
        for role in [Role::Admin, Role::Staff, Role::Student] {
            let nav = nav_for(&user(role, false));
            assert!(nav[0].title.is_none(), "{role:?} starts with a heading");
            assert!(
                nav.iter().skip(1).all(|g| g.title.is_some()),
                "{role:?} has an unlabelled group below the first"
            );
        }
    }

    #[test]
    fn a_plain_teacher_gets_no_department_link() {
        let links = hrefs(&nav_for(&user(Role::Faculty, false)));
        assert!(links.iter().all(|h| h.starts_with("/teacher")));
        assert!(links.contains(&"/teacher/sheet"));
    }

    #[test]
    fn the_hod_nav_keeps_its_department_tools_and_gains_nothing_college_wide() {
        let links = hrefs(&nav_for(&user(Role::Faculty, true)));
        for old in [
            "/admin/work-queue",
            "/admin/departments",
            "/admin/departments/attendance",
            "/admin/academics",
            "/admin/courses",
            "/admin/courses/offerings",
            "/admin/courses/external",
            "/admin/courses/selections",
            "/admin/timetable",
            "/admin/substitutions",
            "/admin/exams",
            "/admin/marks",
        ] {
            assert!(links.contains(&old), "the HOD nav lost {old}");
        }
        // These handlers are `AdminOnly`; an HOD link to one would 403.
        for forbidden in [
            "/admin/people",
            "/admin/people/import",
            "/admin/settings",
            "/admin/attendance",
            "/admin/notices",
            "/admin/pages",
            "/admin/rank-holders",
        ] {
            assert!(!links.contains(&forbidden), "the HOD nav gained {forbidden}");
        }
    }
}
