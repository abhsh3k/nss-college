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
    /// Planned but not built yet; shown greyed out instead of linking to a 404.
    pub soon: bool,
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

fn item(label: &'static str, href: &'static str, soon: bool) -> NavItem {
    NavItem { label, href, soon }
}

fn group(title: Option<&'static str>, items: Vec<NavItem>) -> NavGroup {
    NavGroup { title, items }
}

fn nav_for(user: &AuthUser) -> Vec<NavGroup> {
    let nav = match user.role {
        Role::Admin => vec![
            group(
                None,
                vec![
                    item("Overview", "/admin", false),
                    item("Work queue", "/admin/work-queue", false),
                ],
            ),
            group(
                Some("User & accounts"),
                vec![
                    item("People", "/admin/people", false),
                    item("Import students", "/admin/people/import", false),
                ],
            ),
            group(
                Some("Academic management"),
                vec![
                    item("Departments", "/admin/departments", false),
                    item("Courses and programmes", "/admin/academics", false),
                    item("Course catalogue", "/admin/courses", false),
                ],
            ),
            group(
                Some("Course operations"),
                vec![
                    item("Course offerings", "/admin/courses/offerings", false),
                    item("External offerings", "/admin/courses/external", false),
                    item("Student selections", "/admin/courses/selections", false),
                ],
            ),
            group(
                Some("Timetable & scheduling"),
                vec![
                    item("Timetable", "/admin/timetable", false),
                    item("Substitutions", "/admin/substitutions", false),
                ],
            ),
            group(
                Some("Attendance"),
                vec![
                    item("Attendance reports", "/admin/attendance", false),
                    item("Department attendance", "/admin/departments/attendance", false),
                ],
            ),
            group(
                Some("Examinations & results"),
                vec![
                    item("Exam timetable", "/admin/exams", false),
                    item("Marks entry", "/admin/marks", false),
                ],
            ),
            group(
                Some("Website & content"),
                vec![
                    item("Notices", "/admin/notices", false),
                    item("News", "/admin/news", false),
                    item("Events", "/admin/events", false),
                    item("Documents", "/admin/documents", false),
                    item("Pages", "/admin/pages", false),
                    item("Rank holders", "/admin/rank-holders", false),
                ],
            ),
            group(
                Some("Settings"),
                vec![item("Site settings", "/admin/settings", false)],
            ),
        ],
        Role::Staff => vec![
            group(None, vec![item("Overview", "/admin", false)]),
            // Office staff are `OfficeOrAdmin`: content and nothing else.
            group(
                Some("Website & content"),
                vec![
                    item("Notices", "/admin/notices", false),
                    item("News", "/admin/news", false),
                    item("Events", "/admin/events", false),
                ],
            ),
        ],
        Role::Faculty => {
            let mut groups = vec![group(
                None,
                vec![
                    item("Today", "/teacher", false),
                    item("My timetable", "/teacher/timetable", false),
                    item("Mark attendance", "/teacher/attendance", false),
                    item("Reports", "/teacher/reports", false),
                    item("Attendance sheet", "/teacher/sheet", false),
                ],
            )];
            if user.manages_department() {
                // Department tools. Every one of these handlers takes `Manager`,
                // which confines the query to the HOD's own department.
                groups.push(group(
                    Some("Your department"),
                    vec![item("Work queue", "/admin/work-queue", false)],
                ));
                groups.push(group(
                    Some("Department & students"),
                    vec![item("Students", "/admin/departments", false)],
                ));
                groups.push(group(
                    Some("Academic management"),
                    vec![
                        item("Courses and programmes", "/admin/academics", false),
                        item("Course catalogue", "/admin/courses", false),
                    ],
                ));
                groups.push(group(
                    Some("Course operations"),
                    vec![
                        item("My offerings", "/admin/courses/offerings", false),
                        item("External offerings", "/admin/courses/external", false),
                        item("Student selections", "/admin/courses/selections", false),
                    ],
                ));
                groups.push(group(
                    Some("Timetable & scheduling"),
                    vec![
                        item("Timetable", "/admin/timetable", false),
                        item("Substitutions", "/admin/substitutions", false),
                    ],
                ));
                groups.push(group(
                    Some("Attendance"),
                    vec![item("Department attendance", "/admin/departments/attendance", false)],
                ));
                groups.push(group(
                    Some("Examinations & results"),
                    vec![
                        item("Exam timetable", "/admin/exams", false),
                        item("Marks entry", "/admin/marks", false),
                    ],
                ));
            }
            groups
        }
        Role::Student => vec![group(
            None,
            vec![
                item("Overview", "/hub", false),
                item("Timetable", "/hub/timetable", false),
                item("My courses", "/hub/courses", false),
                item("Results", "/hub/results", false),
                item("Exam timetable", "/hub/exams", false),
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
