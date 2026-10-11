//! End-to-end tests through the real router.
//!
//! These build the same `AppState` and the same layered router the server
//! serves, then send requests through it with `tower`'s `oneshot` instead of a
//! socket. That exercises the whole stack — routing, the session layer, the
//! CSRF token, the Askama templates and the response headers — without needing
//! a listening port.
//!
//! `#[sqlx::test]` gives each test a migrated database of its own.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tower::ServiceExt;

use nss_college::auth::password;
use nss_college::services::{appointments, content};
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
async fn re_authentication_requires_a_signed_in_session(pool: PgPool) {
    init_session_store(&pool).await;
    // The re-authentication page is guarded by `AuthUser`, so a signed-out
    // visitor is sent to sign in; the page itself must never render.
    let (status, _, _) = get(public_app(&pool, false), "/account/reauth").await;
    assert!(
        status.is_redirection(),
        "an unauthenticated /account/reauth returned {status}"
    );
}

// ---------- Authorization through the real router ----------

/// The management listener, wired exactly as `main` wires it.
fn management_app(pool: &PgPool) -> axum::Router {
    let state = state::AppState {
        db: pool.clone(),
        upload_dir: "uploads".into(),
        login_limiter: Arc::new(state::LoginLimiter::default_policy()),
    };
    apply_layers(routes::management_router(), pool, false).with_state(state)
}

fn cookie_of(headers: &axum::http::HeaderMap) -> String {
    for v in headers.get_all("set-cookie") {
        let s = v.to_str().unwrap_or("");
        if let Some(rest) = s.strip_prefix("nss_session=") {
            return format!("nss_session={}", rest.split(';').next().unwrap_or(""));
        }
    }
    String::new()
}

fn extract_csrf(body: &str) -> String {
    let idx = body.find("name=\"csrf_token\"").expect("a csrf_token field");
    let rest = &body[idx..];
    let vstart = rest.find("value=\"").expect("a csrf value") + 7;
    let rest = &rest[vstart..];
    let vend = rest.find('"').expect("closing quote");
    rest[..vend].to_string()
}

async fn get_with(app: axum::Router, uri: &str, cookie: &str) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

async fn post_form(
    app: axum::Router,
    uri: &str,
    cookie: &str,
    body: String,
) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/x-www-form-urlencoded")
                .header("cookie", cookie)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

/// Sign in through the real login route and return the session cookie.
async fn sign_in(app: axum::Router, identifier: &str, pass: &str) -> String {
    let (_, headers, body) = get(app.clone(), "/login").await;
    let pre = cookie_of(&headers);
    let csrf = extract_csrf(&body);
    let form = format!("identifier={identifier}&password={pass}&csrf_token={csrf}");
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 0))))
                .header("content-type", "application/x-www-form-urlencoded")
                .header("cookie", &pre)
                .body(Body::from(form))
                .unwrap(),
        )
        .await
        .unwrap();
    let after = cookie_of(response.headers());
    if after.is_empty() {
        pre
    } else {
        after
    }
}

async fn make_user(pool: &PgPool, email: &str, role: &str, pass: &str) -> i64 {
    let hash = password::hash(pass).expect("hash");
    sqlx::query_scalar(
        "INSERT INTO users (email, password_hash, role, must_change_password)
         VALUES ($1, $2, $3, false) RETURNING id",
    )
    .bind(email)
    .bind(hash)
    .bind(role)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn department(pool: &PgPool, slug: &str) -> i64 {
    sqlx::query_scalar("INSERT INTO departments (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn programme(pool: &PgPool, department_id: i64, slug: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO programmes (department_id, slug, name, level) VALUES ($1, $2, $2, 'ug') RETURNING id",
    )
    .bind(department_id)
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test]
async fn an_hod_cannot_touch_another_departments_programme(pool: PgPool) {
    init_session_store(&pool).await;
    let dept_a = department(&pool, "alpha").await;
    let dept_b = department(&pool, "beta").await;
    let _prog_a = programme(&pool, dept_a, "alpha-ug").await;
    let prog_b = programme(&pool, dept_b, "beta-ug").await;

    let admin = make_user(&pool, "admin@example.com", "admin", "Password123!").await;
    let hod = make_user(&pool, "hod-a@example.com", "faculty", "Password123!").await;
    sqlx::query("INSERT INTO faculty (user_id, department_id, name, is_hod) VALUES ($1, $2, 'HOD A', true)")
        .bind(hod)
        .bind(dept_a)
        .execute(&pool)
        .await
        .unwrap();
    appointments::grant_head(&pool, hod, dept_a, false, None, admin, "test")
        .await
        .unwrap();

    let app = management_app(&pool);
    let cookie = sign_in(app.clone(), "hod-a@example.com", "Password123!").await;
    assert!(!cookie.is_empty(), "sign-in did not return a session");

    // A CSRF token from the HOD's own marks page.
    let (status, body) = get_with(app.clone(), "/admin/marks", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    let csrf = extract_csrf(&body);

    // Filing marks against the *other* department's programme is refused.
    let form = format!(
        "csrf_token={csrf}&programme={prog_b}&semester=1&course=1&kind=coursework&assessment=internal&exam_name=&max=100"
    );
    let (status, _) = post_form(app, "/admin/marks/save", &cookie, form).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "an HOD must not file marks for another department"
    );
}

#[sqlx::test]
async fn a_teacher_with_no_appointment_cannot_reach_the_academic_tools(pool: PgPool) {
    init_session_store(&pool).await;
    let dept = department(&pool, "alpha").await;
    let teacher = make_user(&pool, "teacher@example.com", "faculty", "Password123!").await;
    sqlx::query("INSERT INTO faculty (user_id, department_id, name, is_hod) VALUES ($1, $2, 'Teacher', false)")
        .bind(teacher)
        .bind(dept)
        .execute(&pool)
        .await
        .unwrap();

    let app = management_app(&pool);
    let cookie = sign_in(app.clone(), "teacher@example.com", "Password123!").await;
    let (status, _) = get_with(app, "/admin/marks", &cookie).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a teacher without an appointment must not open the academic tools"
    );
}

#[sqlx::test]
async fn the_it_admin_may_act_for_any_programme(pool: PgPool) {
    init_session_store(&pool).await;
    let dept_b = department(&pool, "beta").await;
    let prog_b = programme(&pool, dept_b, "beta-ug").await;
    let _admin = make_user(&pool, "admin@example.com", "admin", "Password123!").await;

    let app = management_app(&pool);
    let cookie = sign_in(app.clone(), "admin@example.com", "Password123!").await;
    let (status, body) = get_with(app.clone(), &format!("/admin/marks?programme={prog_b}"), &cookie).await;
    assert_eq!(status, StatusCode::OK);
    let csrf = extract_csrf(&body);

    // The admin is not refused for the other department's programme (the
    // submission may still be redirected for a missing course, but never 403).
    let form = format!(
        "csrf_token={csrf}&programme={prog_b}&semester=1&course=1&kind=coursework&assessment=internal&exam_name=&max=100"
    );
    let (status, _) = post_form(app, "/admin/marks/save", &cookie, form).await;
    assert_ne!(status, StatusCode::FORBIDDEN);
}

/// Post a `multipart/form-data` body, the shape the content forms use.
async fn post_multipart(
    app: axum::Router,
    uri: &str,
    cookie: &str,
    fields: &[(&str, &str)],
) -> (StatusCode, String) {
    const BOUNDARY: &str = "----nssTestBoundary";
    let mut body = String::new();
    for (name, value) in fields {
        body.push_str(&format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        ));
    }
    body.push_str(&format!("--{BOUNDARY}--\r\n"));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={BOUNDARY}"),
                )
                .header("cookie", cookie)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

#[sqlx::test]
async fn the_notice_form_schedules_a_publish_time(pool: PgPool) {
    init_session_store(&pool).await;
    let _office = make_user(&pool, "office@example.com", "staff", "Password123!").await;
    let app = management_app(&pool);
    let cookie = sign_in(app.clone(), "office@example.com", "Password123!").await;

    let (status, body) = get_with(app.clone(), "/admin/notices/new", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("name=\"publish_at\""),
        "the notice form has no publish time"
    );
    assert!(
        body.contains("name=\"unpublish_at\""),
        "the notice form has no expiry"
    );
    let csrf = extract_csrf(&body);

    let (status, body) = post_multipart(
        app,
        "/admin/notices",
        &cookie,
        &[
            ("csrf_token", &csrf),
            ("title", "Scheduled for later"),
            ("category", "Notice"),
            ("body", "Body"),
            ("audience", "public"),
            ("status", "scheduled"),
            ("publish_at", "2099-01-01T09:00"),
        ],
    )
    .await;
    assert!(
        status.is_redirection(),
        "the save should redirect, got {status}: {body}"
    );

    // The time the editor typed is stored as the publish time, and the public
    // list hides the row until that time arrives.
    let stored: String = sqlx::query_scalar(
        "SELECT to_char(published_at, 'YYYY-MM-DD HH24:MI') FROM notices WHERE title = 'Scheduled for later'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stored, "2099-01-01 09:00");

    let titles: Vec<String> = content::notices(&pool, 50)
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.title)
        .collect();
    assert!(
        !titles.contains(&"Scheduled for later".to_string()),
        "a notice scheduled for the future must stay off the public list"
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
