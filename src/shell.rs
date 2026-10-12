//! Shared pieces for every signed-in page: sidebar navigation and the CSRF token.

use tower_sessions::Session;

use crate::{
    auth::{csrf, AuthUser, Role},
    error::{internal, AppError},
    models::SiteInfo,
    sections,
    site,
};

pub struct NavItem {
    pub label: &'static str,
    /// The canonical `/admin`-rooted path; see [`NavItem::url`].
    pub href: &'static str,
}

impl NavItem {
    /// This link as it should be rendered in the workspace `base`.
    ///
    /// A head of department works under `/hod` on the public listener, where
    /// the `/admin` area is not served at all, so the shared department links
    /// are rewritten rather than duplicated. Links outside the management area
    /// (`/teacher/...`, `/hub/...`) are returned untouched.
    pub fn url(&self, base: &str) -> String {
        sections::in_workspace(base, self.href)
    }
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
    /// The workspace this caller's links are built in: [`sections::ADMIN_BASE`]
    /// for the management area, [`sections::HOD_BASE`] for a head of
    /// department's own pages. See [`NavItem::url`].
    pub base: &'static str,
    /// Where this caller's own dashboard is (`Role::home`), for breadcrumbs
    /// that lead back out of a management-area page.
    pub home: &'static str,
    pub nav: Vec<NavGroup>,
    pub csrf_token: String,
    pub flash: Option<String>,
    /// The college name and other chrome, from the settings cache.
    pub site: SiteInfo,
    /// Head appointments waiting for a decision, shown as a badge on the
    /// Appointments link. Zero for roles that cannot decide them.
    pub pending_approvals: usize,
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
    /// Build the shared page chrome. `db` is only read for the pending-approval
    /// badge, and only for the roles that can actually decide those approvals.
    pub async fn build(
        user: &AuthUser,
        session: &Session,
        db: &sqlx::PgPool,
    ) -> Result<Shell, AppError> {
        let pending_approvals = match user.role {
            Role::Admin | Role::SuperAdmin | Role::Principal => {
                crate::services::appointments::count_pending_heads(db).await? as usize
            }
            _ => 0,
        };
        Ok(Shell {
            user_name: if user.full_name.is_empty() {
                user.role.label().to_string()
            } else {
                user.full_name.clone()
            },
            role_label: user.role.label(),
            base: sections::base_for(user),
            home: user.role.home(),
            nav: nav_for(user),
            csrf_token: csrf::token(session).await?,
            flash: session.remove::<String>("flash").await.map_err(internal)?,
            site: site::cached(),
            pending_approvals,
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

    /// The same links, rendered for a head of department: the department tools
    /// move into `/hod` (the listener they sign in to), and the teacher's own
    /// pages are left exactly as they are.
    #[test]
    fn the_hod_sidebar_renders_in_the_department_workspace() {
        let nav = nav_for(&user(Role::Faculty, true));
        let urls: Vec<String> = nav
            .iter()
            .flat_map(|g| g.items.iter())
            .map(|i| i.url(crate::sections::HOD_BASE))
            .collect();

        for moved in [
            "/hod/work-queue",
            "/hod/notifications",
            "/hod/departments",
            "/hod/academics",
            "/hod/courses",
            "/hod/courses/offerings",
            "/hod/timetable",
            "/hod/substitutions",
            "/hod/exams",
            "/hod/marks",
        ] {
            assert!(urls.iter().any(|u| u == moved), "the HOD sidebar lost {moved}");
        }
        // Their own pages are not management paths and must not be rewritten.
        assert!(urls.iter().any(|u| u == "/teacher"));
        assert!(urls.iter().any(|u| u == "/teacher/reports"));
        assert!(
            !urls.iter().any(|u| u.starts_with("/admin")),
            "an HOD's sidebar must not point at /admin"
        );
    }

    /// Office staff read every content link under `/staff`, their own workspace.
    #[test]
    fn the_staff_sidebar_renders_in_the_staff_workspace() {
        let nav = nav_for(&user(Role::Staff, false));
        let urls: Vec<String> = nav
            .iter()
            .flat_map(|g| g.items.iter())
            .map(|i| i.url(crate::sections::STAFF_BASE))
            .collect();
        for moved in [
            "/staff",
            "/staff/notifications",
            "/staff/notices",
            "/staff/news",
            "/staff/events",
        ] {
            assert!(urls.iter().any(|u| u == moved), "the staff sidebar lost {moved}");
        }
        assert!(
            !urls.iter().any(|u| u.starts_with("/admin")),
            "office staff must not see /admin links"
        );
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
