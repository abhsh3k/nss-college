//! The NSS College server binary.
//!
//! This file is deliberately thin: it turns the environment into a pool and a
//! configuration, runs the migrations, then hands two routers to the listeners.
//! The application itself lives in the library crate (`src/lib.rs`), which the
//! integration tests also use.

use std::sync::Arc;

use tracing_subscriber::EnvFilter;

use nss_college::{apply_layers, cli, config, init_session_store, migrator, routes, services, site, state};

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

    let db = nss_college::connect(&cfg.database_url)
        .await
        .expect("could not connect to PostgreSQL. Is it running? Try: docker compose up -d");

    if let Err(e) = migrator().run(&db).await {
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
        // Credentials are never written to the log: they go to the operator's
        // file (SEED_CREDENTIALS_FILE) or nowhere at all. Logs get collected,
        // shipped and retained; a password in them is a leaked password.
        match cli::seed_demo_users(&db).await {
            Ok(lines) => match std::env::var("SEED_CREDENTIALS_FILE") {
                Ok(path) if !path.trim().is_empty() => {
                    let path = std::path::PathBuf::from(path);
                    match cli::write_credentials(&lines, &path) {
                        Ok(()) => tracing::warn!(
                            path = %path.display(),
                            accounts = lines.len(),
                            "SEED: demo accounts ready; credentials written to the operator file (move them to a password manager and delete the file)"
                        ),
                        Err(msg) => tracing::error!("SEED: could not write the credentials file: {msg}"),
                    }
                }
                _ => tracing::warn!(
                    accounts = lines.len(),
                    "SEED: demo accounts ready, but SEED_CREDENTIALS_FILE is unset so the generated passwords were discarded; re-run `cargo run -- seed-demo-users --credentials-file <path>`"
                ),
            },
            Err(msg) => tracing::error!("SEED: demo user seeding failed: {msg}"),
        }
    }

    // Command-line helpers (e.g. creating the first admin) run instead of the server.
    let cli_args: Vec<String> = std::env::args().skip(1).collect();
    if cli::run(&cli_args, &db).await {
        return;
    }

    init_session_store(&db).await;

    // The error pages and the dashboard sidebar have no database handle of
    // their own, so they read the cached copy of these settings.
    if let Err(e) = site::get(&db).await {
        tracing::warn!(error = ?e, "could not load site settings; falling back to defaults");
    }
    site::spawn_refresher(db.clone());

    // Housekeeping: expired sessions and staged import batches (and the raw
    // text they hold), plus the recurring backup. It keeps running even if the
    // backup cannot (`pg_dump` missing) — those are independent failures.
    let backup_cfg = services::backup::BackupConfig::from_env();
    if backup_cfg.enabled {
        tracing::info!(
            dir = %backup_cfg.dir.display(),
            interval_hours = backup_cfg.interval_hours,
            keep = backup_cfg.keep,
            "scheduled backups enabled"
        );
    } else {
        tracing::warn!("scheduled backups are disabled (BACKUP_ENABLED is off)");
    }
    services::maintenance::spawn(db.clone(), backup_cfg);

    let state = state::AppState {
        db,
        upload_dir: cfg.upload_dir.clone(),
        login_limiter: Arc::new(state::LoginLimiter::default_policy()),
    };

    // App assets live on both listeners. User uploads are served through an
    // authorization-aware route rather than a raw directory mount.
    let public_app =
        apply_layers(routes::public_router(), &state.db, cfg.cookie_secure).with_state(state.clone());
    let management_app =
        apply_layers(routes::management_router(), &state.db, cfg.cookie_secure).with_state(state);

    let public_listener = tokio::net::TcpListener::bind(cfg.addr)
        .await
        .expect("failed to bind public address");
    let management_listener = tokio::net::TcpListener::bind(cfg.management_addr)
        .await
        .expect("failed to bind management address");
    tracing::info!(address = %cfg.addr, "public listener ready");
    tracing::info!(address = %cfg.management_addr, "loopback management listener ready");

    // `into_make_service_with_connect_info` exposes the peer address to the
    // sign-in limiter. The forwarded headers are deliberately not consulted:
    // a client can set those, but not the socket it connects from.
    tokio::try_join!(
        axum::serve(
            public_listener,
            public_app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        ),
        axum::serve(
            management_listener,
            management_app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        ),
    )
    .expect("server error");
}
