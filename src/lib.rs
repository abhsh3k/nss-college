//! The NSS College application, as a library.
//!
//! `main.rs` is only the binary: process setup, the two listeners and shutdown.
//! Everything else lives here so the integration tests under `tests/` can drive
//! the same router, migrations, services and authorization checks the server
//! uses, rather than a parallel copy of them.

pub mod auth;
pub mod cli;
pub mod config;
pub mod dates;
pub mod error;
pub mod layout;
pub mod models;
pub mod routes;
pub mod seo;
pub mod sections;
pub mod services;
pub mod shell;
pub mod site;
pub mod state;
pub mod uploads;
pub mod validate;

use std::time::Duration;

use axum::{
    http::{header, HeaderValue},
    Router,
};
use sqlx::{postgres::PgPoolOptions, PgPool};
use tower_http::{
    compression::CompressionLayer,
    services::ServeDir,
    set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};
use tower_sessions::{
    cookie::{time::Duration as CookieDuration, SameSite},
    Expiry, SessionManagerLayer,
};
use tower_sessions_sqlx_store::PostgresStore;

use crate::state::AppState;

/// Connect to PostgreSQL with the pool settings the server has always used.
pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(database_url)
        .await
}

/// The versioned migrations bundled into the binary.
pub fn migrator() -> sqlx::migrate::Migrator {
    sqlx::migrate!("./migrations")
}

/// Create the `tower-sessions` table if it is not there yet.
///
/// This is separate from the SQLx migrations (the session store owns its own
/// table), so both the server and the integration tests have to run it once
/// before the session layer can write anything.
pub async fn init_session_store(db: &PgPool) {
    PostgresStore::new(db.clone())
        .migrate()
        .await
        .expect("could not create the sessions table");
}

/// Build the session store and apply the shared middleware: session, security
/// headers, compression and tracing.
///
/// The security headers are applied to every response on both listeners. They
/// are `if_not_present`, so a handler that sets a more specific value keeps it.
///
/// `cookie_secure` doubles as "this deployment is behind HTTPS": the session
/// cookie's `Secure` flag and HSTS are both only correct over TLS, and sending
/// HSTS from a plain-HTTP development server would pin the browser to https://
/// for the host.
pub fn apply_layers(
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

    let mut router = router
        // App assets live on both listeners. User uploads are served through an
        // authorization-aware route rather than a raw directory mount.
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
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CONTENT_SECURITY_POLICY),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            axum::http::HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("geolocation=(), microphone=(), camera=()"),
        ));

    if cookie_secure {
        router = router.layer(SetResponseHeaderLayer::if_not_present(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        ));
    }

    router
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
}

/// The site's Content-Security-Policy.
///
/// Scripts are `'self'` only: HTMX is served from `/static/js/htmx.min.js`
/// rather than a CDN, and every inline `on*` handler was replaced by a
/// `data-*` hook handled in `app.js` precisely so no `'unsafe-inline'` is
/// needed for scripts. That is the directive that actually stops an injected
/// `<script>` or `onerror=`.
///
/// `style-src` keeps `'unsafe-inline'` because a few templates set a CSS custom
/// property or a computed width on an element (the homepage carousel and the
/// attendance bars); a style attribute cannot be moved into a class without
/// losing the per-row value. The allowed external origins are the Google Fonts
/// stylesheet and font files.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; \
     base-uri 'self'; \
     object-src 'none'; \
     frame-ancestors 'none'; \
     form-action 'self'; \
     script-src 'self'; \
     style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; \
     font-src 'self' https://fonts.gstatic.com data:; \
     img-src 'self' data:; \
     connect-src 'self'";
