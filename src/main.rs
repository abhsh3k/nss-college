mod config;
mod error;
mod models;
mod routes;
mod seo;
mod services;
mod state;

use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use tower_http::{compression::CompressionLayer, services::ServeDir, trace::TraceLayer};
use tracing_subscriber::EnvFilter;

use crate::state::AppState;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("nss_college=debug,tower_http=info")),
        )
        .init();

    let cfg = config::Config::from_env();

    let db = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&cfg.database_url)
        .await
        .expect("could not connect to PostgreSQL. Is it running? Try: docker compose up -d");

    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .expect("database migrations failed");
    tracing::info!("database ready");

    let state = AppState { db };

    // App assets (css/js/img) and user uploads live in separate directories.
    let app = routes::router()
        .nest_service("/static", ServeDir::new("static"))
        .nest_service("/uploads", ServeDir::new(&cfg.upload_dir))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(cfg.addr)
        .await
        .expect("failed to bind address");
    tracing::info!("listening on http://{}", cfg.addr);
    axum::serve(listener, app).await.expect("server error");
}
