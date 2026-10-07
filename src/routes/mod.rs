mod admin;
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

pub fn router() -> Router<AppState> {
    // Signed-in pages: never cached, so the back button after sign-out shows nothing private.
    let protected = Router::new()
        .route("/login", get(auth::login_form).post(auth::login_submit))
        .route("/logout", post(auth::logout))
        .route(
            "/account/password",
            get(auth::password_form).post(auth::password_submit),
        )
        .route("/admin", get(dashboards::admin))
        .route("/teacher", get(teacher::today))
        .route("/teacher/attendance", get(teacher::attendance_list))
        .route(
            "/teacher/attendance/:entry_id/:date",
            get(teacher::mark_form).post(teacher::mark_save),
        )
        .route("/teacher/timetable", get(teacher::timetable))
        .route("/teacher/reports", get(teacher::reports))
        // The newer HTMX attendance sheet, kept alongside the pages above.
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
        // Add these alongside your existing dashboard routes:
        .route("/dashboard/teacher/session/:entry_id/attendance", axum::routing::get(teacher::get_attendance_sheet))
        .route("/dashboard/teacher/session/:session_id/toggle", axum::routing::post(teacher::toggle_attendance_status))
        .merge(admin::routes())
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ));

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
        .route("/fragments/notices", get(htmx::notices))
        .route("/fragments/account-link", get(htmx::account_link))
        .route("/robots.txt", get(seo::robots))
        .route("/sitemap.xml", get(seo::sitemap))
        .route("/healthz", get(public::health))
        // Informational pages (about, IQAC, fees, ...) are looked up by path in the database,
        // so pages created in the admin dashboard work without a restart.
        .fallback(public::page_or_404)
        // The public pages are rendered from the database, so an edit made in
        // the admin must not be hidden behind a cached copy. `no-cache` makes
        // the browser revalidate every time, on the routes above and on the
        // fallback alike. It is applied before the merge, so the signed-in
        // pages keep their stricter `no-store` from their own layer.
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ))
        .merge(protected)
}
