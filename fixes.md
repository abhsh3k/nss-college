# NSS College redesign audit and fixes

Date: 2026-10-10  
Scope: repository at `c982ec1`, the current `src/`, templates, migrations, setup files, and `redesign.md`.

This document records the concrete fixes required by the redesign brief after inspecting the implementation, followed by the verification limitations for the implementation pass.

## Implementation pass status

Implemented in the current working tree: separate public/loopback management routers and configuration, loopback bind validation, protected upload delivery, manager `NULL`-department denial, HTMX attendance ownership/roster checks and read-only sheet loading, college-local time helpers, report/timetable/teacher-scope fixes, offering clash expansion, non-destructive account archival, transactional upload cleanup, page-section parent scoping, CMS publish restrictions, additive authorization/result-workflow migrations, student-action audit entries, and focused attendance unit tests.

Not fully implemented yet: the complete appointment/permission cutover, result preview UI/commit handlers, full course-version migration, rate limiting/session revocation, backup automation, and the complete integration/end-to-end suite. These remain listed below with their required implementation details.

## 1. Executive summary

The application has a useful foundation: the required Rust/Axum/Askama/HTMX/Tailwind/SQLx/PostgreSQL stack is in place, SQL values are generally parameterized, Askama templates are escaped, CSRF is present on the existing forms, login rotates the session ID, and several course-offering handlers re-check department scope server-side.

It is not yet ready for the redesign scope. The highest-risk problems are:

1. All public, student, faculty, office, and administrative routes run on one listener. There is no server-enforced localhost-only Super Admin interface.
2. The authorization model is only a role string plus mutable `faculty.is_hod`/`can_manage` flags. It has no Super Admin, Principal, appointment, effective-date, delegation, handover, or permission lifecycle model.
3. A manager with a faculty row whose `department_id` is `NULL` can be treated as unrestricted by several scope helpers. A missing department must deny access, never mean “all departments.”
4. The newer HTMX attendance endpoints accept arbitrary timetable/session/student IDs. A teacher can create or edit another teacher’s attendance, and a `GET` creates an attendance session.
5. Course, offering, timetable, exam, enrolment, and result relationships are not consistently enforced by database constraints or transaction-scoped checks. Nullable faculty assignments can produce timetable rows that no teacher can see or mark.
6. Marks/results are immediately published and upserted/deleted in place. There is no finalized-record protection, correction workflow, immutable history, preview/confirmation import flow, or explicit official-result precedence.
7. `/uploads` is mounted directly with `ServeDir`, so knowing the path can expose a draft or archived attachment. Unpublished content must be protected at the storage/download boundary.
8. Permanent user deletion cascades through student academic records, which contradicts the requirement to preserve academic history.
9. The automated test surface is very small and there are no database, authorization-isolation, workflow, migration, or end-to-end suites.

## 2. Existing feature inventory

The current repository contains:

- Public pages, programmes, departments, notices, news, rank holders, contact, database-driven pages, page sections, homepage settings, and a faculty page.
- Login, PostgreSQL-backed sessions, password change/reset, account activation, lockout, CSRF, role dashboards, and CLI account creation.
- Admin/office content management, document and image uploads, student import/review, people management, promotion, programme courses, catalogue courses, offerings, targets, external approvals, student selections, cohort specializations, timetable, substitutions, attendance, exam timetables, marks grid, and CSV result import.
- Teacher today/timetable/attendance/report screens and a student hub for courses, timetable, exams, attendance, notices, and results.
- Migrations through `0020_teaching_departments.sql`.

The fixes below preserve these features. “Replace” means replacing an unsafe implementation with an equivalent workflow, not silently removing the feature.

## 3. P0 fixes: security and authorization

### P0.1 Split the public and private listeners

**Evidence:** `src/config.rs` has one `HOST`/`PORT`; `src/main.rs` binds one listener; `src/routes/mod.rs` merges public, hub, teacher, and admin routes into one router. `Role::Admin` is the only technical administrator role.

**Fix:**

- Add separate configuration for a public listener and a management listener.
- Bind the management listener explicitly to `127.0.0.1` in application code. Do not use a hidden link, browser check, `Host` header, forwarded header, or JavaScript check as the boundary.
- Build separate routers so `/admin` and the future Super Admin routes are not registered on the public listener. Keep public website/student/teacher routes on the public listener according to the deployment decision.
- Document the proxy arrangement and verify that the proxy does not forward public/LAN traffic to the management listener.
- Retain sessions, CSRF, authentication, and audit logging on the management listener.
- Add a startup test/configuration validation that rejects a non-loopback management bind.

### P0.2 Introduce explicit account, appointment, and permission lifecycles

**Evidence:** `users.role` only allows `student`, `faculty`, `staff`, `admin`, and `alumni` (`migrations/0001_core.sql`). HOD authority is represented by `faculty.is_hod` and `faculty.can_manage` (`0006`, `0009`). There are no appointments, Principal/Super Admin roles, effective dates, delegation records, or handover records.

**Fix with a versioned migration:**

- Keep account existence, login activation, academic appointment, role/permission grant, and department/course assignment as distinct concepts.
- Add an explicit role/permission model for Super Admin, IT Admin, Principal/Academic Admin, HOD, Acting HOD, faculty, student, office staff, and alumni. Prefer role assignment rows with effective dates and revocation timestamps over more boolean columns.
- Add appointment rows with department, appointment type, start/end, status, approver, approval time, and audit metadata.
- Add delegation rows with the granting appointment, exact allowed responsibilities, start/end, revocation, and successor review state. Do not grant restricted HOD-only powers through delegation.
- Ensure the Principal appointment is approved by Super Admin and cannot be self-approved. Ensure privileged appointments cannot be activated merely because an account exists.
- Add a partial unique index or an equivalent transaction-safe constraint for at most one active HOD/Acting HOD appointment per department. Use a transaction/locking strategy so concurrent appointment requests cannot create two active heads.
- When an appointment expires, derive permissions as expired immediately and mark its delegated work for handover. Preserve the former actor on every academic record.
- Move the current `is_hod`/`can_manage` values into the new model through a documented data migration; do not simply drop them.

### P0.3 Correct the `NULL` department scope escalation

**Evidence:** `Manager` in `src/auth/mod.rs` constructs `Scope::Department(p.department_id)`. Several helpers in `src/services/academics.rs` interpret `department: None` as unrestricted. `departments.rs` also skips checks when the manager department is `None`.

**Fix:**

- Represent global scope separately from “manager has no department.” For example, use `Scope::All` only for the explicit global role and `Scope::Department(i64)` for a valid department; reject a faculty manager with no department.
- Make `may_manage_programme`, `may_manage_teacher`, `ensure_our_department`, placement, attendance, marks, exams, substitutions, offerings, and selections use the same scope decision service.
- Refuse creation or update of an HOD/delegate appointment without a valid department.
- Add a regression test proving a faculty account with `is_hod/can_manage`-equivalent authority and `NULL department_id` receives `403` for every management operation.
- Prevent department deletion or transfer from silently nulling an active appointment. Require a succession/transfer workflow first.

### P0.4 Remove teacher attendance IDORs and state-changing GETs

**Evidence:** `src/routes/teacher.rs`:

- `get_attendance_sheet` accepts any `entry_id`, fetches its course, and creates an `attendance_sessions` row without checking teacher ownership or substitution.
- The route is `GET /dashboard/teacher/session/:entry_id/attendance`, but it mutates the database.
- `toggle_attendance_status` checks only the date window, then accepts arbitrary `session_id` and `student_id` values.
- The HTMX roster does not apply the offering-specific roster filter used by `services::attendance::roster`.

**Fix:**

- Make sheet/session creation a POST, or create the session as part of an authorized attendance-save operation.
- Resolve the timetable entry through the same `attendance::period`/ownership query as the classic attendance path. The authorized teacher must be the assigned teacher or a valid substitute for that date.
- Resolve the session from its timetable entry and date; never trust a session ID alone.
- Verify the submitted student belongs to the exact roster for that entry, course, date, and offering. Reject forged or stale IDs.
- Reuse the offering narrowing logic: only confirmed/locked selections for that offering may appear on an offering roster.
- Validate status through one shared domain function and record the acting user, actual teacher, correction reason where applicable, and timestamp.
- Add cross-teacher, cross-student, cross-offering, stale-date, substitution, and GET-no-mutation tests.

### P0.5 Make uploaded content private by default

**Evidence:** `src/main.rs` mounts `ServeDir::new(&cfg.upload_dir)` at `/uploads`. Content rows store paths, but draft/archived state is not checked when a file path is fetched directly.

**Fix:**

- Stop exposing the raw upload directory as an unrestricted public directory.
- Serve public assets through a handler that checks the owning content row and its published/audience state, or store private uploads outside the public directory and issue authorized download responses.
- Keep documents/attachments/images used by public published content explicitly public; keep drafts and archived content inaccessible to students and anonymous users.
- Add no-cache/no-store rules appropriate to private downloads and ensure stale browser/proxy caches cannot reveal unpublished files.
- Include an orphan-file reconciliation command and a database/file consistency check.

### P0.6 Strengthen authentication/session boundaries

**Evidence:** Login has account lockout and session cycling, but no IP/user rate limiter, session revocation/version, reauthentication for critical changes, or absolute session lifetime. `COOKIE_SECURE` defaults to `false`.

**Fix:**

- Add a rate limit covering both account and source/IP dimensions without revealing whether an identifier exists.
- Add session revocation/version checking so deactivation, password reset, role/appointment change, and emergency intervention invalidate existing sessions.
- Keep the current login session rotation and CSRF rotation. Add reauthentication for role/appointment changes, result finalization/correction, backup/restore, and emergency actions.
- Validate production configuration at startup: secure cookies, HTTPS/public URL consistency, trusted proxy configuration, and loopback management bind.
- Do not add MFA as an unagreed requirement; document it as a future option only if the audit later establishes that need.

### P0.7 Make audit records trustworthy and complete

**Evidence:** `audit_log` has actor/action/entity/entity_id/details but no append-only enforcement. Many records omit details/reasons; student course actions do not write audit rows; some audit errors are ignored and others cause a 500 after the business mutation already committed.

**Fix:**

- Add a shared audit service with action, actor, target, request/correlation ID, before/after summary, reason, and source/listener.
- Audit role and appointment changes, activation/deactivation, delegation, emergency actions, CMS publishing, settings, upload/download access where sensitive, timetable changes, attendance corrections, selection/assignment/change requests, imports, result filing/publication/correction/finalization, and backup/restore.
- Enforce append-only behavior for ordinary administrators at the database permission/trigger boundary. Do not expose an audit-delete/update route.
- Never include passwords, temporary passwords, CSRF tokens, sessions, or upload secrets in audit details or logs.
- Decide and document one failure policy: critical mutations must commit with their audit atomically, or an outbox must retry audit writes. Never return a misleading 500 after a mutation succeeded.
- Remove the duplicate `user_created_cli` audit call in `src/cli.rs`.

## 4. P0 fixes: data integrity and academic boundaries

### P0.8 Replace unsafe/destructive migration behavior

**Evidence:** `0006_auth_and_attendance.sql` executes `DROP TABLE attendance`; `people::delete_users` deletes student/faculty rows and cascades marks, attendance, enrolments, selections, and other records.

**Fix:**

- Do not use destructive table drops in the redesign migration path. If old attendance must be retired, copy/transform it into the new model, validate counts, retain the source/history, and document rollback assumptions.
- Replace permanent account deletion with deactivation/archival. Keep identity references and academic authorship. Remove or restrict the bulk-delete action; if emergency deletion is required, make it Super Admin-only, audited, and blocked for accounts with academic history.
- Block deactivation/deletion of the last active Super Admin/IT recovery account.
- Add migration checks for row counts, foreign-key integrity, duplicate identities, orphan uploads, and audit preservation.

### P0.9 Enforce course identity, version, and relationship correctness

**Evidence:** `courses` is simultaneously used for programme-bound courses and department catalogue courses. Codes/titles can be edited in place. Catalogue courses have `semester = NULL`; offering semester supplies context, but enrolments/results/timetable queries do not always use that context. There is no explicit course equivalence/version mapping.

**Fix with new schema concepts:**

- Give each conceptual course a stable internal identity.
- Add curriculum/course-version rows for batch/academic-year applicability, code, title, credits, department owner, and active dates. Store confirmed equivalence mappings when a code changes; do not infer equivalence from names.
- Make offerings, enrolments, timetable entries, exams, marks, and results reference the applicable course version/offering, not only a mutable course code.
- Preserve old versions when a code/title changes. Retire them instead of rewriting history.
- Add composite relationship constraints or transaction checks so a course, programme, department, semester, academic year, offering target, and faculty assignment are mutually valid.
- Add indexes and uniqueness constraints for the intended academic scope, including normalized codes and one current version where appropriate.

### P0.10 Stop silently creating incomplete timetable/course rows

**Evidence:** `academics::create_course`, `update_course`, `add_slot`, and offering-period creation use `NULLIF(..., 0)` for faculty IDs. `add_slot` accepts a submitted `faculty_id` without checking that it exists or is in the permitted scope. `build_panel` uses all teacher options even for a department manager. The result can be a timetable row with no visible teacher.

**Fix:**

- Decide explicitly whether an unassigned course is a valid draft. If it is valid, model it as a draft/unassigned course and exclude it from schedulable/published timetable workflows. If it is not valid for a timetable, reject the write; do not silently convert invalid IDs to `NULL`.
- Validate course ownership, programme, semester, department, faculty existence, faculty active appointment, and manager scope in the same transaction as the insert/update.
- Use department-scoped teacher options for HOD panels and re-check the submitted teacher server-side.
- Require a valid teacher for a published/scheduled period, or use an explicit `teacher_tbd` state with a visible work-queue item.
- Add DB checks/FKs for exact timetable owner relationships where PostgreSQL can express them, and application transaction checks where they cannot.
- Add regression tests for newly created course/timetable records and for forged faculty/course IDs.

### P0.11 Fix race conditions and stale scope in course offerings

**Evidence:** capacity is checked by counting rows before insert in `offering_for_confirm`; concurrent confirmations can both observe a free seat. HOD assignment and cohort finalization do not consistently enforce capacity. `replace_targets`, approval synchronization, fixed-course retraction, and auto-application are separate writes. Cohort finalization finds “other” offerings by group/year without constraining them to the current cohort.

**Fix:**

- Lock the offering row or use an atomic capacity reservation/update inside the confirmation transaction.
- Apply one capacity policy consistently to student confirmation, HOD assignment, cohort finalization, and change approval; if an authorized override is intended, record it with reason and audit.
- Make target replacement, approval synchronization, fixed-course application/retraction, and audit one transaction or use a durable task/outbox with visible failures.
- Constrain cohort alternatives to the current cohort’s targets and choice group. Do not delete or rewrite another cohort’s selections.
- Add a uniqueness rule for one pending change request per student/current offering where required, and prevent a locked selection from being changed through a request or approval.
- Audit select, withdraw, confirm, assign, unassign, state lock/unlock, change request, approval, and cohort finalization.

### P0.12 Validate exam/timetable course relationships at the write boundary

**Evidence:** `exams::create_exam` inserts the submitted `programme_id`, `semester`, and `course_id` without verifying the course is one of the selected programme/semester’s applicable courses. The database has independent foreign keys but no composite relationship. A forged form can attach an unrelated course.

**Fix:**

- Use one service-level applicability query for exam creation, marks sheets, result imports, and timetable writes.
- Ensure catalogue courses are attached through a published, targeted offering for the selected programme, semester, and academic year; programme-bound courses must match their programme and semester.
- Add database constraints or immutable mapping tables so an exam/result cannot be attached to an unrelated course/programme.
- Test forged course IDs and cross-department course IDs as authorization/integrity failures.

## 5. P1 fixes: academic workflow correctness

### P1.1 Add explicit result and assessment lifecycles

**Evidence:** `marks` are written with `published = true` by `services/exams.rs` and `services/marks.rs`; management can toggle or delete rows. Upserts overwrite the existing row. There is no finalization, correction reason, supersession, or history table.

**Fix:**

- Model assessment rows with states such as `draft -> reviewed -> finalized -> published`, plus `correction_pending`/`superseded` where needed.
- Store internal coursework/assessments separately from the official university result while linking both to the same applicable course version and semester.
- Define official-result precedence explicitly in student views: university result wins for the official result display; authorized staff can still see internal history.
- Prevent updates/deletes to finalized/published rows except through an authorized correction workflow requiring reason, actor, before/after values, approval, and audit history.
- Keep prior semesters and internal assessments when a student advances or a university result arrives.
- Support one to three internal exams through assessment rows/configuration, not schema columns per exam.
- Rework `results_for` and the student totals so it does not simply sum mutually exclusive internal/university rows as one official total.

### P1.2 Build a real result-import preview and commit flow

**Evidence:** `/admin/exams/results`, `/admin/exams/results/csv`, and `/admin/marks/import` validate and write rows in the same request. Valid rows are committed even if other rows fail. Course matching uses a code lookup that can select the first duplicate course. There is no durable preview, explicit course-code mapping, duplicate resolution screen, or confirmation step.

**Fix:**

- Parse the upload into a staged import batch with uploader, scope, semester, kind, source file metadata, and expiry.
- Show a preview with row-level errors, duplicate rows, student identity match, course/version match, applicable offering, existing-result conflict, and proposed state transition.
- Require explicit mapping when a code is ambiguous or changed. Never match by student name alone.
- Require an explicit confirmation POST with a fresh CSRF token and re-check authorization/relationships before committing.
- Commit the selected valid rows in one transaction, with no silent overwrite of finalized results. Offer an all-or-nothing option for official imports and clearly label any permitted partial mode.
- Add CSV and Excel tests for malformed input, duplicate identities, ambiguous codes, invalid marks, cross-department rows, existing finalized rows, and rollback.

### P1.3 Repair semester/enrolment/result drift

**Evidence:** promotion in `services/people.rs` only increments `students.semester`; it does not synchronize new semester enrolments. Several offering/enrolment paths use the student’s current semester instead of the offering semester. `hub::week_schedule` joins offering periods by course and student but does not require `enrollments.offering_id = offering.id`. Attendance/course percentages also aggregate broadly by course.

**Fix:**

- Make promotion a transaction that records a lifecycle event, preserves old enrolments/results, and creates the new applicable enrolments/selection tasks according to policy.
- Set enrollment semester from the applicable offering/course version, never from the mutable current student semester when recording historical work.
- Require offering identity in offering timetable/result/attendance joins. A student enrolled through one offering must not see another offering’s periods.
- Scope attendance percentages to the student’s active applicable enrollment/selection and relevant academic period.
- Add tests for promotion, a cross-department offering, past semester results, and two offerings of the same catalogue course.

### P1.4 Fix attendance date/route/report defects

**Evidence:** Several paths still use `OffsetDateTime::now_utc()` for college-local dates/times (`routes/dashboards.rs`, `routes/teacher.rs`, admin attendance defaults). The HTMX edit-window query uses database `CURRENT_DATE` rather than the shared Kolkata expression. The department attendance routes in `src/routes/admin/mod.rs` expose only `/:entry_id`, while handlers destructure `Path<(entry_id, date)>`, matching the known 500 error. `reports::by_session` selects `sess.on_date` twice with incompatible decoding. Offering periods are omitted by `attendance::teacher_day` because it inner-joins `programmes`. README also records programme/course-filter inconsistencies in reports.

**Fix:**

- Create one college-time service/helper returning local date, weekday, and local time; use it in dashboards, teacher views, hub pages, attendance sessions, edit windows, reports, exams, and event publishing.
- Make department attendance routes consistently use `/.../:entry_id/:date` or a validated query date, and add route tests.
- Remove the duplicate/incompatible `on_date` projection and align programme/course filters across totals, sessions, students, and options.
- Include offering periods in substitution candidates and all relevant teacher views.
- Fix offering-vs-offering clash detection when different offerings target the same programme/cohort.
- Decide whether the stored `leave` value remains an internal status while UI says “Special”; document and test the mapping.

### P1.5 Complete academic content features already represented in the schema

**Evidence:** `study_materials` exists since migration `0002`, but there is no route/UI/service flow. README identifies this as missing.

**Fix:**

- Add teacher/HOD upload and student authorized download for study materials, scoped to assigned course/offering and published state.
- Use the same private-upload policy, audit, replacement cleanup, and finalization rules as other academic content.
- Add tests proving a teacher/student cannot access materials for an unrelated course or department.

## 6. P1 fixes: CMS and content administration

### P1.1 Align publishing authority with the brief

**Evidence:** notices, news, and events use `OfficeOrAdmin`, so office staff can create, edit, publish, archive, and delete CMS content. The redesign brief assigns CMS publishing authority to IT Admin.

**Fix:**

- Decide whether “office staff” is a separate permitted CMS author role. Under the stated brief, move publish/unpublish/schedule/archive authority to IT Admin and let office staff create drafts or submit publishing tasks.
- Enforce this server-side, not by hiding controls.
- Add tests for draft creation, publish, unpublish, schedule, and archive by IT Admin versus office staff.

### P1.2 Add complete CMS state and scheduling rules

**Evidence:** `clean_status` supports only `draft`, `published`, and `archived`; no scheduled state exists. `published_at` is retained when content is unpublished and reused on republish. Direct rows and page sections are checked inconsistently.

**Fix:**

- Add explicit scheduled/published/unpublished/archived transitions with timezone-aware start/end times and a single public visibility predicate.
- Clear or version publication timestamps when appropriate; preserve publication history in an audit/version table.
- Reserve system routes and normalize page paths so an admin cannot create a content row that conflicts with `/login`, `/admin`, `/hub`, `/static`, `/uploads`, or another protected route.
- In page section update/delete/move operations, constrain the section by both `page_id` and `section_id`; currently the section ID is checked independently of the page ID.
- Add audience-specific authorization for student/faculty notices and documents.

### P1.3 Make uploads transactional and cleanup-safe

**Evidence:** `uploads::save` writes the file before inserting the `uploads` row. If the insert fails, the file is orphaned. Several content create/error paths can retain a newly written file when later validation or DB work fails.

**Fix:**

- Use a storage transaction/compensation helper: write to a quarantine name, insert the metadata, update the owning row, then finalize; delete the staged file on every error.
- Track ownership/reference count so replacing one record cannot delete a file still referenced elsewhere.
- Periodically reconcile filesystem and `uploads` rows, and audit cleanup failures.
- Validate file content as well as client MIME where practical; retain the current random names and extension allowlist.

## 7. P1 fixes: UI and architecture

### P1.1 Build role-specific workspaces from permissions

**Evidence:** `shell::nav_for` and `sections` are mostly role-based; the UI has Admin/Staff/Faculty/Student branches but no appointment-aware task permissions. The office dashboard displays overview counts while content-only restrictions are enforced only by target handlers.

**Fix:**

- Generate navigation and work queues from server-provided permitted actions, but keep every target handler independently authorized.
- Add separate Super Admin, IT Admin, Principal, HOD/Acting HOD, faculty, student, and office workspaces as required by the brief.
- Prioritize pending approvals, failed imports, result corrections, handovers, timetable problems, and deadlines; add notification/task tables. Opening a notification must re-check authorization.
- Standardize reusable table, form, status, validation, empty, loading, error, and confirmation components.
- Keep the public editorial design distinct from administration and maintain responsive/accessible behavior.

### P1.2 Fix navigation/content consistency

- Remove or implement “Soon” links; a visible feature should not lead to an unimplemented flow.
- Add a pending approval badge to the navigation/work queue.
- Add the missing catalogue course edit UI or clearly expose the existing POST endpoint.
- Make category options data-driven. `courses.category`, the offering form’s hard-coded list, free-text offering edit, and student `course_type` currently represent different taxonomies.
- Ensure all public and private pages use the same college timezone and publication predicate.

### P1.3 Harden third-party assets and response headers

**Evidence:** both base layouts load HTMX from unpkg and Google Fonts without SRI/CSP. `main.rs` sets nosniff, X-Frame-Options, and Referrer-Policy but not CSP/HSTS.

**Fix:**

- Prefer self-hosting pinned HTMX; otherwise pin a version with SRI and a narrowly scoped CSP.
- Add a CSP compatible with the actual HTMX/Tailwind/templates, at minimum `object-src 'none'` and controlled `script-src`, `style-src`, `img-src`, `connect-src`, and `frame-src` values.
- Add HSTS only when HTTPS is confirmed/configured, and document the production header policy.

## 8. P2 fixes: reliability, operations, and maintainability

1. Add periodic expired-session cleanup. The 8-hour inactivity expiry does not by itself remove expired rows from the sessions table.
2. Purge staged import batches/temporary credentials on a scheduled task, not only when another import starts; log purge failures.
3. Do not log generated demo passwords from `SEED_DEMO_USERS`; return them only through a controlled one-time operator channel and redact logs.
4. Replace `prompt_new_password(...).unwrap()` in `src/cli.rs` with error propagation; remove the unused `maybe_pw` binding and the duplicate audit statement.
5. Consolidate the duplicate `temp_password`/`temporary` helpers in `src/auth/password.rs`.
6. Consolidate the two date validators and add year/timezone tests.
7. Define a role-aware `safe_next` allowlist so a student cannot be redirected into unrelated management paths after login.
8. Review all ignored `let _ =` results, especially audit and purge operations. Do not hide failures in security-critical paths.
9. Add length/range validation to text fields and query limits; avoid unbounded user-provided strings in audit/content/import forms.
10. Add database health/readiness and migration reporting suitable for the separate listeners, without exposing sensitive database details through `/healthz`.
11. Update README/setup instructions for the two listeners, role appointments, migrations, private uploads, backup/restore, and test commands. The current README repeats the sign-in section and still describes incomplete features as later work.
12. Add protected backup/recovery procedures covering database, uploads, retention, restore verification, and access limited to Super Admin/operations. Do not put backups under `/static` or `/uploads`.
13. Review the `sqlx-postgres 0.7.4` future-incompatibility warning and all compiler warnings. Upgrade only after compatibility review; do not upgrade dependencies indiscriminately.

## 9. Migration sequence

Use additive, versioned SQLx migrations. Each migration should have a rollback/data-preservation note and a verification query where practical.

1. **Foundation and safety:** introduce audit metadata, session revocation/version, content publication predicate, appointment/permission tables, stable academic identity/version tables, and indexes. Add constraints after cleaning mock data.
2. **Data backfill:** map current admin/IT/faculty/HOD flags into appointments/permissions; map course rows to stable identities/versions; map offerings/enrolments/results; validate duplicates and unresolved relationships.
3. **Authorization cutover:** switch `AuthUser`, `Manager`, all protected handlers, and query services to appointment/scope decisions. Keep compatibility reads only for the duration of the backfill.
4. **Academic workflow:** add assessment/result state/history/correction tables, import staging tables, capacity/concurrency constraints, offering-target consistency, and promotion/enrolment events.
5. **Private content/uploads:** add content ownership/reference metadata and move downloads behind authorization/publication checks. Reconcile existing files before removing raw serving.
6. **Retirement:** only after verification, stop using legacy booleans/columns and old destructive paths. Do not drop historical tables or columns until an explicit retention decision and backup verification exist.

## 10. Required test plan

### Unit tests

- Scope decisions for every role, appointment state, delegation expiry, `NULL` department, and cross-department target.
- Course/version applicability, offering eligibility, capacity, selection transitions, locked selections, correction transitions, publication predicates, date/timezone helpers, import parsing, and marks validation.

### Database integration tests

- Apply all migrations to a clean PostgreSQL database and verify seed data.
- Run migration/backfill checks against the current mock schema, including history preservation.
- Assert foreign keys, unique active HOD/Acting HOD appointment, offering-target consistency, capacity under concurrent confirmation, audit append-only behavior, and no cascade deletion of academic history.
- Test transaction rollback for timetable writes, selection changes, result imports, content publication, and uploads.

### Authorization/isolation tests

- Every role against every protected route and mutation.
- Cross-department programme, course, offering, timetable, attendance, marks, exam, student, CMS, upload, and import IDs.
- Expired appointment, revoked delegation, deactivated account, transferred student, locked selection, finalized result, and invalid listener access.

### Workflow/regression tests

- Super Admin appointment, Principal appointment approval, HOD succession, acting HOD expiry, delegation handover.
- Catalogue/version mapping, offering approvals, student selection, HOD assignment, cohort finalization, capacity, timetable clashes, substitutions, attendance, promotion, and course history.
- Internal versus university results, result precedence, preview/confirm import, correction/supersession, publication visibility, and past-semester results.
- CMS draft/publish/schedule/unpublish/archive and direct URL/upload leakage.
- Known defects: department attendance route 500, HTMX attendance IDOR, UTC/IST transitions, offering substitution omission, report filter mismatch, offering clash detection, and newly created unassigned timetable records.

### End-to-end journeys

At minimum test:

1. IT Admin creates an account and publishes a CMS draft.
2. Super Admin appoints a Principal and a Principal approves an HOD/Acting HOD request.
3. HOD publishes an offering, receives approval, assigns/selects a student, and schedules it.
4. Faculty marks only an assigned/substituted class and cannot access another class.
5. Student sees only their enrolments, timetable, attendance, published result, and permitted requests.
6. Authorized staff import, preview, confirm, correct, finalize, and publish results without overwriting history.

## 11. Verification status and limitations

- The codebase and `redesign.md` were read directly, including routes, services, templates, migrations, setup files, and existing tests in source modules.
- No live PostgreSQL instance was available for this audit.
- `cargo check` and `cargo test --all-targets` could not be run in this environment because `cargo` is not installed. The deleted `audit.md` in `HEAD` records an earlier baseline of `cargo check` passing with 16 warnings and 19 unit tests passing; that is historical evidence, not a result of this audit run.
- Database runtime behavior, migration application, listener exposure, proxy configuration, and all end-to-end workflows remain to be verified after implementation.

## 12. Recommended implementation order

1. Listener separation, explicit authority model, and the `NULL` department scope fix.
2. Attendance IDOR/GET mutation fix, private uploads, session revocation, and audit policy.
3. Non-destructive database foundation and stable course/version relationships.
4. Timetable/offering/enrolment integrity, capacity locking, timezone repair, and known route/report defects.
5. Result states/history, import preview/confirmation, official-result precedence, corrections, and promotion history.
6. CMS authority/publication/download hardening and upload compensation.
7. Role-specific workspaces, notifications, reusable UI states, study materials, and accessibility polish.
8. Full migration, authorization, workflow, regression, and end-to-end verification; then update README and final report with actual command results.

## 13. Implementation pass 2 (2026-10-10)

Verified against a live PostgreSQL 16 (docker compose) and the running server on both listeners.

### Build and tests
- Fixed two compile errors: `src/config.rs` needed an explicit `SocketAddr` type for the management
  bind, and `src/auth/mod.rs` used a `let`-chain that the 2021 edition crate does not allow (rewritten
  as an explicit `is_some`/mismatch check, which also makes a session that predates the version column
  fail closed).
- Removed the warnings the compiler could see: the unused `session_layer` in `main`, an unused import
  in `teacher.rs`, a doc comment on an expression in `cli.rs`, and an unused handler argument in
  `people.rs` (now used to audit the credentials export).
- `cargo check --all-targets` is clean (only the historical `sqlx-postgres 0.7.4` future-incompat note
  and struct-field dead-code warnings remain). `cargo test --all-targets` passes 41 tests.

### Bugs found and fixed in this pass
1. **The IT-admin dashboard route was dropped in the listener split.** `Role::Admin::home()` is
   `/admin`, but after `b0e45e4` split the routers nothing registered `/admin`, so the post-login
   landing page returned 404. `management_router` now registers `/admin` (`OfficeOrAdmin`).
2. **`seed-demo-users` was not idempotent.** It looked students up through `find_for_login`, which
   signs students in by admission number, so the email lookup always missed and the second run failed
   on the unique email index. It now looks the seed accounts up by email.
3. **Department attendance save redirected to a nonexistent URL** (`.../{entry_id}?date={date}` when
   the route is `.../{entry_id}/{date}`).

### Feature implemented: study materials (P1.5)
The `study_materials` table (migration `0002`) now has a service, routes and UI.
- `src/services/materials.rs` — manager- and student-scoped queries. `scope_allows` is a pure,
  unit-tested function.
- `/admin/materials` (`Manager`): upload a file for a course, publish/archive, delete. The course id
  in the form is re-checked against the manager's department server-side, so an HOD cannot touch
  another department's course (verified: own-department upload 303, other department 403).
- `/hub/materials` (`StudentOnly`): published material for the courses the student is actively
  enrolled in.
- `/uploads/...` now authorizes a study-material file for the enrolled students of its course, the
  teachers of that course, and the IT admin; the rest get 404, and served files are `private` rather
  than `public` cacheable.
- Nav links added for IT admin, HOD and students; the section catalogues and their consistency tests
  were updated. Two new unit tests cover the scope rule.

### Also removed
- `services::people::delete_users`, the unused destructive cascade-delete superseded by
  `archive_users`, and the unused `routes::router` compatibility constructor.

### Still not implemented (unchanged from §12)
The appointment/permission cutover (the `role_assignments`/`delegations` tables exist and are
backfilled but no handler reads them yet), the result import preview/commit UI, the course-version
migration, login rate limiting, backup automation, and the full integration/end-to-end suite.
Teacher (non-HOD) upload of study materials is now wired — see §14.

## 14. Implementation pass 3 (2026-10-10)

Two more items from the brief, both verified against the running server.

### Login rate limiting (P0.6)
- `src/state.rs` gains a `LoginLimiter`: an in-memory sliding window keyed on **both** the account
  identifier and the client address, so one IP cannot grind one account and one account cannot be
  hammered from many. The reply for a throttled attempt is a generic "too many sign-in attempts", the
  same shape as a wrong password, so it does not reveal whether an identifier exists. It is checked
  before any account lookup.
- Policy: 20 attempts / 5 min per account, 60 / 5 min per address.
- The peer address comes from `ConnectInfo`, via
  `into_make_service_with_connect_info` on both listeners. Forwarded headers are deliberately ignored.
- Verified live: the address budget blocks after 60 attempts in the window, and the account budget
  blocks the 21st attempt with the same identifier. Three unit tests cover the sliding window.
- This complements, and does not replace, the existing database lockout (5 wrong passwords → 15
  minutes); the account limit is set above it so the DB lockout is what a genuine user meets first.

### Teacher study-material upload (completes P1.5)
- `/teacher/materials` (`TeacherOnly`) lets a teacher upload a file for a course they are attached to
  (course owner, or a timetable period). The course id is re-checked with `materials::teacher_teaches`,
  so a forged course id is a 403.
- A teacher's upload is always saved as a **draft**; publishing stays with the head of department or
  the IT admin at `/admin/materials`. This matches the brief's "teachers create drafts, higher
  authority publishes".
- Verified live: `teacher1` (who teaches course 1) sees only course 1, uploads a draft (303), and is
  refused course 2 (403); the HOD publishes that draft (303); a teacher who teaches nothing is refused
  for every course (403).

### Finding that blocks the appointment cutover (P0.2)
The live database shows `role_assignments` is **out of sync** with the `faculty.is_hod` columns: the
0021 compatibility backfill ran once, but the demo seeder and the people tools change `is_hod`
afterwards. In the current dev data user 2 (a plain teacher) holds an active `hod` assignment for
department 1, while the real HOD (user 7) holds none. Cutting `Manager` over to `role_assignments`
today would therefore both over-grant (user 2) and under-grant (user 7). The cutover must be preceded
by a reconciliation (a trigger or a backfill plus keeping the writers in step), and is left for a
focused pass.

## 15. Implementation pass 4 (2026-10-11)

The remaining items from the brief, plus two bugs that live verification found in pass 3's own work.
Everything below was run; the commands and their real results are in the notes.

### P0.2 — the appointment cutover, unblocked
`role_assignments`/`delegations` are now what decides departmental authority, not the legacy booleans.
- `services::appointments` resolves the effective department from an **active, in-date** head assignment
  or an **active, in-date** delegation whose granting assignment is still live. A user who has
  appointment rows but none effective is *denied* — an expired appointment must not fall back to
  `faculty.is_hod`. Only a user with no appointment rows at all (an account predating the model) still
  reads the boolean.
- Migration `0023` reconciles the drift pass 3 found: it revokes head assignments whose faculty is no
  longer a head, and gives every current head an active assignment unless the department already has
  one. `sync_hod_from_flags` is called by every writer of `faculty.is_hod` (seeder, people tools,
  grant/revoke), so the two cannot drift again.
- `one_active_department_head` (a partial unique index) enforces one active head per department; a
  second appointment in a transaction is refused with `23505`.
- Ending an appointment revokes it, clears the legacy flag, and opens a handover task. Ending the
  granting assignment also stops every delegation derived from it.
- Verified live: grant, one-per-department, revoke, delegation, and handover; and the reconciliation
  repaired the stale rows (teacher1's bogus `hod` revoked, the real HOD restored).

### P1.1 — staged result imports (preview before commit)
- `services::result_import` stages an uploaded CSV (or the guided marks sheet) in
  `result_import_batches`/`result_import_rows`, keeping the raw text in `payload` so the commit
  re-validates the exact rows that were previewed rather than trusting a second upload.
- The preview shows every row's outcome and blocks the commit while a conflict with a **finalized**
  mark exists; the finalized row is never touched.
- Verified live: preview, commit, and a finalized conflict refusing the commit.
- Migrations `0024` (payload) and `0025` (guided assessment/exam name).

### P1.4 — course versions
- `course_versions` (migration `0026`) is an append-only history beside `courses`; every existing course
  became version 1, and `course_offerings` records the version it was built from.
- `GET/POST /admin/courses/:id/edit` is the catalogue edit UI the README listed as missing (#8), with a
  version-history panel. Saving appends a version when anything changed and is a no-op when it did not.
- Verified live: editing course 1 appended version 2, marked version 1 non-current, and offering 1 stayed
  pinned to version 1 ("Python") after the rename.

### Backups and housekeeping
- `services::backup` writes `backup-<UTC>/` with `database.dump`, `uploads.tar.gz` and a manifest of
  sizes and SHA-256 sums; retention keeps the newest `BACKUP_KEEP`. It refuses a directory inside
  `static/` or the upload directory and creates the run directory `0700`. It prefers `pg_dump` and falls
  back to `docker compose exec` via `BACKUP_DOCKER_SERVICE`.
- `services::maintenance` runs hourly: expired sessions, the student-import purge, result-import expiry
  (dropping the raw payload) and 90-day batch cleanup, plus the due backup.
- `cargo run -- backup [--list]` and `cargo run -- maintenance`.
- Verified live: a backup wrote a 191 690-byte dump and a 56 080-byte uploads archive; `pg_restore
  --list` read 442 TOC entries back, the manifest parsed as JSON, and the tar listed real files. The
  scheduler did not re-run it, because the last one was inside the 24-hour window.

### Two bugs live verification found in pass 3
- **Session cleanup never worked.** `purge_expired_sessions` deleted from `session`, but `tower-sessions`
  keeps its table in the `tower_sessions` schema, which is not on the connection's `search_path`, so
  every run failed with `42P01`. The query now names `tower_sessions.session`.
- **A container backup always wrote an empty dump.** The `docker compose exec` dump used
  `Command::output()`, which replaces the stdout redirect with a pipe, so `database.dump` stayed empty
  and the run was correctly rejected as "the dump was empty". It now spawns the child with the file as
  its stdout, captures only stderr, and waits on it.

### Ops hardening
- **Input length limits** (`src/validate.rs`): every form field is now bounded before the database
  (`MAX_TITLE`, `MAX_NAME`, `MAX_CODE`, `MAX_NOTE`, `MAX_BODY`, `MAX_EMAIL`, `MAX_PASSWORD`), counted in
  characters. Applied to login, the content forms (notices, news, events, documents, pages), materials
  and both course forms. Verified live: a 250-character course title is refused with a form message and
  writes no version.
- **One date validator** (`src/dates.rs`). There were three copies — `routes::admin::is_valid_date`,
  `routes::admin::exams::valid_date` and `services::attendance::valid_date` — and they disagreed
  (only one had a year range, only one trimmed). All three now delegate to one function.
- Confirmed already sound: the post-login redirect is already role-allowlisted (`safe_next_for_role`,
  same-site relative paths only), and seeded credentials are already kept out of the log — the seeder's
  output goes to a `0600` file or is discarded, and the start-up path only logs the account count.

### P1.3 — front-end hardening
- **HTMX is self-hosted** at `static/js/htmx.min.js`; the two CDN `<script>` tags are gone.
- **Content-Security-Policy** added in `apply_layers`, along with `Permissions-Policy` and (when
  `COOKIE_SECURE=true`) HSTS. `script-src 'self'` is the strict part, so every inline `on*` handler and
  the one inline `<script>` block were replaced by `data-*` hooks handled in `app.js`. `style-src` keeps
  `'unsafe-inline'` for the few per-row CSS values (documented in the README).
- The dead **"Soon"** nav mechanism was removed: no item used it, so the `soon` field, its parameter and
  the greyed-out badge are gone.
- **Course categories are data-driven**: the list lives once in `services::courses::CATEGORIES` and both
  catalogue forms render from it, instead of three hardcoded `<option>` lists.

### lib split and the test suite
- `main.rs` is now only process setup and the listeners; the application is `src/lib.rs`, which the tests
  build against. `apply_layers`, `connect`, `migrator` and `init_session_store` are library functions.
- **`tests/database.rs`** (16 tests) uses `#[sqlx::test]`, so each test gets a database with every
  migration applied, and asserts the constraints and the services: tables and successful migration
  history; one active head per department; an expired appointment denying instead of falling back; the
  legacy-flag path; a delegation ending with the appointment that granted it; acting appointments;
  handover on revoke; the append-only audit log; one version per new course; append-on-edit keeping
  history; offering version pinning; the `NOT NULL` that stops a versionless offering; the category CHECK
  agreeing with `category_is_valid`; the marks workflow states; and the staged result-import payload,
  state and cascade.
- **`tests/app.rs`** (8 tests) drives the real layered router with `tower::oneshot`: the security headers
  and CSP, HSTS only when secure, `robots.txt`, the login page's CSRF field and local HTMX, the served
  HTMX bundle, a 404 for an unknown path, an unauthenticated `/hub` redirect, and the management router.
- Result: `cargo test` — **57 unit + 16 database + 8 end-to-end, all passing**. The only build warning is
  the pre-existing `sqlx-postgres v0.7.4` future-incompatibility notice from the dependency.

### Still not done
The remaining known limitations in the README (offering-course results filtering, offering clash
detection between two offerings of the same programme, attendance-report filters under a programme,
capacity enforcement outside student confirmation, and student-action auditing). They are unchanged and
listed there with detail.
