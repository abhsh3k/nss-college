mod auth;
mod dashboards;
mod htmx;
mod public;

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
        .route("/teacher", get(dashboards::teacher))
        .route("/hub", get(dashboards::student))
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
        .route("/robots.txt", get(seo::robots))
        .route("/sitemap.xml", get(seo::sitemap))
        .route("/healthz", get(public::health))
        .merge(protected)
        // Informational pages (about, IQAC, fees, ...) are looked up by path in the database,
        // so pages created in the admin dashboard work without a restart.
        .fallback(public::page_or_404)
}
