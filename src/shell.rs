//! Shared pieces for every signed-in page: sidebar navigation and the CSRF token.

use tower_sessions::Session;

use crate::{
    auth::{csrf, AuthUser, Role},
    error::{internal, AppError},
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
    pub flash: Option<String>,
}

fn item(label: &'static str, href: &'static str, soon: bool) -> NavItem {
    NavItem { label, href, soon }
}

fn nav_for(user: &AuthUser) -> Vec<NavItem> {
    let nav = match user.role {
        Role::Admin => vec![
            item("Overview", "/admin", false),
            item("People", "/admin/people", false),
            item("Courses and programmes", "/admin/academics", false),
            item("Timetable", "/admin/timetable", false),
            item("Substitutions", "/admin/substitutions", false),
            item("Departments", "/admin/departments", false),
            item("Department attendance", "/admin/departments/attendance", false),
            item("Attendance reports", "/admin/attendance", false),
            item("Notices", "/admin/notices", false),
            item("News", "/admin/news", false),
            item("Events", "/admin/events", false),
            item("Documents", "/admin/documents", true),
            item("Pages", "/admin/pages", true),
            item("Settings", "/admin/settings", true),
        ],
        Role::Staff => vec![
            item("Overview", "/admin", false),
            item("Notices", "/admin/notices", false),
            item("News", "/admin/news", false),
            item("Events", "/admin/events", false),
            item("Documents", "/admin/documents", true),
        ],
        Role::Faculty => {
            let mut items = vec![
                item("Today", "/teacher", false),
                item("My timetable", "/teacher/timetable", false),
                item("Mark attendance", "/teacher/attendance", false),
                item("Reports", "/teacher/reports", false),
                item("Attendance sheet", "/teacher/sheet", false),
            ];
            if user.manages_department() {
                items.push(item("Courses and programmes", "/admin/academics", false));
                items.push(item("Timetable", "/admin/timetable", false));
                items.push(item("Substitutions", "/admin/substitutions", false));
                items.push(item("Students", "/admin/departments", false));
                items.push(item("Department attendance", "/admin/departments/attendance", false));
            }
            items
        }
        Role::Student => vec![
            item("Overview", "/hub", false),
            item("Today's classes", "/hub#today", false),
            item("Timetable", "/hub#timetable", false),
            item("My courses", "/hub#courses", false),
            item("Results", "/hub#results", false),
            item("Notices", "/hub#notices", false),
        ],
        Role::Alumni => vec![],
    };
    nav
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
