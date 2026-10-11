//! End-to-end tests through the real router.
//!
//! These build the same `AppState` and the same layered router the server
//! serves, then send requests through it with `tower`'s `oneshot` instead of a
//! socket. That exercises the whole stack — routing, the session layer, the
//! CSRF token, the Askama templates and the response headers — without needing
//! a listening port.
//!
//! `#[sqlx::test]` gives each test a migrated database of its own.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tower::ServiceExt;

use nss_college::{apply_layers, init_session_store, routes, state};

/// The public listener, wired exactly as `main` wires it.
fn public_app(pool: &PgPool, cookie_secure: bool) -> axum::Router {
    let state = state::AppState {
        db: pool.clone(),
        upload_dir: "uploads".into(),
        login_limiter: Arc::new(state::LoginLimiter::default_policy()),
    };
    apply_layers(routes::public_router(), pool, cookie_secure).with_state(state)
}

async fn get(app: axum::Router, uri: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("the router answered");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    (status, headers, String::from_utf8_lossy(&body).to_string())
}

#[sqlx::test]
async fn the_health_check_answers_and_carries_the_security_headers(pool: PgPool) {
    init_session_store(&pool).await;
    let (status, headers, body) = get(public_app(&pool, false), "/healthz").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "ok");

    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    };
    assert_eq!(header("x-content-type-options"), "nosniff");
    assert_eq!(header("x-frame-options"), "DENY");
    assert_eq!(header("referrer-policy"), "strict-origin-when-cross-origin");

    let csp = header("content-security-policy");
    assert!(!csp.is_empty(), "no Content-Security-Policy was sent");
    assert!(
        csp.contains("script-src 'self'"),
        "scripts must be same-origin only, got: {csp}"
    );
    assert!(
        !csp.contains("script-src 'self' 'unsafe-inline'")
            && !csp.contains("script-src 'unsafe-inline'"),
        "the policy still admits inline scripts: {csp}"
    );
    assert!(csp.contains("frame-ancestors 'none'"), "got: {csp}");
    assert!(csp.contains("object-src 'none'"), "got: {csp}");
    assert!(csp.contains("form-action 'self'"), "got: {csp}");

    // No HSTS on a plain-HTTP deployment: it would pin the browser to https.
    assert!(headers.get("strict-transport-security").is_none());
}

#[sqlx::test]
async fn hsts_is_sent_only_when_the_deployment_is_secure(pool: PgPool) {
    init_session_store(&pool).await;
    let (status, headers, _) = get(public_app(&pool, true), "/healthz").await;
    assert_eq!(status, StatusCode::OK);
    let hsts = headers
        .get("strict-transport-security")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        hsts.starts_with("max-age="),
        "an HTTPS deployment should pin HSTS, got: {hsts:?}"
    );
}

#[sqlx::test]
async fn robots_txt_keeps_the_private_areas_out_of_the_index(pool: PgPool) {
    init_session_store(&pool).await;
    let (status, _, body) = get(public_app(&pool, false), "/robots.txt").await;
    assert_eq!(status, StatusCode::OK);
    for private in ["/admin", "/hub", "/teacher", "/account"] {
        assert!(body.contains(private), "robots.txt does not disallow {private}");
    }
}

#[sqlx::test]
async fn the_login_page_renders_with_a_csrf_token(pool: PgPool) {
    init_session_store(&pool).await;
    let (status, _, body) = get(public_app(&pool, false), "/login").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("csrf_token"), "the form has no CSRF field");
    assert!(
        body.contains("/static/js/htmx.min.js"),
        "HTMX is not served from the local copy"
    );
    assert!(
        !body.contains("unpkg.com"),
        "a CDN script would be blocked by the Content-Security-Policy"
    );
}

#[sqlx::test]
async fn htmx_is_served_from_the_application_itself(pool: PgPool) {
    init_session_store(&pool).await;
    let (status, headers, body) = get(public_app(&pool, false), "/static/js/htmx.min.js").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.len() > 10_000,
        "the served HTMX file looks truncated ({} bytes)",
        body.len()
    );
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.contains("javascript"),
        "unexpected content type for the HTMX bundle: {content_type:?}"
    );
}

#[sqlx::test]
async fn an_unknown_path_under_the_site_is_a_404_not_a_redirect(pool: PgPool) {
    init_session_store(&pool).await;
    let (status, _, _) = get(public_app(&pool, false), "/no-such-page-here").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn a_signed_out_visitor_cannot_reach_a_dashboard(pool: PgPool) {
    init_session_store(&pool).await;
    // `/hub` is on the public listener but requires a student session; the
    // extractor must redirect to the sign-in page rather than render anything.
    let (status, _, _) = get(public_app(&pool, false), "/hub").await;
    assert!(
        status.is_redirection(),
        "an unauthenticated /hub request returned {status}"
    );
}

#[sqlx::test]
async fn the_management_router_builds_and_serves_the_login_page(pool: PgPool) {
    init_session_store(&pool).await;
    let state = state::AppState {
        db: pool.clone(),
        upload_dir: "uploads".into(),
        login_limiter: Arc::new(state::LoginLimiter::default_policy()),
    };
    let app = apply_layers(routes::management_router(), &pool, false).with_state(state);
    let (status, _, body) = get(app, "/login").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("csrf_token"));
}
