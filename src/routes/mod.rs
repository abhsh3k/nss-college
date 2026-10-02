mod htmx;
mod public;

use axum::{routing::get, Router};

use crate::{seo, state::AppState};

pub fn router() -> Router<AppState> {
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
        // Informational pages (about, IQAC, fees, ...) are looked up by path in the database,
        // so pages created in the admin dashboard work without a restart.
        .fallback(public::page_or_404)
}
