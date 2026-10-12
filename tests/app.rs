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
use nss_college::services::{appointments, content, courses};
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
    let (status, _, body) = get_with_headers(app, uri, cookie).await;
    (status, body)
}

/// The same GET, keeping the headers so a download's `Content-Disposition` and
/// a redirect's `Location` can be read.
async fn get_with_headers(
    app: axum::Router,
    uri: &str,
    cookie: &str,
) -> (StatusCode, axum::http::HeaderMap, String) {
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
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8_lossy(&bytes).to_string())
}

async fn post_form(
    app: axum::Router,
    uri: &str,
    cookie: &str,
    body: String,
) -> (StatusCode, String) {
    let (status, _, body) = post_form_with_headers(app, uri, cookie, body).await;
    (status, body)
}

/// The same POST, keeping the headers so a redirect's `Location` can be read.
async fn post_form_with_headers(
    app: axum::Router,
    uri: &str,
    cookie: &str,
    body: String,
) -> (StatusCode, axum::http::HeaderMap, String) {
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
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8_lossy(&bytes).to_string())
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

/// A student profile (and the id the hub scopes everything to), returning the
/// `students.id`.
async fn student(pool: &PgPool, user_id: i64, admission_no: &str, programme_id: i64) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO students (user_id, admission_no, name, programme_id, batch_year)
         VALUES ($1, $2, $3, $4, 2026) RETURNING id",
    )
    .bind(user_id)
    .bind(admission_no)
    .bind(format!("Student {admission_no}"))
    .bind(programme_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A course of a programme, in one semester, as the exam and marks pickers
/// offer it. Returns the `courses.id`.
async fn course_row(pool: &PgPool, programme_id: i64, code: &str, semester: i32) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO courses (programme_id, code, title, semester, credits)
         VALUES ($1, $2, $2, $3, 4) RETURNING id",
    )
    .bind(programme_id)
    .bind(code)
    .bind(semester)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A faculty row, returning its id (the value `timetable_entries.faculty_id`
/// and `faculty.id` both use).
async fn faculty_row(pool: &PgPool, user_id: i64, department_id: i64) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO faculty (user_id, department_id, name, is_hod) VALUES ($1, $2, 'Teacher', false) RETURNING id",
    )
    .bind(user_id)
    .bind(department_id)
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

#[sqlx::test]
async fn a_course_wise_sheet_files_through_the_review_with_its_own_maximum(pool: PgPool) {
    init_session_store(&pool).await;
    let dept = department(&pool, "alpha").await;
    let prog = programme(&pool, dept, "alpha-ug").await;
    let bca101 = course_row(&pool, prog, "BCA101", 1).await;
    let bca102 = course_row(&pool, prog, "BCA102", 1).await;
    let s1 = make_user(&pool, "s1@example.com", "student", "Password123!").await;
    let s2 = make_user(&pool, "s2@example.com", "student", "Password123!").await;
    let st1 = student(&pool, s1, "2501", prog).await;
    sqlx::query("UPDATE students SET prn = 'PRN2501' WHERE id = $1")
        .bind(st1)
        .execute(&pool)
        .await
        .unwrap();
    let _st2 = student(&pool, s2, "2502", prog).await;
    let _admin = make_user(&pool, "admin@example.com", "admin", "Password123!").await;

    let app = management_app(&pool);
    let cookie = sign_in(app.clone(), "admin@example.com", "Password123!").await;

    // The blank form: the key, the name and every course of the semester, one
    // line per student, with the mark cells empty.
    let template = format!("/admin/marks/template.csv?programme={prog}&semester=1&kind=internal");
    let (status, headers, body) = get_with_headers(app.clone(), &template, &cookie).await;
    assert_eq!(status, StatusCode::OK);
    let disposition = headers
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        disposition.contains("attachment"),
        "the form should download, got: {disposition:?}"
    );
    assert_eq!(
        body.lines().next().unwrap_or("").trim_end(),
        "admission_no,name,BCA101,BCA102"
    );
    assert!(
        body.contains("2501,Student 2501,,"),
        "the form should pre-fill the class with blank marks, got: {body}"
    );
    assert!(body.contains("2502,Student 2502,,"), "got: {body}");

    // With a course open, the same download covers just that course.
    let (status, _, body) =
        get_with_headers(app.clone(), &format!("{template}&course={bca101}"), &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.lines().next().unwrap_or("").trim_end(),
        "admission_no,name,BCA101"
    );

    // Upload the filled-in sheet: it stages for review, it does not file.
    let (_, page) = get_with(
        app.clone(),
        &format!(
            "/admin/marks?programme={prog}&semester=1&course={bca101}&kind=internal&assessment=internal&exam_name=&max="
        ),
        &cookie,
    )
    .await;
    let csrf = extract_csrf(&page);

    // The marks window itself offers both upload shapes, and the blank form for
    // the whole semester or — since a course is open — for that course alone.
    assert!(
        page.contains("/marks/import/preview"),
        "the marks window should still offer the one-row shape"
    );
    assert!(
        page.contains("/marks/import/courses/preview"),
        "the marks window should offer the course-wise shape"
    );
    assert!(
        page.contains(&format!(
            "/marks/template.csv?programme={prog}&amp;semester=1&amp;kind=internal\""
        )),
        "the marks window should offer the blank form for the semester"
    );
    assert!(
        page.contains(&format!(
            "/marks/template.csv?programme={prog}&amp;semester=1&amp;kind=internal&amp;course={bca101}"
        )),
        "with a course open, the blank form should be offered for that course alone"
    );

    let programme = prog.to_string();
    let sheet = "admission_no,name,BCA101,BCA102\n2501,Asha Rao,42,38\n2502,Bhavna,,36\n";
    let (status, headers, _) = post_multipart_file(
        app.clone(),
        "/admin/marks/import/courses/preview",
        &cookie,
        &[
            ("csrf_token", &csrf),
            ("programme", &programme),
            ("semester", "1"),
            ("kind", "internal"),
            ("assessment", "internal"),
            ("exam_name", ""),
            ("max", ""),
        ],
        "coursewise.csv",
        sheet,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::SEE_OTHER,
        "the upload should stage for review, not file"
    );
    let preview = headers
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        preview.contains("/admin/exams/results/preview?batch="),
        "unexpected redirect: {preview}"
    );
    let batch: i64 = preview
        .split("batch=")
        .nth(1)
        .unwrap_or("")
        .parse()
        .unwrap_or(0);
    assert!(batch > 0, "no batch was staged: {preview}");

    let (status, page) = get_with(app.clone(), &preview, &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        page.contains("Asha Rao"),
        "the review should show the name the file carried"
    );
    for course in [bca101, bca102] {
        assert!(
            page.contains(&format!("name=\"max_{course}\"")),
            "the review should offer a maximum for every course"
        );
    }

    // Committing an import asks for the password again, like every other
    // critical write.
    let (status, page) = get_with(app.clone(), "/account/reauth", &cookie).await;
    assert_eq!(status, StatusCode::OK, "the re-entry page should render");
    let (_, _, _) = post_form_with_headers(
        app.clone(),
        "/account/reauth",
        &cookie,
        format!(
            "csrf_token={}&password=Password123!&next=%2Fadmin%2Fmarks",
            extract_csrf(&page)
        ),
    )
    .await;

    // Committing files the rows, each course out of the maximum reviewed.
    let commit = format!("csrf_token={csrf}&batch={batch}&max_{bca101}=50&max_{bca102}=40");
    let (status, _, _) = post_form_with_headers(app, "/admin/exams/results/commit", &cookie, commit).await;
    assert!(status.is_redirection(), "committing redirects, got {status}");

    let filed: Vec<(String, String, f64, f64)> = sqlx::query_as(
        r#"SELECT st.admission_no, c.code, m.marks_obtained::float8, m.max_marks::float8
             FROM marks m
             JOIN students st ON st.id = m.student_id
             JOIN courses  c  ON c.id  = m.course_id
            WHERE m.semester = 1 AND m.exam_kind = 'internal'
            ORDER BY st.admission_no, c.code"#,
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        filed,
        vec![
            ("2501".to_string(), "BCA101".to_string(), 42.0, 50.0),
            ("2501".to_string(), "BCA102".to_string(), 38.0, 40.0),
            ("2502".to_string(), "BCA102".to_string(), 36.0, 40.0),
        ],
        "a blank cell files nothing, and every row takes the reviewed maximum"
    );
}

#[sqlx::test]
async fn a_one_row_per_result_sheet_carries_its_name_and_maximum_into_the_review(pool: PgPool) {
    init_session_store(&pool).await;
    let dept = department(&pool, "alpha").await;
    let prog = programme(&pool, dept, "alpha-ug").await;
    let bca101 = course_row(&pool, prog, "BCA101", 1).await;
    let s1 = make_user(&pool, "s1@example.com", "student", "Password123!").await;
    student(&pool, s1, "2501", prog).await;
    let _admin = make_user(&pool, "admin@example.com", "admin", "Password123!").await;

    let app = management_app(&pool);
    let cookie = sign_in(app.clone(), "admin@example.com", "Password123!").await;
    let (_, page) = get_with(
        app.clone(),
        &format!("/admin/marks?programme={prog}&semester=1&course={bca101}&kind=internal&assessment=internal&exam_name=&max="),
        &cookie,
    )
    .await;
    let csrf = extract_csrf(&page);
    let programme = prog.to_string();

    // The name column is a label beside the key; the row still files on the
    // admission number. The file's own maximum travels into the review box.
    let csv = "admission_no,name,course_code,marks_obtained,maxmarks\n2501,Asha Rao,BCA101,42,50\n";
    let (status, headers, _) = post_multipart_file(
        app.clone(),
        "/admin/marks/import/preview",
        &cookie,
        &[
            ("csrf_token", &csrf),
            ("programme", &programme),
            ("semester", "1"),
            ("kind", "internal"),
            ("assessment", "internal"),
            ("exam_name", ""),
        ],
        "results.csv",
        csv,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "the upload should stage for review");
    let preview = headers
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let (status, page) = get_with(app, &preview, &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Asha Rao"), "the review should show the name label");
    assert!(
        page.contains(&format!("name=\"max_{bca101}\"")),
        "the review should offer a maximum for the course"
    );
    assert!(
        page.contains("value=\"50\""),
        "the maximum box should start from the file's own value"
    );
}

#[sqlx::test]
async fn root_authority_belongs_to_the_super_admin_alone(pool: PgPool) {
    init_session_store(&pool).await;
    let it = make_user(&pool, "admin@example.com", "admin", "Password123!").await;
    make_user(&pool, "root@example.com", "super_admin", "Password123!").await;
    let app = management_app(&pool);

    // The IT administrator works in the management area, but does not hold root
    // authority: it can neither open the screen nor promote itself through it.
    let it_cookie = sign_in(app.clone(), "admin@example.com", "Password123!").await;
    let (status, _) = get_with(app.clone(), "/admin/roles", &it_cookie).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "root authority is not the IT admin's");
    let (status, _) = post_form(
        app.clone(),
        &format!("/admin/roles/{it}/role"),
        &it_cookie,
        "csrf_token=x&role=super_admin".to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "an IT admin must not promote itself");

    // The Super Admin reaches the screen, and a change made there is real.
    let root_cookie = sign_in(app.clone(), "root@example.com", "Password123!").await;
    let (status, body) = get_with(app.clone(), "/admin/roles", &root_cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Roles and recovery"));
    assert!(body.contains("admin@example.com"), "the account table lists the IT admin");
    let csrf = extract_csrf(&body);

    // These writes need a fresh password check, exactly like the other
    // critical operations.
    let (status, _) = post_form(
        app.clone(),
        "/account/reauth",
        &root_cookie,
        format!("csrf_token={csrf}&password=Password123!&next=%2Fadmin%2Froles"),
    )
    .await;
    assert!(status.is_redirection(), "re-entry should redirect, got {status}");

    let (status, _) = post_form(
        app.clone(),
        &format!("/admin/roles/{it}/role"),
        &root_cookie,
        format!("csrf_token={csrf}&role=super_admin"),
    )
    .await;
    assert!(status.is_redirection(), "a role change redirects, got {status}");
    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE id = $1")
        .bind(it)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(role, "super_admin", "the promotion took effect");

    // The account is demoted again: with two root accounts, the guard allows it.
    let (status, _) = post_form(
        app,
        &format!("/admin/roles/{it}/role"),
        &root_cookie,
        format!("csrf_token={csrf}&role=admin"),
    )
    .await;
    assert!(status.is_redirection(), "a role change redirects, got {status}");
    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE id = $1")
        .bind(it)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(role, "admin");
}

/// Post a `multipart/form-data` body carrying one uploaded file, the shape the
/// import forms use. Returns the headers too, so a redirect can be followed.
async fn post_multipart_file(
    app: axum::Router,
    uri: &str,
    cookie: &str,
    fields: &[(&str, &str)],
    filename: &str,
    contents: &str,
) -> (StatusCode, axum::http::HeaderMap, String) {
    const BOUNDARY: &str = "----nssTestBoundary";
    let mut body = String::new();
    for (name, value) in fields {
        body.push_str(&format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        ));
    }
    body.push_str(&format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"csv_file\"; filename=\"{filename}\"\r\nContent-Type: text/csv\r\n\r\n{contents}\r\n"
    ));
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
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8_lossy(&bytes).to_string())
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
async fn a_timetable_period_can_be_left_for_a_teacher_to_be_assigned(pool: PgPool) {
    init_session_store(&pool).await;
    let dept = department(&pool, "alpha").await;
    let prog = programme(&pool, dept, "alpha-ug").await;
    let course_id: i64 = sqlx::query_scalar(
        "INSERT INTO courses (programme_id, department_id, code, title, credits, category, semester)
         VALUES ($1, $2, 'ALP101', 'Alpha course', 4, 'MAJOR', 3) RETURNING id",
    )
    .bind(prog)
    .bind(dept)
    .fetch_one(&pool)
    .await
    .unwrap();
    let _admin = make_user(&pool, "admin@example.com", "admin", "Password123!").await;
    let app = management_app(&pool);
    let cookie = sign_in(app.clone(), "admin@example.com", "Password123!").await;

    // The form offers the explicit "to be assigned" choice instead of forcing a
    // teacher for every period.
    let (status, body) = get_with(
        app.clone(),
        &format!("/admin/timetable?programme={prog}&semester=3"),
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("name=\"teacher_tbd\""),
        "the timetable form has no teacher-to-be-assigned option"
    );
    let csrf = extract_csrf(&body);

    let form = format!(
        "csrf_token={csrf}&programme_id={prog}&semester=3&weekday=1&start_at=09:00&end_at=10:00&course_id={course_id}&faculty_id=&teacher_tbd=1&room="
    );
    let (status, _) = post_form(app.clone(), "/admin/timetable/slots", &cookie, form).await;
    assert_eq!(status, StatusCode::OK, "the panel should render, not error");

    let tbd: bool = sqlx::query_scalar(
        "SELECT teacher_tbd FROM timetable_entries WHERE course_id = $1",
    )
    .bind(course_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(tbd, "the period was not saved as to-be-assigned");

    // The work queue is where the outstanding period surfaces.
    let (status, body) = get_with(app, "/admin/work-queue", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Periods with no teacher"));
    assert!(
        body.contains("Assign a teacher"),
        "the queue does not offer to resolve the period"
    );
    assert!(body.contains("ALP101"), "the queue does not name the course");
}

#[sqlx::test]
async fn every_request_gets_its_own_audit_correlation_id(pool: PgPool) {
    init_session_store(&pool).await;
    let dept = department(&pool, "alpha").await;
    let prog = programme(&pool, dept, "alpha-ug").await;
    let course_id: i64 = sqlx::query_scalar(
        "INSERT INTO courses (programme_id, department_id, code, title, credits, category, semester)
         VALUES ($1, $2, 'ALP102', 'Alpha course', 4, 'MAJOR', 3) RETURNING id",
    )
    .bind(prog)
    .bind(dept)
    .fetch_one(&pool)
    .await
    .unwrap();
    let _admin = make_user(&pool, "admin@example.com", "admin", "Password123!").await;
    let app = management_app(&pool);
    let cookie = sign_in(app.clone(), "admin@example.com", "Password123!").await;

    let (_, body) = get_with(
        app.clone(),
        &format!("/admin/timetable?programme={prog}&semester=3"),
        &cookie,
    )
    .await;
    let csrf = extract_csrf(&body);

    // Two separate requests, both audited.
    for (weekday, start, end) in [("1", "09:00", "10:00"), ("2", "09:00", "10:00")] {
        let form = format!(
            "csrf_token={csrf}&programme_id={prog}&semester=3&weekday={weekday}&start_at={start}&end_at={end}&course_id={course_id}&faculty_id=&teacher_tbd=1&room="
        );
        let (status, _) = post_form(app.clone(), "/admin/timetable/slots", &cookie, form).await;
        assert_eq!(status, StatusCode::OK);
    }

    let ids: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT correlation_id FROM audit_log WHERE action = 'timetable_slot_added' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(ids.len(), 2, "both adds should be audited");
    assert!(ids.iter().all(|id| id.is_some()), "every row needs an id");
    assert_ne!(ids[0], ids[1], "each request gets its own correlation id");
}

#[sqlx::test]
async fn the_notice_list_uses_the_shared_empty_state_and_status_badge(pool: PgPool) {
    init_session_store(&pool).await;
    let _office = make_user(&pool, "office@example.com", "staff", "Password123!").await;
    let app = management_app(&pool);
    let cookie = sign_in(app.clone(), "office@example.com", "Password123!").await;

    // A row carries the shared badge — one definition of the markup and the
    // colour, not a copy per list template.
    let (status, body) = get_with(app.clone(), "/admin/notices", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("rounded px-2 py-0.5 text-xs font-medium"),
        "a listed row did not carry the shared status badge"
    );

    // Filtering to a status with no rows shows the shared empty state.
    let (status, body) = get_with(app, "/admin/notices?status=archived", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("No notices match. Add one with the button above."),
        "the list did not use the shared empty state"
    );
}

#[sqlx::test]
async fn the_navigation_badges_appointments_waiting_for_a_decision(pool: PgPool) {
    init_session_store(&pool).await;
    let dept = department(&pool, "alpha").await;
    let admin = make_user(&pool, "admin@example.com", "admin", "Password123!").await;
    let teacher = make_user(&pool, "teacher@example.com", "faculty", "Password123!").await;
    sqlx::query("INSERT INTO faculty (user_id, department_id, name, is_hod) VALUES ($1, $2, 'Teacher', false)")
        .bind(teacher)
        .bind(dept)
        .execute(&pool)
        .await
        .unwrap();
    // An IT admin asks for a head appointment; it stays pending until the
    // Principal decides, so it is exactly the work the badge counts.
    appointments::request_head(&pool, teacher, dept, false, None, admin, "test")
        .await
        .unwrap();

    let app = management_app(&pool);
    let cookie = sign_in(app.clone(), "admin@example.com", "Password123!").await;
    let (status, body) = get_with(app, "/admin", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("Appointments waiting for a decision"),
        "the admin nav should badge the pending appointment"
    );

    // Office staff cannot decide appointments and get no badge. (They also get
    // no Appointments link at all.)
    let _office = make_user(&pool, "office@example.com", "staff", "Password123!").await;
    let app = management_app(&pool);
    let cookie = sign_in(app.clone(), "office@example.com", "Password123!").await;
    let (_, body) = get_with(app, "/admin", &cookie).await;
    assert!(
        !body.contains("Appointments waiting for a decision"),
        "an office user must not see the approval badge"
    );
}

#[sqlx::test]
async fn one_student_cannot_withdraw_another_students_selection(pool: PgPool) {
    init_session_store(&pool).await;
    let dept = department(&pool, "alpha").await;
    let prog = programme(&pool, dept, "alpha-ug").await;
    let a = make_user(&pool, "a@example.com", "student", "Password123!").await;
    let b = make_user(&pool, "b@example.com", "student", "Password123!").await;
    let _a_student = student(&pool, a, "S1001", prog).await;
    let b_student = student(&pool, b, "S1002", prog).await;

    let cat = courses::create_catalogue_course(
        &pool,
        dept,
        &courses::CourseInput {
            code: "CAT101".into(),
            title: "Catalogue course".into(),
            credits: 4,
            category: "MAJOR".into(),
            semester: 0,
        },
    )
    .await
    .unwrap();
    let year: i64 = sqlx::query_scalar(
        "INSERT INTO academic_years (label, start_year, end_year) VALUES ('2099-2100', 2099, 2100) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let offering = courses::create_offering(
        &pool,
        &courses::NewOffering {
            course_id: cat,
            offering_department_id: dept,
            academic_year_id: year,
            semester: 1,
            course_type: "Regular".into(),
            selection_mode: "INDIVIDUAL_CHOICE".into(),
            choice_group: String::new(),
            capacity: None,
            faculty_id: None,
        },
    )
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO student_course_selections (student_id, offering_id, state) VALUES ($1, $2, 'draft')",
    )
    .bind(b_student)
    .bind(offering)
    .execute(&pool)
    .await
    .unwrap();

    let app = public_app(&pool, false);
    let cookie = sign_in(app.clone(), "S1001", "Password123!").await;
    // The student signs in with their admission number, never their email.
    assert!(!cookie.is_empty(), "a student signs in with the admission number");

    let (_, body) = get_with(app.clone(), "/hub/courses", &cookie).await;
    let csrf = extract_csrf(&body);
    // A posts the offering B holds. The route resolves the student from the
    // session, so the delete is scoped to A and B's row must survive.
    let (status, _) = post_form(
        app,
        "/hub/courses/withdraw",
        &cookie,
        format!("csrf_token={csrf}&offering_id={offering}"),
    )
    .await;
    assert!(status.is_redirection(), "expected a redirect, got {status}");
    let survived: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM student_course_selections WHERE student_id = $1 AND offering_id = $2",
    )
    .bind(b_student)
    .bind(offering)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(survived, 1, "another student's selection was withdrawn");
}

#[sqlx::test]
async fn a_student_reaches_their_own_hub_but_not_the_staff_tools(pool: PgPool) {
    init_session_store(&pool).await;
    let dept = department(&pool, "alpha").await;
    let prog = programme(&pool, dept, "alpha-ug").await;
    let a = make_user(&pool, "a@example.com", "student", "Password123!").await;
    student(&pool, a, "S1001", prog).await;

    let app = public_app(&pool, false);
    let cookie = sign_in(app.clone(), "S1001", "Password123!").await;

    let (status, body) = get_with(app.clone(), "/hub", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("/logout"),
        "the student hub should render for the signed-in student"
    );

    // The teacher workspace and the management tools are role-guarded.
    let (status, _) = get_with(app, "/teacher", &cookie).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a student must not open /teacher");
    let (status, _) = get_with(management_app(&pool), "/admin/marks", &cookie).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a student must not manage marks");
}

#[sqlx::test]
async fn a_teacher_cannot_mark_attendance_for_another_teachers_class(pool: PgPool) {
    init_session_store(&pool).await;
    let dept = department(&pool, "alpha").await;
    let prog = programme(&pool, dept, "alpha-ug").await;
    let course_id: i64 = sqlx::query_scalar(
        "INSERT INTO courses (programme_id, department_id, code, title, credits, category, semester)
         VALUES ($1, $2, 'ALP201', 'Alpha course', 4, 'MAJOR', 1) RETURNING id",
    )
    .bind(prog)
    .bind(dept)
    .fetch_one(&pool)
    .await
    .unwrap();
    let t1 = make_user(&pool, "t1@example.com", "faculty", "Password123!").await;
    let t2 = make_user(&pool, "t2@example.com", "faculty", "Password123!").await;
    let f1 = faculty_row(&pool, t1, dept).await;
    let _f2 = faculty_row(&pool, t2, dept).await;
    // Today's weekday, so the period is inside the attendance edit window.
    let weekday: i32 = sqlx::query_scalar("SELECT EXTRACT(ISODOW FROM CURRENT_DATE)::int")
        .fetch_one(&pool)
        .await
        .unwrap();
    let entry: i64 = sqlx::query_scalar(
        "INSERT INTO timetable_entries (programme_id, semester, course_id, faculty_id, teacher_tbd, weekday, start_time, end_time)
         VALUES ($1, 1, $2, $3, false, $4, '09:00', '10:00') RETURNING id",
    )
    .bind(prog)
    .bind(course_id)
    .bind(f1)
    .bind(weekday as i16)
    .fetch_one(&pool)
    .await
    .unwrap();

    let app = public_app(&pool, false);
    // The assigned teacher may open the sheet.
    let assigned = sign_in(app.clone(), "t1@example.com", "Password123!").await;
    let (status, _) = get_with(
        app.clone(),
        &format!("/dashboard/teacher/session/{entry}/attendance"),
        &assigned,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the assigned teacher should reach their own sheet");

    // Another teacher, even of the same department, cannot mark it.
    let other = sign_in(app.clone(), "t2@example.com", "Password123!").await;
    let (_, body) = get_with(app.clone(), "/teacher/timetable", &other).await;
    let csrf = extract_csrf(&body);
    let (status, _) = post_form(
        app,
        &format!("/dashboard/teacher/session/0/toggle"),
        &other,
        format!("csrf_token={csrf}&student_id=1&status=present&entry_id={entry}"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a teacher must not mark another teacher's class"
    );
}

/// Office staff manage the CMS from their own `/staff` workspace: create,
/// edit, publish, archive and delete. A student may do none of it.
#[sqlx::test]
async fn office_staff_publish_and_delete_cms_content_from_the_staff_workspace(pool: PgPool) {
    init_session_store(&pool).await;
    let _staff = make_user(&pool, "staff@example.com", "staff", "Password123!").await;
    let _student = make_user(&pool, "student@example.com", "student", "Password123!").await;
    let notice: i64 = sqlx::query_scalar(
        "INSERT INTO notices (title, category, body, audience, status)
         VALUES ('Authz notice', 'Notice', 'Body', 'public', 'draft') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let app = management_app(&pool);

    // Office staff reach their list at `/staff/notices` and may publish.
    let staff = sign_in(app.clone(), "staff@example.com", "Password123!").await;
    let (status, body) = get_with(app.clone(), "/staff/notices", &staff).await;
    assert_eq!(status, StatusCode::OK, "office staff must reach their notices list");
    let csrf = extract_csrf(&body);
    let (status, _) = post_form(
        app.clone(),
        &format!("/staff/notices/{notice}/status"),
        &staff,
        format!("csrf_token={csrf}&status=published&back=/staff/notices"),
    )
    .await;
    assert!(status.is_redirection(), "a staff publish should redirect, got {status}");
    let published: String = sqlx::query_scalar("SELECT status FROM notices WHERE id = $1")
        .bind(notice)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(published, "published", "the staff publish must have changed the row");

    // ...and may delete.
    let (_, body) = get_with(app.clone(), "/staff/notices", &staff).await;
    let csrf = extract_csrf(&body);
    let (status, _) = post_form(
        app.clone(),
        &format!("/staff/notices/{notice}/delete"),
        &staff,
        format!("csrf_token={csrf}&back=/staff/notices"),
    )
    .await;
    assert!(status.is_redirection(), "a staff delete should redirect, got {status}");
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM notices WHERE id = $1")
        .bind(notice)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(remaining, 0, "the staff delete must have removed the row");

    // A student is refused before the CSRF token is even read.
    let student = sign_in(app.clone(), "student@example.com", "Password123!").await;
    let (status, _) = post_form(
        app,
        "/staff/notices/1/status",
        &student,
        "csrf_token=x&status=published&back=/staff/notices".to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a student must not publish content");
}

#[sqlx::test]
async fn the_it_admin_still_publishes_cms_content_from_admin(pool: PgPool) {
    init_session_store(&pool).await;
    let _admin = make_user(&pool, "admin@example.com", "admin", "Password123!").await;
    let notice: i64 = sqlx::query_scalar(
        "INSERT INTO notices (title, category, body, audience, status)
         VALUES ('Admin notice', 'Notice', 'Body', 'public', 'draft') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let app = management_app(&pool);
    let admin = sign_in(app.clone(), "admin@example.com", "Password123!").await;
    let (_, body) = get_with(app.clone(), "/admin/notices", &admin).await;
    let csrf = extract_csrf(&body);
    let (status, _) = post_form(
        app,
        &format!("/admin/notices/{notice}/status"),
        &admin,
        format!("csrf_token={csrf}&status=published&back=/admin/notices"),
    )
    .await;
    assert!(status.is_redirection(), "the publish should redirect, got {status}");
    let published: String = sqlx::query_scalar("SELECT status FROM notices WHERE id = $1")
        .bind(notice)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(published, "published");
}

/// An HOD is a teacher: they sign in on the public listener, where the
/// department pages are mounted at `/hod` — the same handlers, with the same
/// guards. The `/admin` form of the management area is served on this listener
/// for the IT administrator, the Principal and office staff, but not for a
/// `faculty` account: a head of department always works under `/hod`.
#[sqlx::test]
async fn an_hod_works_from_the_public_listener_under_hod(pool: PgPool) {
    init_session_store(&pool).await;
    let dept = department(&pool, "alpha").await;
    let _prog = programme(&pool, dept, "alpha-ug").await;
    let admin = make_user(&pool, "admin@example.com", "admin", "Password123!").await;
    let hod = make_user(&pool, "hod@example.com", "faculty", "Password123!").await;
    sqlx::query("INSERT INTO faculty (user_id, department_id, name, is_hod) VALUES ($1, $2, 'HOD', true)")
        .bind(hod)
        .bind(dept)
        .execute(&pool)
        .await
        .unwrap();
    appointments::grant_head(&pool, hod, dept, false, None, admin, "test")
        .await
        .unwrap();

    let app = public_app(&pool, false);
    let cookie = sign_in(app.clone(), "hod@example.com", "Password123!").await;
    assert!(!cookie.is_empty(), "sign-in did not return a session");

    // Every page the department catalogue and the HOD's sidebar point at.
    for page in [
        "/hod/work-queue",
        "/hod/manage/students",
        "/hod/manage/academic",
        "/hod/manage/offerings",
        "/hod/manage/timetable",
        "/hod/manage/attendance",
        "/hod/manage/exams",
        "/hod/notifications",
        "/hod/departments",
        "/hod/departments/attendance",
        "/hod/academics",
        "/hod/courses",
        "/hod/courses/offerings",
        "/hod/courses/external",
        "/hod/courses/selections",
        "/hod/timetable",
        "/hod/substitutions",
        "/hod/exams",
        "/hod/marks",
        "/hod/materials",
    ] {
        let (status, body) = get_with(app.clone(), page, &cookie).await;
        assert_eq!(status, StatusCode::OK, "{page} did not load for the HOD");
        assert!(
            !body.contains("/admin/"),
            "{page} still links into /admin, which this listener does not serve"
        );
    }

    // The sidebar itself is rendered in the same workspace.
    let (_, body) = get_with(app.clone(), "/hod/work-queue", &cookie).await;
    assert!(body.contains("href=\"/hod/work-queue\""));
    assert!(body.contains("href=\"/hod/marks\""));
    assert!(!body.contains("href=\"/admin"));

    // The reason the second mount exists at all: a teacher works under `/hod`,
    // so the `/admin` form of the same page is refused on this listener.
    let (status, _) = get_with(app, "/admin/marks", &cookie).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a head of department must use /hod on the public listener"
    );
}

/// Mounting the department handlers under `/hod` widens nothing: the guards run
/// per request, exactly as they do at `/admin`.
#[sqlx::test]
async fn the_department_workspace_is_refused_to_a_teacher_without_an_appointment(pool: PgPool) {
    init_session_store(&pool).await;
    let dept = department(&pool, "alpha").await;
    let _prog = programme(&pool, dept, "alpha-ug").await;
    let teacher_user = make_user(&pool, "teacher@example.com", "faculty", "Password123!").await;
    let _student_user = make_user(&pool, "student@example.com", "student", "Password123!").await;
    sqlx::query("INSERT INTO faculty (user_id, department_id, name, is_hod) VALUES ($1, $2, 'Teacher', false)")
        .bind(teacher_user)
        .bind(dept)
        .execute(&pool)
        .await
        .unwrap();

    let app = public_app(&pool, false);
    let teacher = sign_in(app.clone(), "teacher@example.com", "Password123!").await;
    let student_cookie = sign_in(app.clone(), "student@example.com", "Password123!").await;

    for page in ["/hod/marks", "/hod/manage/students", "/hod/courses/offerings"] {
        let (status, _) = get_with(app.clone(), page, &teacher).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{page} must refuse a teacher who heads nothing"
        );
    }
    // The section index is `Ready`, but the catalogue lookup still refuses a
    // role that has no areas at all.
    let (status, _) = get_with(app, "/hod/manage/students", &student_cookie).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a student has no department tools");
}

/// A department action returns the browser to the workspace it was taken in.
#[sqlx::test]
async fn a_department_action_redirects_inside_the_department_workspace(pool: PgPool) {
    init_session_store(&pool).await;
    let dept = department(&pool, "alpha").await;
    let prog = programme(&pool, dept, "alpha-ug").await;
    let admin = make_user(&pool, "admin@example.com", "admin", "Password123!").await;
    let hod = make_user(&pool, "hod@example.com", "faculty", "Password123!").await;
    sqlx::query("INSERT INTO faculty (user_id, department_id, name, is_hod) VALUES ($1, $2, 'HOD', true)")
        .bind(hod)
        .bind(dept)
        .execute(&pool)
        .await
        .unwrap();
    appointments::grant_head(&pool, hod, dept, false, None, admin, "test")
        .await
        .unwrap();
    let student_user = make_user(&pool, "s1@example.com", "student", "Password123!").await;
    let student_id = student(&pool, student_user, "A1", prog).await;

    // The HOD, on the public listener: the redirect stays in `/hod`.
    let app = public_app(&pool, false);
    let cookie = sign_in(app.clone(), "hod@example.com", "Password123!").await;
    let (_, body) = get_with(app.clone(), "/hod/departments", &cookie).await;
    let csrf = extract_csrf(&body);
    let (status, headers, _) = post_form_with_headers(
        app,
        &format!("/hod/departments/{student_id}"),
        &cookie,
        format!("csrf_token={csrf}&department_id={dept}"),
    )
    .await;
    assert!(status.is_redirection(), "placing a student should redirect, got {status}");
    let location = headers
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_eq!(location, "/hod/departments", "an HOD must not be sent into /admin");

    // The same action from the management workspace stays in `/admin`.
    let app = management_app(&pool);
    let admin_cookie = sign_in(app.clone(), "admin@example.com", "Password123!").await;
    let (_, body) = get_with(app.clone(), "/admin/departments", &admin_cookie).await;
    let csrf = extract_csrf(&body);
    let (status, headers, _) = post_form_with_headers(
        app,
        &format!("/admin/departments/{student_id}"),
        &admin_cookie,
        format!("csrf_token={csrf}&department_id={dept}"),
    )
    .await;
    assert!(status.is_redirection(), "the IT admin's placement should redirect");
    assert_eq!(
        headers.get("location").and_then(|v| v.to_str().ok()),
        Some("/admin/departments")
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

/// The appointments screen renders even when the starter faculty list contains
/// rows with no account: such a row cannot be appointed and must be skipped,
/// never decoded as an error (a NULL `user_id` used to make the page a 500).
#[sqlx::test]
async fn the_appointments_screen_skips_faculty_without_an_account(pool: PgPool) {
    init_session_store(&pool).await;
    let dept = department(&pool, "alpha").await;
    let _admin = make_user(&pool, "admin@example.com", "admin", "Password123!").await;
    sqlx::query(
        "INSERT INTO faculty (user_id, department_id, name, is_hod, status)
         VALUES (NULL, $1, 'Starter Teacher', false, 'published')",
    )
    .bind(dept)
    .execute(&pool)
    .await
    .unwrap();

    let app = management_app(&pool);
    let cookie = sign_in(app.clone(), "admin@example.com", "Password123!").await;
    let (status, body) = get_with(app, "/admin/appointments", &cookie).await;
    assert_eq!(status, StatusCode::OK, "the appointments screen must render");
    assert!(
        body.contains("Request a head appointment"),
        "the appointments form is missing"
    );
    // The account-less row is not offered as an appointee.
    assert!(
        !body.contains("Starter Teacher"),
        "a faculty row with no account must not be listed"
    );
}

/// On the public listener the IT administrator reaches the management area; root
/// authority does not, because it is loopback-only.
#[sqlx::test]
async fn an_it_admin_works_on_the_public_listener_but_root_authority_does_not(pool: PgPool) {
    init_session_store(&pool).await;
    let _admin = make_user(&pool, "admin@example.com", "admin", "Password123!").await;
    let _root = make_user(&pool, "root@example.com", "super_admin", "Password123!").await;

    let app = public_app(&pool, false);
    let admin = sign_in(app.clone(), "admin@example.com", "Password123!").await;
    let (status, _) = get_with(app.clone(), "/admin", &admin).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the IT admin must reach /admin on the public listener"
    );
    let (status, _) = get_with(app.clone(), "/admin/people", &admin).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the IT admin's tools are served on the public listener"
    );

    let root = sign_in(app.clone(), "root@example.com", "Password123!").await;
    let (status, _) = get_with(app.clone(), "/admin", &root).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "root authority is loopback-only");
    let (status, _) = get_with(app, "/admin/roles", &root).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "root authority is loopback-only");
}

/// Office staff work from `/staff` on the public listener, where they manage
/// content; `/admin` is not their workspace.
#[sqlx::test]
async fn office_staff_work_from_the_staff_workspace_on_the_public_listener(pool: PgPool) {
    init_session_store(&pool).await;
    let _staff = make_user(&pool, "staff@example.com", "staff", "Password123!").await;

    let app = public_app(&pool, false);
    let cookie = sign_in(app.clone(), "staff@example.com", "Password123!").await;
    for page in [
        "/staff",
        "/staff/manage/website",
        "/staff/notices",
        "/staff/news",
        "/staff/events",
    ] {
        let (status, _) = get_with(app.clone(), page, &cookie).await;
        assert_eq!(status, StatusCode::OK, "{page} did not load for office staff");
    }
}

