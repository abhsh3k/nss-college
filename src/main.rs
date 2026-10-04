mod auth;
mod cli;
mod config;
mod error;
mod models;
mod routes;
mod seo;
mod services;
mod shell;
mod state;
mod uploads;

use std::time::Duration;

use axum::http::{header, HeaderValue};
use sqlx::postgres::PgPoolOptions;
use tower_http::{
    compression::CompressionLayer, services::ServeDir, set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};
use tower_sessions::{cookie::{time::Duration as CookieDuration, SameSite}, Expiry, SessionManagerLayer};
use tower_sessions_sqlx_store::PostgresStore;
use tracing_subscriber::EnvFilter;

use crate::state::AppState;

/// Lowercase hex, for readable checksum logging.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

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

    let migrator = sqlx::migrate!("./migrations");

    // TEMPORARY one-shot repair, enabled by MIGRATION_CHECKSUM_REPAIR=<version>.
    //
    // A migration file's recorded checksum can go stale when the file was
    // applied from a checkout with different line endings, which makes every
    // later start fail with "was previously applied but has been modified"
    // even though the schema is identical. This re-records the checksum from
    // the migration the binary actually embeds, so the value can never be
    // mistyped. Remove once the production row has been corrected.
    if let Ok(spec) = std::env::var("MIGRATION_CHECKSUM_REPAIR") {
        let version: i64 = spec.trim().parse().unwrap_or_else(|_| {
            panic!("MIGRATION_CHECKSUM_REPAIR must be a migration version, got {spec:?}")
        });
        let expected = migrator
            .iter()
            .find(|m| m.version == version)
            .map(|m| m.checksum.clone())
            .unwrap_or_else(|| panic!("no embedded migration with version {version}"));

        let current: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version = $1")
                .bind(version)
                .fetch_optional(&db)
                .await
                .expect("could not read the recorded migration checksum");

        match current {
            Some(current) if current == expected.as_ref() => {
                tracing::warn!("checksum repair: v{version} already correct, nothing to do");
            }
            Some(current) => {
                tracing::warn!(
                    "checksum repair: v{version} recorded {:?}, embedding {:?}; updating",
                    hex(&current),
                    hex(expected.as_ref())
                );
                sqlx::query("UPDATE _sqlx_migrations SET checksum = $2 WHERE version = $1")
                    .bind(version)
                    .bind(expected.as_ref())
                    .execute(&db)
                    .await
                    .expect("could not update the migration checksum");
                tracing::warn!("checksum repair: v{version} updated");
            }
            None => tracing::warn!("checksum repair: v{version} is not recorded, nothing to do"),
        }
    }

    if let Err(e) = migrator.run(&db).await {
        // A version mismatch is invisible without knowing what the database
        // already has, so print the applied set before giving up.
        tracing::error!("database migrations failed: {e}");
        let applied: Vec<(i64, String, bool, Option<String>)> = sqlx::query_as(
            "SELECT version, description, success, encode(checksum, 'hex')
               FROM _sqlx_migrations ORDER BY version",
        )
        .fetch_all(&db)
        .await
        .unwrap_or_default();
        tracing::error!("database already has {} applied migration(s):", applied.len());
        for (version, description, success, checksum) in &applied {
            tracing::error!("  v{version} {description} success={success} checksum={checksum:?}");
        }
        panic!("database migrations failed: {e}");
    }
    tracing::info!("database ready");

    // Command-line helpers (e.g. creating the first admin) run instead of the server.
    let cli_args: Vec<String> = std::env::args().skip(1).collect();
    if cli::run(&cli_args, &db).await {
        return;
    }

    let session_store = PostgresStore::new(db.clone());
    session_store
        .migrate()
        .await
        .expect("could not create the sessions table");
    let session_layer = SessionManagerLayer::new(session_store)
        .with_name("nss_session")
        .with_secure(cfg.cookie_secure)
        .with_same_site(SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(CookieDuration::hours(8)));

    let state = AppState {
        db,
        upload_dir: cfg.upload_dir.clone(),
    };

    // App assets (css/js/img) and user uploads live in separate directories.
    let app = routes::router()
        .nest_service("/static", ServeDir::new("static"))
        .nest_service("/uploads", ServeDir::new(&cfg.upload_dir))
        .layer(session_layer)
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(cfg.addr)
        .await
        .expect("failed to bind address");
    tracing::info!("listening on http://{}", cfg.addr);
    axum::serve(listener, app).await.expect("server error");
}
