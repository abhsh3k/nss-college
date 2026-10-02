//! Shared pieces for every signed-in page: sidebar navigation and the CSRF token.

use tower_sessions::Session;

use crate::{
    auth::{csrf, AuthUser, Role},
    error::AppError,
};

pub struct NavItem {
    pub label: &'static str,
    pub href: &'static str,
    /// Planned but not built yet; shown greyed out instead of linking to a 404.
    pub soon: bool,
}

pub struct Shell {
    pub user_name: String,
    pub role_label: &'static str,
    pub nav: Vec<NavItem>,
    pub csrf_token: String,
}

fn item(label: &'static str, href: &'static str, soon: bool) -> NavItem {
    NavItem { label, href, soon }
}

fn nav_for(role: Role) -> Vec<NavItem> {
    match role {
        Role::Admin => vec![
            item("Overview", "/admin", false),
            item("People", "/admin/people", true),
            item("Courses and programmes", "/admin/academics", true),
            item("Timetable", "/admin/timetable", true),
            item("Attendance reports", "/admin/attendance", true),
            item("Notices", "/admin/notices", true),
            item("News", "/admin/news", true),
            item("Events", "/admin/events", true),
            item("Documents", "/admin/documents", true),
            item("Pages", "/admin/pages", true),
            item("Settings", "/admin/settings", true),
        ],
        Role::Staff => vec![
            item("Overview", "/admin", false),
            item("Notices", "/admin/notices", true),
            item("News", "/admin/news", true),
            item("Events", "/admin/events", true),
            item("Documents", "/admin/documents", true),
        ],
        Role::Faculty => vec![
            item("Today", "/teacher", false),
            item("My timetable", "/teacher/timetable", true),
            item("Mark attendance", "/teacher/attendance", true),
            item("Reports", "/teacher/reports", true),
        ],
        Role::Student => vec![
            item("Overview", "/hub", false),
            item("My courses", "/hub/courses", true),
            item("Timetable", "/hub/timetable", true),
            item("Attendance", "/hub/attendance", true),
            item("Notices", "/hub/notices", true),
        ],
        Role::Alumni => vec![],
    }
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
            nav: nav_for(user.role),
            csrf_token: csrf::token(session).await?,
        })
    }
}
