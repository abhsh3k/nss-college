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

use nss_college::services::{
    academics, appointments, content, content_admin, courses, exams, notifications, reports,
    site_admin, users,
    marks::{self, CorrectionOutcome},
};

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

// ---------- Result corrections ----------

async fn a_filed_mark(pool: &PgPool) -> (i64, i64, i64) {
    let dept = department(pool, "cs").await;
    let programme = programme(pool, dept, "test-ug").await;
    let course_id = courses::create_catalogue_course(pool, dept, &course("BCA101", "MAJOR"))
        .await
        .unwrap();
    let student: i64 = sqlx::query_scalar(
        "INSERT INTO students (admission_no, name, programme_id, batch_year, semester)
         VALUES ('2501', 'Student One', $1, 2024, 3) RETURNING id",
    )
    .bind(programme)
    .fetch_one(pool)
    .await
    .unwrap();
    let mark: i64 = sqlx::query_scalar(
        "INSERT INTO marks (student_id, course_id, assessment, marks_obtained, max_marks, semester, published)
         VALUES ($1, $2, 'internal', 42, 50, 3, true) RETURNING id",
    )
    .bind(student)
    .bind(course_id)
    .fetch_one(pool)
    .await
    .unwrap();
    (programme, course_id, mark)
}

#[sqlx::test]
async fn finalizing_a_course_locks_its_published_marks(pool: PgPool) {
    let (programme, course_id, mark) = a_filed_mark(&pool).await;
    let actor = user(&pool, "hod@example.com", "faculty").await;

    let n = marks::finalize_course(&pool, programme, 3, course_id, actor)
        .await
        .unwrap();
    assert_eq!(n, 1, "the one published row should be locked");

    let state: String = sqlx::query_scalar("SELECT workflow_state FROM marks WHERE id = $1")
        .bind(mark)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "finalized");

    // A finalized row cannot be re-saved by the grid.
    let student: i64 = sqlx::query_scalar("SELECT student_id FROM marks WHERE id = $1")
        .bind(mark)
        .fetch_one(&pool)
        .await
        .unwrap();
    let err = marks::save_sheet(
        &pool,
        programme,
        3,
        course_id,
        "internal",
        "coursework",
        "",
        50.0,
        &[marks::Cell {
            student_id: student,
            obtained: 45.0,
        }],
    )
    .await
    .expect_err("a finalized mark must not be overwritten");
    assert!(err.to_string().contains("correction workflow"), "{err}");

    let value: f64 = sqlx::query_scalar("SELECT marks_obtained::float8 FROM marks WHERE id = $1")
        .bind(mark)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(value, 42.0, "the finalized value is unchanged");
}

#[sqlx::test]
async fn a_correction_needs_a_second_manager_and_keeps_the_old_value(pool: PgPool) {
    let (programme, course_id, mark) = a_filed_mark(&pool).await;
    let first = user(&pool, "one@example.com", "faculty").await;
    let second = user(&pool, "two@example.com", "faculty").await;

    marks::finalize_course(&pool, programme, 3, course_id, first)
        .await
        .unwrap();

    let outcome = marks::request_correction(&pool, mark, first, 45.0, None, "arithmetic slip")
        .await
        .unwrap();
    assert_eq!(outcome, CorrectionOutcome::Done(1));

    let state: String = sqlx::query_scalar("SELECT workflow_state FROM marks WHERE id = $1")
        .bind(mark)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "correction_pending");

    // The requester cannot approve their own correction.
    let same = marks::decide_correction(&pool, mark, first, true)
        .await
        .unwrap();
    assert_eq!(same, CorrectionOutcome::SameActor);

    let done = marks::decide_correction(&pool, mark, second, true)
        .await
        .unwrap();
    assert_eq!(done, CorrectionOutcome::Done(1));

    let (value, state): (f64, String) =
        sqlx::query_as("SELECT marks_obtained::float8, workflow_state FROM marks WHERE id = $1")
            .bind(mark)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(value, 45.0, "the proposed value is applied");
    assert_eq!(state, "finalized", "the corrected row is locked again");

    // The replaced value is kept in the history.
    let (old_value, old_state, reason): (f64, String, String) = sqlx::query_as(
        "SELECT marks_obtained::float8, workflow_state, change_reason
           FROM mark_history WHERE mark_id = $1",
    )
    .bind(mark)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(old_value, 42.0);
    assert_eq!(old_state, "superseded");
    assert_eq!(reason, "arithmetic slip");
}

#[sqlx::test]
async fn only_a_finalized_mark_can_be_corrected(pool: PgPool) {
    let (_programme, _course_id, mark) = a_filed_mark(&pool).await;
    let actor = user(&pool, "hod@example.com", "faculty").await;

    // The row is still published (never finalized), so the workflow does not
    // apply; a plain save remains the right path.
    let outcome = marks::request_correction(&pool, mark, actor, 45.0, None, "nope")
        .await
        .unwrap();
    assert_eq!(outcome, CorrectionOutcome::NotFinalized);

    let state: String = sqlx::query_scalar("SELECT workflow_state FROM marks WHERE id = $1")
        .bind(mark)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "published");
}

// ---------- Audit detail ----------

#[sqlx::test]
async fn an_audit_detail_row_keeps_its_structured_payload(pool: PgPool) {
    let actor = user(&pool, "a@example.com", "admin").await;
    users::audit_detail(
        &pool,
        Some(actor),
        "mark_correction_requested",
        "mark",
        Some(7),
        serde_json::json!({ "reason": "arithmetic slip", "obtained": 45 }),
    )
    .await
    .unwrap();

    let (action, entity, id, details): (String, String, Option<i64>, String) = sqlx::query_as(
        "SELECT action, entity, entity_id, details::text FROM audit_log ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(action, "mark_correction_requested");
    assert_eq!(entity, "mark");
    assert_eq!(id, Some(7));
    let json: serde_json::Value = serde_json::from_str(&details).unwrap();
    assert_eq!(json["reason"], "arithmetic slip");
    assert_eq!(json["obtained"], 45);
}

// ---------- Cohort capacity ----------

#[sqlx::test]
async fn cohort_finalization_refuses_when_the_offering_cannot_hold_the_cohort(pool: PgPool) {
    let dept = department(&pool, "cs").await;
    let prog = programme(&pool, dept, "test-ug").await;
    let year = academic_year(&pool, "2024-25", 2024).await;
    let mut input = course("BCA201", "MAJOR");
    input.semester = 0;
    let cat = courses::create_catalogue_course(&pool, dept, &input)
        .await
        .unwrap();
    let version: i64 = sqlx::query_scalar(
        "SELECT id FROM course_versions WHERE course_id = $1 AND is_current",
    )
    .bind(cat)
    .fetch_one(&pool)
    .await
    .unwrap();
    let cohort: i64 = sqlx::query_scalar(
        "INSERT INTO cohorts (programme_id, batch_year) VALUES ($1, 2024) RETURNING id",
    )
    .bind(prog)
    .fetch_one(&pool)
    .await
    .unwrap();
    // One seat, two students.
    let offering: i64 = sqlx::query_scalar(
        r#"INSERT INTO course_offerings
               (course_id, course_version_id, offering_department_id, academic_year_id,
                semester, selection_mode, choice_group, capacity, status)
           VALUES ($1, $2, $3, $4, 3, 'COHORT_CHOICE', 'G', 1, 'published') RETURNING id"#,
    )
    .bind(cat)
    .bind(version)
    .bind(dept)
    .bind(year)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO course_offering_targets (offering_id, programme_id, cohort_id) VALUES ($1, $2, $3)",
    )
    .bind(offering)
    .bind(prog)
    .bind(cohort)
    .execute(&pool)
    .await
    .unwrap();
    for admission in ["2501", "2502"] {
        sqlx::query(
            "INSERT INTO students (admission_no, name, programme_id, batch_year, semester)
             VALUES ($1, $1, $2, 2024, 3)",
        )
        .bind(admission)
        .bind(prog)
        .execute(&pool)
        .await
        .unwrap();
    }
    let actor = user(&pool, "hod@example.com", "faculty").await;

    let result =
        courses::finalize_cohort_specialization(&pool, cohort, "G", offering, "pick", actor).await;
    assert!(result.is_err(), "capacity must stop the cohort finalisation");

    // Nothing was enrolled, so the offering is still empty.
    let enrolled: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM enrollments WHERE offering_id = $1 AND status = 'active'",
    )
    .bind(offering)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(enrolled, 0);

    // Raise the capacity and the same decision goes through.
    sqlx::query("UPDATE course_offerings SET capacity = 2 WHERE id = $1")
        .bind(offering)
        .execute(&pool)
        .await
        .unwrap();
    let done = courses::finalize_cohort_specialization(&pool, cohort, "G", offering, "pick", actor)
        .await
        .unwrap();
    assert_eq!(done, 2, "a cohort that fits is enrolled");
    let enrolled: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM enrollments WHERE offering_id = $1 AND status = 'active'",
    )
    .bind(offering)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(enrolled, 2);
}

// ---------- Course applicability ----------

#[sqlx::test]
async fn a_catalogue_course_is_applicable_through_its_offering(pool: PgPool) {
    let dept = department(&pool, "cs").await;
    let prog = programme(&pool, dept, "test-ug").await;
    let year = academic_year(&pool, "2024-25", 2024).await;

    // A course that belongs to the programme directly.
    let direct: i64 = sqlx::query_scalar(
        "INSERT INTO courses (programme_id, department_id, code, title, credits, category, semester)
         VALUES ($1, $2, 'BCA101', 'Direct course', 4, 'MAJOR', 3) RETURNING id",
    )
    .bind(prog)
    .bind(dept)
    .fetch_one(&pool)
    .await
    .unwrap();

    // A catalogue course (no programme of its own) carried for the programme by
    // an offering that targets it.
    let mut input = course("BCA201", "MAJOR");
    input.semester = 0;
    let cat = courses::create_catalogue_course(&pool, dept, &input)
        .await
        .unwrap();
    let version: i64 = sqlx::query_scalar(
        "SELECT id FROM course_versions WHERE course_id = $1 AND is_current",
    )
    .bind(cat)
    .fetch_one(&pool)
    .await
    .unwrap();
    let offering: i64 = sqlx::query_scalar(
        "INSERT INTO course_offerings (course_id, course_version_id, offering_department_id, academic_year_id, semester, selection_mode)
         VALUES ($1, $2, $3, $4, 3, 'FIXED') RETURNING id",
    )
    .bind(cat)
    .bind(version)
    .bind(dept)
    .bind(year)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO course_offering_targets (offering_id, programme_id) VALUES ($1, $2)")
        .bind(offering)
        .bind(prog)
        .execute(&pool)
        .await
        .unwrap();

    assert!(academics::applicable_course(&pool, prog, 3, direct).await.unwrap());
    assert!(academics::applicable_course(&pool, prog, 3, cat).await.unwrap());
    // `course_in_class` is the same rule.
    assert!(academics::course_in_class(&pool, cat, prog, 3).await.unwrap());

    // A catalogue course with no offering for this programme is not applicable.
    let mut lone = course("BCA301", "MAJOR");
    lone.semester = 0;
    let lone_id = courses::create_catalogue_course(&pool, dept, &lone)
        .await
        .unwrap();
    assert!(!academics::applicable_course(&pool, prog, 3, lone_id).await.unwrap());

    // Another programme does not inherit the target.
    let other = programme(&pool, dept, "other-ug").await;
    assert!(!academics::applicable_course(&pool, other, 3, cat).await.unwrap());
}

// ---------- Attendance report filters ----------

#[sqlx::test]
async fn the_attendance_course_filter_includes_catalogue_courses_taught_to_the_programme(
    pool: PgPool,
) {
    let dept = department(&pool, "cs").await;
    let prog = programme(&pool, dept, "test-ug").await;
    // A catalogue course belongs to no programme of its own.
    let mut input = course("BCA101", "MAJOR");
    input.semester = 0;
    let course_id = courses::create_catalogue_course(&pool, dept, &input)
        .await
        .unwrap();
    let student: i64 = sqlx::query_scalar(
        "INSERT INTO students (admission_no, name, programme_id, batch_year, semester)
         VALUES ('2501', 'Student One', $1, 2024, 3) RETURNING id",
    )
    .bind(prog)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO enrollments (student_id, course_id, semester, status)
         VALUES ($1, $2, 3, 'active')",
    )
    .bind(student)
    .bind(course_id)
    .execute(&pool)
    .await
    .unwrap();

    let opts = reports::course_options(&pool, prog).await.unwrap();
    assert!(
        opts.iter().any(|o| o.id == course_id),
        "the catalogue course taught to the programme must be offered"
    );

    // A programme no one is enrolled in from still hides it.
    let other = programme(&pool, dept, "other-ug").await;
    let opts_other = reports::course_options(&pool, other).await.unwrap();
    assert!(!opts_other.iter().any(|o| o.id == course_id));

    // No filter still lists everything.
    let all = reports::course_options(&pool, 0).await.unwrap();
    assert!(all.iter().any(|o| o.id == course_id));
}

// ---------- Notifications ----------

#[sqlx::test]
async fn notifications_are_per_user_and_marked_read_by_their_owner(pool: PgPool) {
    let alice = user(&pool, "alice@example.com", "admin").await;
    let bob = user(&pool, "bob@example.com", "admin").await;

    let n = notifications::create(
        &pool,
        alice,
        "correction",
        "A result correction needs approval",
        "a finalized mark",
        "/admin/marks",
    )
    .await
    .unwrap();

    assert_eq!(notifications::unread_count(&pool, alice).await.unwrap(), 1);
    assert_eq!(notifications::unread_count(&pool, bob).await.unwrap(), 0);
    assert_eq!(notifications::for_user(&pool, alice, 10).await.unwrap().len(), 1);

    // Bob cannot mark Alice's notification read.
    assert!(!notifications::mark_read(&pool, n, bob).await.unwrap());
    assert_eq!(notifications::unread_count(&pool, alice).await.unwrap(), 1);

    assert!(notifications::mark_read(&pool, n, alice).await.unwrap());
    assert_eq!(notifications::unread_count(&pool, alice).await.unwrap(), 0);
}

#[sqlx::test]
async fn notifying_roles_reaches_every_active_holder(pool: PgPool) {
    let p1 = user(&pool, "p1@example.com", "principal").await;
    let p2 = user(&pool, "p2@example.com", "principal").await;
    let _other = user(&pool, "t@example.com", "faculty").await;

    notifications::notify_roles(
        &pool,
        &["principal"],
        "appointment",
        "Head appointment request",
        "needs a decision",
        "/admin/appointments",
    )
    .await
    .unwrap();

    assert_eq!(notifications::unread_count(&pool, p1).await.unwrap(), 1);
    assert_eq!(notifications::unread_count(&pool, p2).await.unwrap(), 1);
    assert_eq!(notifications::mark_all_read(&pool, p1).await.unwrap(), 1);
    assert_eq!(notifications::unread_count(&pool, p1).await.unwrap(), 0);
}

// ---------- HOD appointment approval ----------

#[sqlx::test]
async fn a_head_appointment_is_pending_until_the_principal_approves(pool: PgPool) {
    let dept = department(&pool, "cs").await;
    let head = user(&pool, "head@example.com", "faculty").await;
    faculty(&pool, head, Some(dept), false).await;
    let it = user(&pool, "it@example.com", "admin").await;
    let principal = user(&pool, "principal@example.com", "principal").await;

    let id = appointments::request_head(&pool, head, dept, false, None, it, "department needs a head")
        .await
        .unwrap();

    // A pending request grants no department authority.
    assert_eq!(
        appointments::effective_department(&pool, head).await.unwrap(),
        None,
        "a pending appointment must not take effect"
    );

    // A Principal cannot approve their own appointment.
    let err = appointments::approve_head(&pool, id, head)
        .await
        .expect_err("self-approval must be refused");
    assert!(err.to_string().contains("cannot approve their own"));
    assert_eq!(
        appointments::effective_department(&pool, head).await.unwrap(),
        None
    );

    let approved = appointments::approve_head(&pool, id, principal).await.unwrap();
    assert_eq!(approved, Some(head));
    assert_eq!(
        appointments::effective_department(&pool, head).await.unwrap(),
        Some(dept),
        "approval activates the appointment"
    );
}

#[sqlx::test]
async fn a_rejected_head_appointment_never_activates(pool: PgPool) {
    let dept = department(&pool, "cs").await;
    let head = user(&pool, "head@example.com", "faculty").await;
    faculty(&pool, head, Some(dept), false).await;
    let it = user(&pool, "it@example.com", "admin").await;
    let principal = user(&pool, "principal@example.com", "principal").await;

    let id = appointments::request_head(&pool, head, dept, false, None, it, "request")
        .await
        .unwrap();
    assert!(appointments::reject_head(&pool, id, principal).await.unwrap());

    assert_eq!(
        appointments::effective_department(&pool, head).await.unwrap(),
        None
    );
    // A decided request cannot be approved afterwards.
    assert_eq!(appointments::approve_head(&pool, id, principal).await.unwrap(), None);
}

// ---------- Offerings in the student results read path ----------

#[sqlx::test]
async fn a_catalogue_course_offering_shows_in_the_student_results(pool: PgPool) {
    let dept = department(&pool, "cs").await;
    let programme = programme(&pool, dept, "test-ug").await;
    // A catalogue course carries no programme and no semester of its own (`NULL`),
    // so the semester has to come from the enrollment the offering produced.
    let mut input = course("BCA101", "MAJOR");
    input.semester = 0;
    let course_id = courses::create_catalogue_course(&pool, dept, &input)
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
    sqlx::query(
        "INSERT INTO enrollments (student_id, course_id, semester, status)
         VALUES ($1, $2, 3, 'active')",
    )
    .bind(student)
    .bind(course_id)
    .execute(&pool)
    .await
    .unwrap();

    let sems = exams::result_semesters(&pool, student).await.unwrap();
    assert!(sems.contains(&3), "semester 3 should be offered: {sems:?}");

    let blocks = exams::results_for(&pool, student, 3).await.unwrap();
    assert_eq!(blocks.len(), 1, "the offering's course should appear");
    assert_eq!(blocks[0].code, "BCA101");
}

// ---------- Scheduled and time-limited publishing ----------

fn notice_input(title: &str, status: &str, publish_at: &str, unpublish_at: &str) -> content_admin::NoticeInput {
    content_admin::NoticeInput {
        title: title.into(),
        category: "Notice".into(),
        body: "Body".into(),
        attachment_path: String::new(),
        audience: "public".into(),
        is_pinned: false,
        status: status.into(),
        publish_at: publish_at.into(),
        unpublish_at: unpublish_at.into(),
    }
}

async fn visible_notice_titles(pool: &PgPool) -> Vec<String> {
    content::notices(pool, 20)
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.title)
        .collect()
}

#[sqlx::test]
async fn a_scheduled_notice_stays_hidden_until_its_publish_time(pool: PgPool) {
    // A future publish time is lined up but must not leak yet.
    content_admin::create_notice(
        &pool,
        &notice_input("Later", "scheduled", "2099-01-01 00:00", ""),
        None,
    )
    .await
    .unwrap();
    // A scheduled row with no time at all must not fall through as visible.
    content_admin::create_notice(&pool, &notice_input("No time", "scheduled", "", ""), None)
        .await
        .unwrap();
    // A plain publish goes live immediately.
    content_admin::create_notice(&pool, &notice_input("Now", "published", "", ""), None)
        .await
        .unwrap();
    // A scheduled time already in the past has arrived, so it shows.
    content_admin::create_notice(
        &pool,
        &notice_input("Arrived", "scheduled", "2000-01-01 00:00", ""),
        None,
    )
    .await
    .unwrap();
    // A draft never shows, whatever its timestamps.
    content_admin::create_notice(&pool, &notice_input("Draft", "draft", "2000-01-01 00:00", ""), None)
        .await
        .unwrap();

    // The seed data ships public notices of its own, so check membership.
    let titles = visible_notice_titles(&pool).await;
    assert!(titles.contains(&"Now".to_string()), "a publish with no time goes live");
    assert!(titles.contains(&"Arrived".to_string()), "a past schedule has arrived");
    assert!(!titles.contains(&"Later".to_string()), "a future schedule stays hidden");
    assert!(!titles.contains(&"No time".to_string()), "an untimed schedule stays hidden");
    assert!(!titles.contains(&"Draft".to_string()), "a draft never shows");
}

#[sqlx::test]
async fn an_expired_notice_drops_out_of_the_public_list(pool: PgPool) {
    content_admin::create_notice(
        &pool,
        &notice_input("Expired", "published", "2000-01-01 00:00", "2000-02-01 00:00"),
        None,
    )
    .await
    .unwrap();
    content_admin::create_notice(
        &pool,
        &notice_input("Live", "published", "2000-01-01 00:00", "2099-01-01 00:00"),
        None,
    )
    .await
    .unwrap();

    let titles = visible_notice_titles(&pool).await;
    assert!(titles.contains(&"Live".to_string()));
    assert!(!titles.contains(&"Expired".to_string()), "an expired notice drops out");
}

#[sqlx::test]
async fn a_scheduled_page_is_not_served_until_its_time(pool: PgPool) {
    let page = |path: &str, status: &str, publish_at: &str| site_admin::PageInput {
        path: path.into(),
        title: path.into(),
        lede: String::new(),
        status: status.into(),
        nav_group: "none".into(),
        nav_label: String::new(),
        nav_sort: 0,
        publish_at: publish_at.into(),
        unpublish_at: String::new(),
    };

    site_admin::create_page(&pool, &page("/later", "scheduled", "2099-01-01 00:00"))
        .await
        .unwrap();
    site_admin::create_page(&pool, &page("/live", "published", ""))
        .await
        .unwrap();

    assert!(content::page_by_path(&pool, "/live").await.unwrap().is_some());
    assert!(
        content::page_by_path(&pool, "/later").await.unwrap().is_none(),
        "a page scheduled for the future must not be served"
    );

    // The quick publish toggle marks the page live and leaves it reachable.
    let id: i64 = sqlx::query_scalar("SELECT id FROM pages WHERE path = '/later'")
        .fetch_one(&pool)
        .await
        .unwrap();
    site_admin::set_page_status(&pool, id, "published").await.unwrap();
    assert!(content::page_by_path(&pool, "/later").await.unwrap().is_some());
}

#[sqlx::test]
async fn a_published_update_keeps_its_stored_publish_time(pool: PgPool) {
    let id = content_admin::create_notice(
        &pool,
        &notice_input("First", "published", "2020-05-05 09:30", ""),
        None,
    )
    .await
    .unwrap();

    // Saving again as published, with the time cleared, must not restamp now().
    let mut edit = notice_input("First", "published", "", "");
    let stored = content_admin::notice_for_edit(&pool, id).await.unwrap().unwrap();
    assert_eq!(stored.publish_at, "2020-05-05T09:30");
    edit.title = "Renamed".into();
    content_admin::update_notice(&pool, id, &edit).await.unwrap();

    let after = content_admin::notice_for_edit(&pool, id).await.unwrap().unwrap();
    assert_eq!(after.publish_at, "2020-05-05T09:30", "the publish time is kept");
    assert_eq!(after.title, "Renamed");
}
