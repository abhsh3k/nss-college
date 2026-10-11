//! Database integration tests.
//!
//! These run against a real PostgreSQL database: `#[sqlx::test]` creates a
//! fresh database per test, applies every migration in `./migrations`, and
//! hands the test a pool. They cover the things a unit test cannot — the
//! constraints the migrations add, and the appointment/workflow services that
//! are only correct in combination with those constraints.
//!
//! Run with `cargo test --test database`. `DATABASE_URL` must point at a server
//! whose role may create databases.

use sqlx::PgPool;

use nss_college::services::{appointments, courses};

// ---------- Fixtures ----------

async fn department(pool: &PgPool, slug: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO departments (slug, name) VALUES ($1, $2) RETURNING id",
    )
    .bind(slug)
    .bind(format!("Department {slug}"))
    .fetch_one(pool)
    .await
    .expect("insert department")
}

async fn user(pool: &PgPool, email: &str, role: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO users (email, password_hash, role) VALUES ($1, 'x', $2) RETURNING id",
    )
    .bind(email)
    .bind(role)
    .fetch_one(pool)
    .await
    .expect("insert user")
}

async fn faculty(pool: &PgPool, user_id: i64, department_id: Option<i64>, is_hod: bool) {
    sqlx::query(
        "INSERT INTO faculty (user_id, department_id, name, is_hod) VALUES ($1, $2, 'Teacher', $3)",
    )
    .bind(user_id)
    .bind(department_id)
    .bind(is_hod)
    .execute(pool)
    .await
    .expect("insert faculty");
}

async fn programme(pool: &PgPool, department_id: i64, slug: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO programmes (department_id, slug, name, level) VALUES ($1, $2, $2, 'ug') RETURNING id",
    )
    .bind(department_id)
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("insert programme")
}

/// An academic year the seed data does not already use.
async fn academic_year(pool: &PgPool, label: &str, start_year: i32) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO academic_years (label, start_year, end_year) VALUES ($1, $2, $2 + 1) RETURNING id",
    )
    .bind(label)
    .bind(start_year)
    .fetch_one(pool)
    .await
    .expect("insert academic year")
}

fn course(code: &str, category: &str) -> courses::CourseInput {
    courses::CourseInput {
        code: code.into(),
        title: format!("{code} title"),
        credits: 4,
        category: category.into(),
        semester: 1,
    }
}

// ---------- Migrations and the schema they build ----------

#[sqlx::test]
async fn the_migrations_create_the_core_tables(pool: PgPool) {
    for table in [
        "users",
        "departments",
        "programmes",
        "faculty",
        "students",
        "courses",
        "course_versions",
        "course_offerings",
        "role_assignments",
        "delegations",
        "appointment_handover_tasks",
        "marks",
        "mark_history",
        "result_import_batches",
        "result_import_rows",
        "audit_log",
        "study_materials",
    ] {
        let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(format!("public.{table}"))
            .fetch_one(&pool)
            .await
            .expect("regclass lookup");
        assert!(exists, "the migrations did not create `{table}`");
    }
}

#[sqlx::test]
async fn every_migration_applied_successfully(pool: PgPool) {
    let failed: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE NOT success")
        .fetch_one(&pool)
        .await
        .expect("migration history");
    assert_eq!(failed, 0, "a migration is recorded as failed");
}

// ---------- Authorization: appointments and delegations ----------

#[sqlx::test]
async fn a_department_can_have_only_one_active_head(pool: PgPool) {
    let dept = department(&pool, "cs").await;
    let first = user(&pool, "first@college.test", "faculty").await;
    let second = user(&pool, "second@college.test", "faculty").await;
    faculty(&pool, first, Some(dept), false).await;
    faculty(&pool, second, Some(dept), false).await;

    appointments::grant_head(&pool, first, dept, false, None, first, "first")
        .await
        .expect("the first appointment is allowed");

    let err = appointments::grant_head(&pool, second, dept, false, None, first, "second")
        .await
        .expect_err("the department already has an active head");
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("23505"),
        "the one-head rule should be the unique index, got: {err}"
    );

    // The loser was not left half-appointed.
    assert_eq!(
        appointments::effective_department(&pool, first).await.unwrap(),
        Some(dept)
    );
    assert_eq!(
        appointments::effective_department(&pool, second).await.unwrap(),
        None
    );
}

#[sqlx::test]
async fn an_expired_appointment_denies_instead_of_falling_back(pool: PgPool) {
    let dept = department(&pool, "cs").await;
    let resigned = user(&pool, "resigned@college.test", "faculty").await;
    // The legacy boolean still says "head", which is exactly the case the
    // framework must not trust once an appointment row exists.
    faculty(&pool, resigned, Some(dept), true).await;
    sqlx::query(
        "INSERT INTO role_assignments (user_id, role, department_id, status, starts_at, ends_at)
         VALUES ($1, 'hod', $2, 'active', now() - interval '30 days', now() - interval '1 day')",
    )
    .bind(resigned)
    .bind(dept)
    .execute(&pool)
    .await
    .unwrap();

    assert_eq!(
        appointments::effective_department(&pool, resigned).await.unwrap(),
        None,
        "an ended appointment must not keep granting departmental scope"
    );
}

#[sqlx::test]
async fn a_head_with_no_appointment_rows_still_uses_the_legacy_flag(pool: PgPool) {
    // The compatibility path: accounts created before the appointment model
    // have no rows at all, so the migrated boolean is what grants them scope.
    let dept = department(&pool, "cs").await;
    let legacy = user(&pool, "legacy@college.test", "faculty").await;
    faculty(&pool, legacy, Some(dept), true).await;

    assert_eq!(
        appointments::effective_department(&pool, legacy).await.unwrap(),
        Some(dept)
    );
}

#[sqlx::test]
async fn a_delegation_follows_the_appointment_it_was_granted_from(pool: PgPool) {
    let dept = department(&pool, "cs").await;
    let head = user(&pool, "head@college.test", "faculty").await;
    let delegate = user(&pool, "delegate@college.test", "faculty").await;
    faculty(&pool, head, Some(dept), false).await;
    faculty(&pool, delegate, Some(dept), false).await;

    let assignment = appointments::grant_head(&pool, head, dept, false, None, head, "appointed")
        .await
        .unwrap();
    appointments::grant_delegation(&pool, delegate, dept, "attendance", "2099-01-01", head)
        .await
        .unwrap();
    assert_eq!(
        appointments::effective_department(&pool, delegate).await.unwrap(),
        Some(dept)
    );

    // Ending the head's appointment must also end what they delegated: the
    // incoming head reviews it, it does not carry over.
    appointments::revoke_head(&pool, assignment, head).await.unwrap();
    assert_eq!(
        appointments::effective_department(&pool, delegate).await.unwrap(),
        None,
        "a delegation outlived the appointment that granted it"
    );
}

#[sqlx::test]
async fn an_acting_head_holds_the_same_scope_for_its_dates(pool: PgPool) {
    let dept = department(&pool, "cs").await;
    let acting = user(&pool, "acting@college.test", "faculty").await;
    faculty(&pool, acting, Some(dept), false).await;

    appointments::grant_head(&pool, acting, dept, true, Some("2099-01-01"), acting, "acting")
        .await
        .unwrap();
    assert_eq!(
        appointments::effective_department(&pool, acting).await.unwrap(),
        Some(dept)
    );

    // An appointment that has already ended grants nothing.
    let expired = user(&pool, "past@college.test", "faculty").await;
    let other = department(&pool, "maths").await;
    faculty(&pool, expired, Some(other), false).await;
    sqlx::query(
        "INSERT INTO role_assignments (user_id, role, department_id, status, starts_at, ends_at)
         VALUES ($1, 'acting_hod', $2, 'active', now() - interval '10 days', now() - interval '1 day')",
    )
    .bind(expired)
    .bind(other)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        appointments::effective_department(&pool, expired).await.unwrap(),
        None
    );
}

#[sqlx::test]
async fn ending_a_head_appointment_opens_a_handover_task(pool: PgPool) {
    let dept = department(&pool, "cs").await;
    let head = user(&pool, "head@college.test", "faculty").await;
    faculty(&pool, head, Some(dept), false).await;
    let assignment = appointments::grant_head(&pool, head, dept, false, None, head, "appointed")
        .await
        .unwrap();

    appointments::revoke_head(&pool, assignment, head).await.unwrap();

    let open: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM appointment_handover_tasks WHERE assignment_id = $1 AND status = 'open'",
    )
    .bind(assignment)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(open, 1, "the successor has nothing to review");
}

#[sqlx::test]
async fn the_audit_log_is_append_only(pool: PgPool) {
    sqlx::query("INSERT INTO audit_log (action, entity) VALUES ('seed', 'test')")
        .execute(&pool)
        .await
        .unwrap();

    assert!(
        sqlx::query("UPDATE audit_log SET action = 'edited'").execute(&pool).await.is_err(),
        "an ordinary administrator could rewrite the audit trail"
    );
    assert!(
        sqlx::query("DELETE FROM audit_log").execute(&pool).await.is_err(),
        "an ordinary administrator could delete the audit trail"
    );
}

// ---------- Courses, versions and offerings ----------

#[sqlx::test]
async fn a_new_catalogue_course_starts_with_one_version(pool: PgPool) {
    let dept = department(&pool, "cs").await;
    let id = courses::create_catalogue_course(&pool, dept, &course("BCA101", "MAJOR"))
        .await
        .unwrap();

    let versions = courses::course_versions(&pool, id).await.unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].version_no, 1);
    assert!(versions[0].is_current);
}

#[sqlx::test]
async fn editing_a_catalogue_course_appends_a_version_and_keeps_history(pool: PgPool) {
    let dept = department(&pool, "cs").await;
    let id = courses::create_catalogue_course(&pool, dept, &course("BCA101", "MAJOR"))
        .await
        .unwrap();

    let mut edited = course("BCA101", "MAJOR");
    edited.title = "Introduction to Computing".into();
    let next = courses::update_catalogue_course(&pool, id, &edited, None)
        .await
        .unwrap();
    assert_eq!(next, Some(2), "a changed course should append version 2");

    let versions = courses::course_versions(&pool, id).await.unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(
        versions.iter().filter(|v| v.is_current).count(),
        1,
        "exactly one version may be current"
    );
    // The old wording is still readable.
    assert!(versions.iter().any(|v| v.title == "BCA101 title"));
    assert!(versions.iter().any(|v| v.title == "Introduction to Computing"));

    // Re-saving the same values changes nothing.
    let again = courses::update_catalogue_course(&pool, id, &edited, None)
        .await
        .unwrap();
    assert_eq!(again, None, "an unchanged save must not churn the history");
    assert_eq!(courses::course_versions(&pool, id).await.unwrap().len(), 2);
}

#[sqlx::test]
async fn an_offering_is_pinned_to_the_version_it_was_built_from(pool: PgPool) {
    let dept = department(&pool, "cs").await;
    let course_id = courses::create_catalogue_course(&pool, dept, &course("BCA101", "MAJOR"))
        .await
        .unwrap();
    let year = academic_year(&pool, "2099-2100", 2099).await;

    let offering = courses::create_offering(
        &pool,
        &courses::NewOffering {
            course_id,
            offering_department_id: dept,
            academic_year_id: year,
            semester: 1,
            course_type: "Regular".into(),
            selection_mode: "FIXED".into(),
            choice_group: String::new(),
            capacity: None,
            faculty_id: None,
        },
    )
    .await
    .unwrap();

    let pinned_version: i64 = sqlx::query_scalar(
        "SELECT course_version_id FROM course_offerings WHERE id = $1",
    )
    .bind(offering)
    .fetch_one(&pool)
    .await
    .unwrap();
    let (pinned_code, pinned_title): (String, String) = sqlx::query_as(
        "SELECT code, title FROM course_versions WHERE id = $1",
    )
    .bind(pinned_version)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pinned_title, "BCA101 title");

    // Editing the course afterwards does not rewrite what the offering ran as.
    let mut edited = course("BCA101", "MAJOR");
    edited.title = "Renamed later".into();
    courses::update_catalogue_course(&pool, course_id, &edited, None)
        .await
        .unwrap();

    let still_pinned: i64 = sqlx::query_scalar(
        "SELECT course_version_id FROM course_offerings WHERE id = $1",
    )
    .bind(offering)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(still_pinned, pinned_version, "the offering followed the edit");
    assert_eq!(pinned_code, "BCA101");
}

#[sqlx::test]
async fn an_offering_cannot_be_created_without_a_version(pool: PgPool) {
    let dept = department(&pool, "cs").await;
    let course_id = courses::create_catalogue_course(&pool, dept, &course("BCA101", "MAJOR"))
        .await
        .unwrap();
    let year = academic_year(&pool, "2099-2100", 2099).await;

    let err = sqlx::query(
        "INSERT INTO course_offerings
             (course_id, offering_department_id, academic_year_id, semester, selection_mode)
         VALUES ($1, $2, $3, 1, 'FIXED')",
    )
    .bind(course_id)
    .bind(dept)
    .bind(year)
    .execute(&pool)
    .await
    .expect_err("an offering with no course version must be refused");
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("23502"),
        "the NOT NULL on course_version_id should be what refuses it"
    );
}

#[sqlx::test]
async fn the_database_rejects_a_category_outside_the_catalogue_list(pool: PgPool) {
    let dept = department(&pool, "cs").await;
    let bogus = courses::create_catalogue_course(&pool, dept, &course("BCA999", "NONSENSE"))
        .await
        .expect_err("the CHECK constraint should refuse this category");
    assert_eq!(
        bogus.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("23514")
    );

    // The service helper agrees with the constraint, so a bad value is caught
    // as a form error before it ever reaches the database.
    assert!(!courses::category_is_valid("NONSENSE"));
    assert!(courses::category_is_valid("MAJOR"));
    assert!(courses::category_is_valid(""), "uncategorised is allowed");
    for c in courses::CATEGORIES {
        assert!(courses::category_is_valid(c.value));
        assert!(!c.label.is_empty());
    }
}

// ---------- Result workflow ----------

#[sqlx::test]
async fn a_mark_carries_a_workflow_state(pool: PgPool) {
    let dept = department(&pool, "cs").await;
    let programme = programme(&pool, dept, "test-ug").await;
    let course_id = courses::create_catalogue_course(&pool, dept, &course("BCA101", "MAJOR"))
        .await
        .unwrap();
    let student: i64 = sqlx::query_scalar(
        "INSERT INTO students (admission_no, name, programme_id, batch_year, semester)
         VALUES ('2501', 'Student One', $1, 2024, 3) RETURNING id",
    )
    .bind(programme)
    .fetch_one(&pool)
    .await
    .unwrap();

    let mark: i64 = sqlx::query_scalar(
        "INSERT INTO marks (student_id, course_id, assessment, marks_obtained, max_marks)
         VALUES ($1, $2, 'internal', 42, 50) RETURNING id",
    )
    .bind(student)
    .bind(course_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let state: String = sqlx::query_scalar("SELECT workflow_state FROM marks WHERE id = $1")
        .bind(mark)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "published", "the default state for a new mark");

    let err = sqlx::query("UPDATE marks SET workflow_state = 'invented' WHERE id = $1")
        .bind(mark)
        .execute(&pool)
        .await
        .expect_err("an unknown workflow state must be refused");
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("23514")
    );
}

#[sqlx::test]
async fn a_staged_result_import_keeps_its_payload_and_a_known_state(pool: PgPool) {
    let dept = department(&pool, "cs").await;
    let programme = programme(&pool, dept, "test-ug").await;

    let batch: i64 = sqlx::query_scalar(
        "INSERT INTO result_import_batches (programme_id, semester, exam_kind, payload, assessment, exam_name)
         VALUES ($1, 3, 'university', 'prn,course_code,marks\n2025BCA0001,BCA101,42', 'internal', 'Internal 1')
         RETURNING id",
    )
    .bind(programme)
    .fetch_one(&pool)
    .await
    .unwrap();

    let (payload, state, assessment): (String, String, String) = sqlx::query_as(
        "SELECT payload, state, assessment FROM result_import_batches WHERE id = $1",
    )
    .bind(batch)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(payload.contains("2025BCA0001"), "the raw text was not kept");
    assert_eq!(state, "preview", "a staged import starts as a preview");
    assert_eq!(assessment, "internal", "the guided import's fixed assessment is kept");

    sqlx::query("INSERT INTO result_import_rows (batch_id, line_no, identity_key, course_code) VALUES ($1, 1, '2025BCA0001', 'BCA101')")
        .bind(batch)
        .execute(&pool)
        .await
        .unwrap();

    let err = sqlx::query("UPDATE result_import_batches SET state = 'invented' WHERE id = $1")
        .bind(batch)
        .execute(&pool)
        .await
        .expect_err("an unknown batch state must be refused");
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("23514")
    );

    // Deleting the batch takes its staged rows with it.
    sqlx::query("DELETE FROM result_import_batches WHERE id = $1")
        .bind(batch)
        .execute(&pool)
        .await
        .unwrap();
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM result_import_rows")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "staged rows should cascade with their batch");
}
