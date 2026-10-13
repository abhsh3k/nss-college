mod admin;
mod about;
mod auth;
mod dashboards;
mod htmx;
mod hub_courses;
mod hub_pages;
mod profile;
mod public;
mod teacher;

use axum::{
    extract::DefaultBodyLimit,
    http::{header, HeaderValue},
    routing::{get, post},
    Router,
};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::{seo, state::AppState};

/// The department workspace, `/hod`.
///
/// A head of department is a teacher: they sign in on the *public* listener,
/// where the `/admin` management area is deliberately not served. The same
/// manager-scoped handlers are therefore mounted a second time under `/hod`,
/// with identical per-request guards, so the pages work on the listener the HOD
/// is actually using. See `crate::routes::admin::manager_routes`.
fn department_routes() -> Router<AppState> {
    admin::manager_routes().layer(SetResponseHeaderLayer::overriding(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    ))
}

fn auth_routes() -> Router<AppState> {
    Router::new()
        .route("/login", get(auth::login_form).post(auth::login_submit))
        .route("/logout", post(auth::logout))
        .route(
            "/account/password",
            get(auth::password_form).post(auth::password_submit),
        )
        // Password re-entry for critical operations.
        .route(
            "/account/reauth",
            get(auth::reauth_form).post(auth::reauth_submit),
        )
}

fn signed_in_public_routes() -> Router<AppState> {
    // Signed-in pages: never cached, so the back button after sign-out shows
    // nothing private. Administrative routes are deliberately not included.
    Router::new()
        .route("/teacher", get(teacher::today))
        .route("/teacher/attendance", get(teacher::attendance_list))
        .route(
            "/teacher/attendance/:entry_id/:date",
            get(teacher::mark_form).post(teacher::mark_save),
        )
        .route("/teacher/timetable", get(teacher::timetable))
        .route("/teacher/reports", get(teacher::reports))
        .route(
            "/teacher/materials",
            get(teacher::materials_page).post(teacher::materials_upload),
        )
        .route("/teacher/sheet", get(dashboards::teacher))
        // The self-service staff profile, for a teacher, the Principal or
        // office staff — whichever listener they signed in on.
        .route("/profile", get(profile::show).post(profile::save))
        .route("/hub", get(dashboards::student))
        .route("/hub/timetable", get(hub_pages::timetable))
        .route("/hub/results", get(hub_pages::results))
        .route("/hub/exams", get(hub_pages::exam_timetable))
        .route("/hub/materials", get(hub_pages::materials))
        .route("/hub/courses", get(hub_courses::my_courses))
        .route("/hub/courses/select", post(hub_courses::select))
        .route("/hub/courses/withdraw", post(hub_courses::withdraw))
        .route("/hub/courses/confirm", post(hub_courses::confirm))
        .route("/hub/courses/change", post(hub_courses::request_change))
        .route(
            "/dashboard/teacher/session/:entry_id/attendance",
            get(teacher::get_attendance_sheet),
        )
        .route(
            "/dashboard/teacher/session/:session_id/toggle",
            post(teacher::toggle_attendance_status),
        )
        // The teacher material form uploads a file, so the whole signed-in
        // tree gets a body limit above axum's 2 MB default.
        .layer(DefaultBodyLimit::max(crate::uploads::MAX_FILE_BYTES + 2 * 1024 * 1024))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
}

/// The `/admin` management area: the landing dashboard and the technical and
/// content tools. It is mounted on both listeners; the public mount is wrapped
/// by [`crate::auth::management_area_on_public`], which keeps root authority
/// loopback-only.
fn management_area_routes() -> Router<AppState> {
    Router::new()
        .route("/admin", get(dashboards::admin))
        .merge(admin::routes())
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
}

/// The `/staff` office-staff workspace, mounted on both listeners so office
/// staff can work from the public one.
fn staff_routes() -> Router<AppState> {
    Router::new()
        .nest("/staff", admin::staff_routes())
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
}

/// The public informational site: the home page, the academic and content
/// pages, uploads, and the database-backed page fallback.
///
/// It is a separate router because both listeners serve it: the public one for
/// visitors, and the loopback management one so an administrator can see
/// exactly what a visitor sees without leaving the management port.
fn site_routes() -> Router<AppState> {
    Router::new()
        .route("/", get(public::home))
        .route("/academics", get(public::programmes))
        .route("/academics/rank-holders", get(public::rank_holders))
        .route("/academics/:slug", get(public::programme))
        .route("/departments", get(public::departments))
        .route("/departments/:slug", get(public::department))
        .route("/news", get(public::news_list))
        .route("/news/:id", get(public::news_item))
        .route("/notices", get(public::notices))
        .route("/contact", get(public::contact))
        .route("/about/staff", get(about::page))
        .route("/fragments/notices", get(htmx::notices))
        .route("/fragments/account-link", get(htmx::account_link))
        .route("/uploads/*path", get(public::upload))
        .route("/robots.txt", get(seo::robots))
        .route("/sitemap.xml", get(seo::sitemap))
        .route("/healthz", get(public::health))
        // Informational pages are rendered from the database, so an edit made
        // in the admin must not be hidden behind a cached copy.
        .fallback(public::page_or_404)
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ))
}

/// Routes reachable on the public listener. The administrative management
/// router is bound separately to loopback by `main`, but its pages are also
/// served here for the roles that work on the public listener — root authority
/// excepted.
pub fn public_router() -> Router<AppState> {
    site_routes()
        .merge(auth_routes())
        .merge(signed_in_public_routes())
        // The management area is served here too, for the roles whose home is
        // `/admin` (the IT administrator and the Principal) and for office
        // staff in `/staff`. The layer refuses root authority and a head of
        // department on this listener; an HOD keeps `/hod`, mounted below.
        .merge(
            management_area_routes()
                .merge(staff_routes())
                .layer(axum::middleware::from_fn(
                    crate::auth::management_area_on_public,
                )),
        )
        // The head of department's pages, on the listener they sign in to.
        .merge(Router::new().nest("/hod", department_routes()))
}

/// Routes for the loopback-only management listener. Keeping this router
/// separate makes the network boundary independent of navigation and role
/// checks.
///
/// The whole public website is served here as well — the loopback copy of
/// `site_routes` — so an administrator on the management port can browse the
/// site as a visitor before changing it. The management routes are layered on
/// top; `/admin/roles` and the other root-only screens stay reachable only on
/// this listener.
pub fn management_router() -> Router<AppState> {
    site_routes()
        // `/profile` is a signed-in page for a teacher, the Principal or office
        // staff, and those roles sign in on either listener.
        .merge(signed_in_public_routes())
        .merge(auth_routes())
        // The role landing page for IT admin and office staff (`Role::home`).
        // It is `AdminDashboard`, so all four management roles reach it; the
        // cards it lists are narrowed to the content area for office staff.
        .merge(management_area_routes())
        // Office staff work in `/staff`, on both listeners.
        .merge(staff_routes())
        // The department workspace is also offered here, so a head of
        // department who is at the server itself lands somewhere that works.
        .merge(Router::new().nest("/hod", department_routes()))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
}
