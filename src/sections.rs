//! The administration tools grouped into management areas.
//!
//! A *section* (the code calls it an `Area`) is a pure navigation concept: it
//! is a title, a one-line description and a list of links to pages that
//! already exist. Nothing here grants, widens or narrows access — every target
//! page keeps its own guard (`AdminOnly`, `OfficeOrAdmin`, `Manager`, ...)
//! checked on the server, so a section can only ever be a *shorter* route to a
//! page the caller could already open.
//!
//! Three catalogues are served, one per audience:
//!
//! * [`admin`]  — the IT administrator (`/admin` overview),
//! * [`staff`]  — office staff, who may only publish notices, news and events,
//! * [`hod`]    — a head of department / approved manager.
//!
//! The catalogues are deliberately not the same list: office staff never see
//! academic tools, and an HOD never sees college-wide pages such as People or
//! Settings. That difference is a UX mirror of the guards in `src/auth`, not a
//! replacement for them.

/// One entry inside a section: a page the user can open, and why they would.
pub struct Link {
    pub label: &'static str,
    pub href: &'static str,
    pub desc: &'static str,
}

const fn link(label: &'static str, href: &'static str, desc: &'static str) -> Link {
    Link { label, href, desc }
}

/// A management area: what the dashboard card leads to, and what the sidebar
/// groups its links under.
pub struct Area {
    /// URL slug for the section index page: `/admin/manage/{key}`.
    pub key: &'static str,
    pub title: &'static str,
    /// One line shown under the title on the dashboard card and the section page.
    pub desc: &'static str,
    pub items: &'static [Link],
}

const fn area(
    key: &'static str,
    title: &'static str,
    desc: &'static str,
    items: &'static [Link],
) -> Area {
    Area {
        key,
        title,
        desc,
        items,
    }
}

// ---------------------------------------------------------------- IT admin --

/// Every area an IT administrator owns, in dashboard order.
pub const ADMIN: &[Area] = &[
    area(
        "users",
        "User & Account Management",
        "Manage users, accounts, credentials and student account operations.",
        &[
            link(
                "People",
                "/admin/people",
                "View, edit, activate or deactivate accounts, issue a new temporary password, delete and promote.",
            ),
            link(
                "Add a person",
                "/admin/people/new",
                "Create one student, teacher or office-staff account with a temporary password.",
            ),
            link(
                "Import students",
                "/admin/people/import",
                "Paste a class list, review it, create the accounts and download the credentials CSV.",
            ),
            link(
                "Promotions",
                "/admin/people",
                "Promote a student or a whole programme cohort to the next semester.",
            ),
        ],
    ),
    area(
        "academic",
        "Academic Management",
        "Departments, programmes, courses and enrolment.",
        &[
            link(
                "Departments",
                "/admin/departments",
                "See who sits in each department and place students into it.",
            ),
            link(
                "Courses and programmes",
                "/admin/academics",
                "Programme structure, semester courses, teachers and enrolment.",
            ),
            link(
                "Course catalogue",
                "/admin/courses",
                "The catalogue of courses that course offerings are built from.",
            ),
        ],
    ),
    area(
        "offerings",
        "Course Offerings & Selection",
        "Which courses are being offered this term, and which students take them.",
        &[
            link(
                "Course offerings",
                "/admin/courses/offerings",
                "Offer a course, set its target cohorts, publish it and schedule its periods.",
            ),
            link(
                "External offerings",
                "/admin/courses/external",
                "Approve or reject courses other departments want their students to take.",
            ),
            link(
                "Student selections",
                "/admin/courses/selections",
                "Assign students, decide change requests and finalise cohort choices.",
            ),
        ],
    ),
    area(
        "timetable",
        "Timetable & Scheduling",
        "Manage class schedules, timetable slots and teacher substitutions.",
        &[
            link(
                "Timetable",
                "/admin/timetable",
                "Add and remove periods per day; clashes on teacher, room or class are refused.",
            ),
            link(
                "Substitutions",
                "/admin/substitutions",
                "Cover an absent teacher day by day, and correct a past substitution.",
            ),
        ],
    ),
    area(
        "attendance",
        "Attendance Management",
        "Attendance, corrections, reports and exports.",
        &[
            link(
                "Attendance reports",
                "/admin/attendance",
                "Per-course and per-student percentages, low-attendance flags and the CSV export.",
            ),
            link(
                "Department attendance",
                "/admin/departments/attendance",
                "Open a past period in your department and correct what was recorded.",
            ),
        ],
    ),
    area(
        "exams",
        "Examination & Results",
        "Manage examinations, marks, results and publication.",
        &[
            link(
                "Exam timetable",
                "/admin/exams",
                "Assemble the timetable for a programme and semester, then push or unpush it.",
            ),
            link(
                "Marks and results",
                "/admin/marks",
                "Enter marks in the grid or import them by CSV, finalise them and publish the results.",
            ),
        ],
    ),
    area(
        "website",
        "Website & Content Management",
        "Everything a visitor sees on the public college website.",
        &[
            link(
                "Notices",
                "/admin/notices",
                "Create, edit, publish or archive notices.",
            ),
            link(
                "News",
                "/admin/news",
                "Create, edit, publish or archive news items.",
            ),
            link(
                "Events",
                "/admin/events",
                "Create, edit, publish or archive events.",
            ),
            link(
                "Documents",
                "/admin/documents",
                "Upload a file, edit its details, publish or remove it.",
            ),
            link(
                "Pages",
                "/admin/pages",
                "Informational pages and their sections: create, edit, publish, delete and reorder.",
            ),
            link(
                "Rank holders",
                "/admin/rank-holders",
                "University rank holders: photos, ordering and publication.",
            ),
        ],
    ),
    area(
        "settings",
        "System / Configuration",
        "College-wide settings that drive the site and the dashboards.",
        &[
            link(
                "Site settings",
                "/admin/settings",
                "College facts, contact details, home-page blocks and display defaults.",
            ),
        ],
    ),
];

// ----------------------------------------------------------- office staff ---

/// Office staff publish content; they have no academic or account powers, so
/// their overview is one area rather than eight.
pub const STAFF: &[Area] = &[area(
    "website",
    "Website & Content Management",
    "Notices, news and events for the public college website.",
    &[
        link(
            "Notices",
            "/admin/notices",
            "Create, edit, publish or archive notices.",
        ),
        link(
            "News",
            "/admin/news",
            "Create, edit, publish or archive news items.",
        ),
        link(
            "Events",
            "/admin/events",
            "Create, edit, publish or archive events.",
        ),
    ],
)];

// ------------------------------------------------------- head of department --

/// What a head of department (or an approved `can_manage` delegate) may reach.
///
/// Every link here sits behind the `Manager` guard, which confines the query to
/// `faculty.department_id`. College-wide pages (`/admin/people`,
/// `/admin/settings`, `/admin/attendance`, ...) are deliberately absent: their
/// handlers take `AdminOnly` and would answer 403 anyway.
pub const HOD: &[Area] = &[
    area(
        "students",
        "Department & Students",
        "The students in your department and where they sit.",
        &[link(
            "Department students",
            "/admin/departments",
            "List your department's students and place or move them between departments.",
        )],
    ),
    area(
        "academic",
        "Academic Management",
        "Department programmes, courses and enrolment.",
        &[
            link(
                "Courses and programmes",
                "/admin/academics",
                "Programme structure, semester courses and enrolling your department's students.",
            ),
            link(
                "Course catalogue",
                "/admin/courses",
                "The catalogue of courses that course offerings are built from.",
            ),
        ],
    ),
    area(
        "offerings",
        "Course Offerings & Selection",
        "Offerings, selections and approvals for your department.",
        &[
            link(
                "Course offerings",
                "/admin/courses/offerings",
                "Offer a course, set its target cohorts, publish it and schedule its periods.",
            ),
            link(
                "External offerings",
                "/admin/courses/external",
                "Approve or reject courses other departments want your students to take.",
            ),
            link(
                "Student selections",
                "/admin/courses/selections",
                "Assign students, decide change requests and finalise cohort choices.",
            ),
        ],
    ),
    area(
        "timetable",
        "Timetable & Scheduling",
        "Your department's timetable, periods and substitutions.",
        &[
            link(
                "Timetable",
                "/admin/timetable",
                "Add and remove periods per day, including periods of a published offering.",
            ),
            link(
                "Substitutions",
                "/admin/substitutions",
                "Cover an absent teacher in your department day by day.",
            ),
        ],
    ),
    area(
        "attendance",
        "Attendance",
        "Department attendance and your own class reports.",
        &[
            link(
                "Department attendance",
                "/admin/departments/attendance",
                "Open a past period in your department and correct what was recorded.",
            ),
            link(
                "Attendance reports",
                "/teacher/reports",
                "Per-course and per-student percentages for the classes you teach.",
            ),
        ],
    ),
    area(
        "exams",
        "Examination & Results",
        "Your department's exam timetable, marks and results.",
        &[
            link(
                "Exam timetable",
                "/admin/exams",
                "Assemble the timetable for a programme and semester, then push or unpush it.",
            ),
            link(
                "Marks and results",
                "/admin/marks",
                "Enter marks in the grid or import them by CSV, finalise them and publish the results.",
            ),
        ],
    ),
];

/// Look an area up by its slug.
pub fn find<'a>(areas: &'a [Area], key: &str) -> Option<&'a Area> {
    areas.iter().find(|a| a.key == key)
}

/// Whether *any* catalogue knows this slug.
///
/// Used to tell a typo (`404`) apart from a section that belongs to somebody
/// else's role (`403`): both are refused, but only one of them is a mistake.
pub fn exists_any(key: &str) -> bool {
    [ADMIN, STAFF, HOD].iter().any(|list| find(list, key).is_some())
}

/// A `Shell` with no session behind it, so the new pages can be rendered in a
/// unit test without a database.
#[cfg(test)]
pub(crate) fn test_shell(role_label: &'static str) -> crate::shell::Shell {
    crate::shell::Shell {
        user_name: "Test User".into(),
        role_label,
        nav: Vec::new(),
        csrf_token: "csrf".into(),
        flash: None,
        site: crate::models::SiteInfo::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys_are_unique(areas: &[Area]) {
        for (i, a) in areas.iter().enumerate() {
            assert!(!a.key.is_empty(), "an area has no key");
            assert!(
                areas[i + 1..].iter().all(|b| b.key != a.key),
                "duplicate area key {}",
                a.key
            );
            assert!(!a.title.is_empty() && !a.desc.is_empty(), "{} is bare", a.key);
            assert!(!a.items.is_empty(), "{} has nothing in it", a.key);
            for l in a.items {
                assert!(l.href.starts_with('/'), "{}: {}", a.key, l.href);
                assert!(!l.label.is_empty() && !l.desc.is_empty(), "{} has a bare link", a.key);
            }
        }
    }

    #[test]
    fn every_catalogue_is_self_consistent() {
        keys_are_unique(ADMIN);
        keys_are_unique(STAFF);
        keys_are_unique(HOD);
    }

    /// A link an HOD could not open would 403 from the page behind it, so the
    /// department catalogue must only ever point at `Manager`-guarded pages.
    #[test]
    fn the_hod_catalogue_has_no_college_wide_pages() {
        let forbidden = [
            "/admin/people",
            "/admin/people/new",
            "/admin/people/import",
            "/admin/settings",
            "/admin/attendance",
            "/admin/notices",
            "/admin/news",
            "/admin/events",
            "/admin/documents",
            "/admin/pages",
            "/admin/rank-holders",
        ];
        for a in HOD {
            for l in a.items {
                assert!(
                    !forbidden.contains(&l.href),
                    "HOD area {} links to the admin-only page {}",
                    a.key,
                    l.href
                );
            }
        }
    }

    /// Office staff are `OfficeOrAdmin`: content pages and nothing else.
    #[test]
    fn the_staff_catalogue_only_offers_content_pages() {
        for a in STAFF {
            for l in a.items {
                assert!(
                    matches!(l.href, "/admin/notices" | "/admin/news" | "/admin/events"),
                    "office staff were offered {}",
                    l.href
                );
            }
        }
    }

    #[test]
    fn every_admin_href_is_a_registered_route() {
        // The paths from `routes::admin::routes()`, minus the POST-only and the
        // parameterised ones — an area should only ever link to a list/landing
        // page a person can open by hand.
        let routes = [
            "/admin/people",
            "/admin/people/new",
            "/admin/people/import",
            "/admin/academics",
            "/admin/courses",
            "/admin/courses/offerings",
            "/admin/courses/external",
            "/admin/courses/selections",
            "/admin/timetable",
            "/admin/substitutions",
            "/admin/departments",
            "/admin/departments/attendance",
            "/admin/attendance",
            "/admin/exams",
            "/admin/marks",
            "/admin/notices",
            "/admin/news",
            "/admin/events",
            "/admin/documents",
            "/admin/pages",
            "/admin/rank-holders",
            "/admin/settings",
            "/teacher/reports",
        ];
        for a in ADMIN.iter().chain(HOD) {
            for l in a.items {
                assert!(routes.contains(&l.href), "{} links to an unregistered {}", a.key, l.href);
            }
        }
    }

    /// A slug some catalogue owns is a boundary (403); anything else is a typo
    /// (404).
    #[test]
    fn a_slug_is_owned_by_some_catalogue_or_by_none() {
        for key in ["users", "website", "students", "offerings", "settings"] {
            assert!(exists_any(key), "{key} should be known");
        }
        for key in ["does-not-exist", "", "people"] {
            assert!(!exists_any(key), "{key} should be unknown");
        }
    }
}
