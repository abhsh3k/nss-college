mod admin;
mod about;
mod auth;
mod dashboards;
mod htmx;
mod hub_courses;
mod hub_pages;
mod public;
mod teacher;

use axum::{
    http::{header, HeaderValue},
    routing::{get, post},
    Router,
};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::{seo, state::AppState};

fn auth_routes() -> Router<AppState> {
    Router::new()
        .route("/login", get(auth::login_form).post(auth::login_submit))
        .route("/logout", post(auth::logout))
        .route(
            "/account/password",
            get(auth::password_form).post(auth::password_submit),
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
        .route("/teacher/sheet", get(dashboards::teacher))
        .route("/hub", get(dashboards::student))
        .route("/hub/timetable", get(hub_pages::timetable))
        .route("/hub/results", get(hub_pages::results))
        .route("/hub/exams", get(hub_pages::exam_timetable))
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
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
}

/// Routes reachable on the public listener. The administrative management
/// router is bound separately to loopback by `main`.
pub fn public_router() -> Router<AppState> {
    let site = Router::new()
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
        ));

    site.merge(auth_routes()).merge(signed_in_public_routes())
}

/// Routes for the loopback-only management listener. Keeping this router
/// separate makes the network boundary independent of navigation and role
/// checks.
pub fn management_router() -> Router<AppState> {
    auth_routes()
        .merge(admin::routes())
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
}

/// Compatibility constructor for test harnesses that still need the complete
/// route tree. Production uses `public_router` and `management_router`.
pub fn router() -> Router<AppState> {
    public_router().merge(admin::routes())
}
