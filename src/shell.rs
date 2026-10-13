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
    /// True for the roles that keep a public profile on the staff page (a
    /// teacher, the Principal or office staff), so the sidebar offers the link
    /// only where it opens something.
    pub has_profile: bool,
}

fn item(label: &'static str, href: &'static str) -> NavItem {
    NavItem { label, href }
}

fn group(title: Option<&'static str>, items: Vec<NavItem>) -> NavGroup {
    NavGroup { title, items }
}

/// The academic part of the management rail, as a role may draw it.
///
/// The Super Admin (the superset) draws the whole set. The IT administrator
/// draws two of its groups on their own: the timetable, which needs a hand that
/// reaches every department to stay clash-free, and the attendance report,
/// which it reads and exports. Everything else in the set would be a link to a
/// page that answers `403` (a substitution, a register correction, the course
/// structure, marks), so the rail is composed per role rather than filtered
/// from one list.
#[derive(Clone, Copy)]
struct AcademicNav {
    /// The full set: courses, offerings, timetable, attendance, examinations.
    full: bool,
    timetable: bool,
    attendance: bool,
}

/// What the IT administrator draws: the two groups it keeps, each without the
/// head of department's entry (substitutions, department attendance).
const IT_ADMIN_SCHEDULE: AcademicNav =
    AcademicNav { full: false, timetable: true, attendance: true };
const FULL_ACADEMIC: AcademicNav = AcademicNav { full: true, timetable: true, attendance: true };

/// The management-area navigation.
///
/// The academic groups are the only part that differs by role: the IT
/// administrator keeps the site, its accounts and its content plus the two
/// academic tools whose guards admit it, while the Super Admin gets the lot.
/// The Work-queue link sits with the full academic set — it is `Manager`-guarded,
/// so the IT admin is not offered it either.
fn admin_groups(academic: AcademicNav) -> Vec<NavGroup> {
    let mut summary = vec![
        item("Overview", "/admin", ),
        item("Notifications", "/admin/notifications", ),
    ];
    if academic.full {
        summary.insert(1, item("Work queue", "/admin/work-queue", ));
    }

    let mut groups = vec![
        group(None, summary),
        group(
            Some("User & accounts"),
            vec![
                item("People", "/admin/people", ),
                item("Import students", "/admin/people/import", ),
                item("Appointments", "/admin/appointments", ),
            ],
        ),
    ];

    if academic.full {
        groups.push(group(
            Some("Academic management"),
            vec![
                item("Departments", "/admin/departments", ),
                item("Courses and programmes", "/admin/academics", ),
                item("Course catalogue", "/admin/courses", ),
                item("Study materials", "/admin/materials", ),
            ],
        ));
        groups.push(group(
            Some("Course operations"),
            vec![
                item("Course offerings", "/admin/courses/offerings", ),
                item("External offerings", "/admin/courses/external", ),
                item("Student selections", "/admin/courses/selections", ),
            ],
        ));
    }
    if academic.full || academic.timetable {
        let mut items = vec![item("Timetable", "/admin/timetable", )];
        if academic.full {
            items.push(item("Substitutions", "/admin/substitutions", ));
        }
        groups.push(group(Some("Timetable & scheduling"), items));
    }
    if academic.full || academic.attendance {
        let mut items = vec![item("Attendance reports", "/admin/attendance", )];
        if academic.full {
            items.push(item("Department attendance", "/admin/departments/attendance", ));
        }
        groups.push(group(Some("Attendance"), items));
    }
    if academic.full {
        groups.push(group(
            Some("Examinations & results"),
            vec![
                item("Exam timetable", "/admin/exams", ),
                item("Marks entry", "/admin/marks", ),
            ],
        ));
    }

    groups.push(group(
        Some("Website & content"),
        vec![
            item("Notices", "/admin/notices", ),
            item("News", "/admin/news", ),
            item("Events", "/admin/events", ),
            item("Documents", "/admin/documents", ),
            item("Pages", "/admin/pages", ),
            item("Rank holders", "/admin/rank-holders", ),
        ],
    ));
    groups.push(group(
        Some("Settings"),
        vec![item("Site settings", "/admin/settings", )],
    ));
    groups
}

fn nav_for(user: &AuthUser) -> Vec<NavGroup> {
    // Root authority: the management areas above, plus the screen where roles,
    // recovery and the emergency reopening of results live.
    if user.role == Role::SuperAdmin {
        let mut nav = admin_groups(FULL_ACADEMIC);
        nav.push(group(
            Some("Root authority"),
            vec![item("Roles and recovery", "/admin/roles", )],
        ));
        return nav;
    }
    let nav = match user.role {
        Role::Admin => admin_groups(IT_ADMIN_SCHEDULE),
        // Returned above; listed so the match stays exhaustive.
        Role::SuperAdmin => admin_groups(FULL_ACADEMIC),
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
            has_profile: matches!(user.role, Role::Faculty | Role::Principal | Role::Staff),
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

    /// The IT administrator is a site/accounts role that keeps two academic
    /// tools: the timetable it must keep clash-free and the attendance report it
    /// reads and exports. Every other academic link would answer 403, so none is
    /// offered.
    #[test]
    fn the_it_admin_nav_is_the_site_the_schedule_and_the_attendance_report() {
        let links = hrefs(&nav_for(&user(Role::Admin, false)));
        for kept in [
            "/admin",
            "/admin/notifications",
            "/admin/people",
            "/admin/people/import",
            "/admin/appointments",
            "/admin/timetable",
            "/admin/attendance",
            "/admin/notices",
            "/admin/news",
            "/admin/events",
            "/admin/documents",
            "/admin/pages",
            "/admin/rank-holders",
            "/admin/settings",
        ] {
            assert!(links.contains(&kept), "the IT admin nav lost {kept}");
        }
        for refused in [
            "/admin/work-queue",
            "/admin/academics",
            "/admin/courses",
            "/admin/courses/offerings",
            "/admin/substitutions",
            "/admin/departments",
            "/admin/departments/attendance",
            "/admin/materials",
            "/admin/marks",
            "/admin/exams",
        ] {
            assert!(!links.contains(&refused), "the IT admin nav gained {refused}");
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
