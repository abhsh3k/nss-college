mod auth;
mod cli;
mod config;
mod error;
mod layout;
mod models;
mod routes;
mod seo;
mod sections;
mod site;
mod services;
mod shell;
mod state;
mod uploads;

use std::time::Duration;

use axum::{http::{header, HeaderValue}, Router};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tower_http::{
    compression::CompressionLayer, services::ServeDir, set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};
use tower_sessions::{cookie::{time::Duration as CookieDuration, SameSite}, Expiry, SessionManagerLayer};
use tower_sessions_sqlx_store::PostgresStore;
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

    let migrator = sqlx::migrate!("./migrations");

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

    // Seed the demo accounts from inside the deployment. The only other way in
    // is `railway ssh`, which needs a key registered against the account, so
    // this is driven by an environment variable that is unset once done.
    if std::env::var("SEED_DEMO_USERS")
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
    {
        match cli::seed_demo_users(&db).await {
            Ok(_) => tracing::warn!("demo accounts seeded; retrieve credentials through the operator-only bootstrap output"),
            Err(msg) => tracing::error!("SEED: demo user seeding failed: {msg}"),
        }
    }

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

    // The error pages and the dashboard sidebar have no database handle of
    // their own, so they read the cached copy of these settings.
    if let Err(e) = site::get(&db).await {
        tracing::warn!(error = ?e, "could not load site settings; falling back to defaults");
    }
    site::spawn_refresher(db.clone());

    let state = AppState {
        db,
        upload_dir: cfg.upload_dir.clone(),
    };

    // App assets live on both listeners. User uploads are served through an
    // authorization-aware route rather than a raw directory mount.
    let public_app = apply_layers(routes::public_router(), &state.db, cfg.cookie_secure)
        .with_state(state.clone());
    let management_app = apply_layers(
        routes::management_router(),
        &state.db,
        cfg.cookie_secure,
    )
    .with_state(state);

    let public_listener = tokio::net::TcpListener::bind(cfg.addr)
        .await
        .expect("failed to bind public address");
    let management_listener = tokio::net::TcpListener::bind(cfg.management_addr)
        .await
        .expect("failed to bind management address");
    tracing::info!(address = %cfg.addr, "public listener ready");
    tracing::info!(address = %cfg.management_addr, "loopback management listener ready");

    tokio::try_join!(
        axum::serve(public_listener, public_app),
        axum::serve(management_listener, management_app),
    )
    .expect("server error");
}

fn apply_layers(
    router: Router<AppState>,
    db: &PgPool,
    cookie_secure: bool,
) -> Router<AppState> {
    let session_store = PostgresStore::new(db.clone());
    let session_layer = SessionManagerLayer::new(session_store)
        .with_name("nss_session")
        .with_secure(cookie_secure)
        .with_same_site(SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(CookieDuration::hours(8)));

    router
        .nest_service("/static", ServeDir::new("static"))
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
}
