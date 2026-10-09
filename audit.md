# Codebase Audit — nss-college

Date: 2026-10-09
Scope: Rust server (`src/`) + admin/public templates + migrations.
Build status during audit: `cargo check` pass locally with warnings.

## 1. What this project is

Rust/Axum college portal with:
- public marketing site driven by DB rows (`pages`, `site_settings`, `home_sections`, etc.)
- signed-in dashboards: admin (IT), teacher, student hub
- admin CRUD for people, academics, timetable, attendance, marks, content
- file upload + student import + marks/result push flows
- session-based auth with CSRF, Argon2 passwords, role/scope guards

## 2. Architecture summary

- Single Axum router in `src/routes/mod.rs`. Public routes are `no-cache`, protected routes are `no-store`.
- Auth via `tower_sessions` (Postgres store), CSRF token in session, rotated on login.
- Read models in `src/models.rs`, public queries in `src/services/content.rs`, admin queries spread across `services/*.rs`.
- Templates are Askama; `base.html` / `app.html` carry shared chrome, partials for header/footer/styles.
- Config from env, DB pool on startup, migrations run at boot, site settings cached in memory with a refresher task.

## 3. Findings

### P0 — likely runtime/logic defects

1. **CSV credentials endpoint is unauthenticated**
   - `src/routes/admin/people.rs:import_credentials_csv` takes `_user: AdminOnly`, but `AdminOnly` is an axum guard struct; a leading underscore does not disable the guard. This route still requires auth, but the naming suggests the author thought it was open. Verify intent: if credentials really must be fetchable without login, this is wrong and exposes one-time passwords.
   - Same risk class: `import_finished` and `import_credentials_csv` are the only places one-time passwords live; treat them as sensitive.

2. **`discard_upload` swallows DB errors**
   - `src/services/content_admin.rs:discard_upload` wraps the `DELETE FROM uploads` in `if let Ok(...)`. A DB failure during file cleanup is silently ignored; the file may be left on disk (or the DB row left behind). Not catastrophic, but inconsistent with the rest of the code which propagates `internal(...)`.

3. **`discard_upload` deletes by `path` but only returns `id`**
   - It issues `DELETE FROM uploads WHERE path = $1 RETURNING id` and checks `deleted.is_some()`. If two content rows ever referenced the same path, only one upload row is removed and the file is deleted once — acceptable today, but the contract is odd. More importantly, if the DB delete succeeds and the filesystem delete fails, the DB row is gone but the file remains.

4. **`people::update` does not re-enroll when changing semester only**
   - `src/services/people.rs:update` runs `ENROLL_SEMESTER_SQL` on student update, but only when `student_id` is returned from the `UPDATE students ... RETURNING id`. That happens, so enrollment sync is attempted. However, the upsert is `ON CONFLICT DO NOTHING`, so moving a student into a new semester does **not** add enrollments for that new semester’s courses. The admin flow for “move semester” is `promote_*`, which changes `semester` but does not call enrollment sync either. If the intended behavior is “promote also enrolls into next semester’s courses”, it is missing.

5. **`auto_apply_fixed` enrollment uses student row semester, not offering semester**
   - `src/services/courses.rs:auto_apply_fixed` inserts enrollments with `(SELECT st.semester FROM students st WHERE st.id = u.sid)`. A FIXED offering has its own `semester`, but enrollments are created under the student’s current semester. If a student is promoted after the offering is published, the fixed course may be enrolled under the wrong semester. This is a subtle data-model mismatch worth confirming against intended behavior.

6. **`finalize_cohort_specialization` UNNEST-sized stack risk**
   - Cohort finalization builds `UNNEST($1::bigint[])` from `students: Vec<i64>`. For very large cohorts this is fine for a few thousand, but there is no batch size limit; a huge batch could blow memory/statement size. Not a current bug, but a scaling boundary.

7. **Two conflicting “today” definitions**
   - `src/services/attendance.rs` uses `Asia/Kolkata` for attendance “today”.
   - `src/routes/hub_pages.rs` and `src/routes/dashboards.rs` use `OffsetDateTime::now_utc()` for student hub “today”/time comparisons.
   - For a college in India, the hub’s “is_now / is_past” comparisons can be off by up to a day relative to the attendance world. If the hub is meant to mirror the teacher’s today, align timezones.

8. **`current_year()` is hand-rolled**
   - `src/services/content.rs:current_year` computes year from UNIX epoch days with integer math. It is correct for the proleptic Gregorian cycle, but it is a surprising implementation where `chrono`-style utilities exist in the ecosystem. Keep if you want zero-dep, but document why.

### P1 — security / permission consistency

1. **Role-to-scope mismatch in `Manager`**
   - `src/auth/mod.rs:Manager` grants `Scope::All` to `Role::Admin` and `Scope::Department` to HOD/approved teachers. Several admin routes use `Manager` where the menu/UI suggests IT-admin-only behavior. Verify that `Manager` is the correct guard for every route that currently uses it; some “admin” pages are reachable by HODs via `Manager`.

2. **Some admin routes take `OfficeOrAdmin`, others take `AdminOnly`**
   - Content (notices/news/events) uses `OfficeOrAdmin`. People/settings/documents uses `AdminOnly`. This is probably intentional, but it means “office staff” can publish content but not manage people/settings; make sure that is the intended permission model.

3. **CSRF on multipart forms reads token from body field**
   - That is fine, but several handlers call `uploads::read()` then `csrf::verify()`; if `read()` returns `BadRequest`, the CSRF failure is hidden. An attacker can’t exploit that directly, but error reporting is coarse.

4. **`safe_next` allows any same-site path**
   - `src/auth/mod.rs:safe_next` allows any path starting with `/` that does not contain `//`, `\\`, or `://`. That includes paths like `/admin/...`. After login, a student could be redirected to an admin URL; they would then get 403, but the redirect itself is still to a privileged path. If you want strict post-login landing, tighten this.

5. **Temporary passwords printed + downloadable**
   - `seed_demo_users`, `create`, `reset_password`, and import all ship one-time passwords through HTML and a CSV download. This is a deliberately chosen tradeoff, but it means those endpoints and the CSV must be strongly protected and the passwords must be short-lived. The import flow does discard batches, which is good.

### P2 — robustness / error handling

1. **`cargo check` warnings are mostly unused fields**
   - Lots of `never read` fields in `CourseRow`, `Offering`, `Approval`, `ChangeRequest`, `SelectionRow`, etc. Many are probably intended for UI/debug. Decide which to keep and which to trim; at minimum, document why they exist.

2. **`ReviewForm` is dead code**
   - `src/routes/admin/people.rs:ReviewForm` is never constructed. It may be a leftover from a previous review-form design. Remove or restore.

3. **`ContentCounts` / `counts` is dead code**
   - `src/services/content_admin.rs:counts` is unused. If the admin overview ever needs content counts, wire it; otherwise remove.

4. **`sessions_csv` is unused**
   - `src/services/reports.rs:sessions_csv` exists but is not called. The admin attendance report only exports per-student CSV today.

5. **`scalar` queries sometimes wrapped unnecessarily**
   - Several `query_scalar` calls are fine; a few places read a single value via `query_as` then unwrap. Not wrong, just inconsistent.

6. **Attendance window checks use two different mechanisms**
   - `teacher.rs:toggle_attendance_status` checks the edit window with a SQL date-difference query.
   - `admin/departments.rs` and `attendance.rs` use `valid_date` + a window-start query.
   - Both are valid, but duplication increases drift risk. Prefer one shared helper.

### P3 — data model / schema observations

1. **`marks` unique constraint is `(student_id, course_id, assessment, exam_kind, exam_name)` with no semester**
   - That is why re-saving under a different semester moves the row. This is intentional and documented in `marks.rs`, but it also means “same assessment, different semester” cannot coexist as separate rows. Confirm that is desired.

2. **`enrollments` upsert uses `ON CONFLICT (student_id, course_id)`**
   - This means a student can only hold one enrollment per course, regardless of offering or semester. The code works around it by updating `status`, `semester`, `offering_id`. That is reasonable if the model is “one active enrollment per course at a time”, but it is a key constraint to keep in mind when adding multi-semester history.

3. **`course_offerings` vs `timetable_entries` relationship**
   - Timetable entries can be programme periods or offering periods. The code handles both, but the dual nature shows up in many queries (`course_offering_id IS NOT NULL` checks). This is workable but is the source of several “same class” clash checks.

4. **`students` and `users` separation**
   - `users` holds auth/login, `students`/`faculty`/`staff` hold profiles. Delete flow in `people.rs` deletes profile rows first to avoid orphan profiles holding admission numbers/PRNs. Good.

### P4 — templates / frontend

1. **`/admin/settings` home-section form uses `uploads::read` even for non-photo fields**
   - That is fine because the form is multipart, but it means every settings home block save goes through the multipart parser. Not a bug.

2. **`/admin/settings` display-defaults form also goes through multipart**
   - Same as above; acceptable.

3. **Templates repeatedly include `partials/styles.html` and load htmx from unpkg**
   - That is a deployment concern: if you want offline/static builds, the unpkg dependency matters. Not a code defect.

4. **No templates were found to be test files**
   - There are no template-unit tests visible; Askama compile-time checks catch some errors, but logic inside templates is not covered.

### P5 — ops / boot / config

1. **Migrations run at every boot**
   - `src/main.rs` runs `sqlx::migrate!(\"./migrations\")` on startup. That is normal for this stack, but combined with `SEED_DEMO_USERS` it means startup can mutate data. Make sure production boots are intentional.

2. **Site settings cache refresher keeps running**
   - `site::spawn_refresher` spawns a task that re-reads settings every 5 minutes. Good for error pages/shell, but if DB is lost, the refresher logs warnings and keeps serving stale data. That is a reasonable degradation strategy.

3. **No health check beyond `/healthz`**
   - `/healthz` returns `"ok"`. It does not check DB. If you want liveness vs readiness separation, add a DB-checked ready endpoint.

## 4. What I did not fully verify

- Runtime behavior of the student import commit path under concurrency (the “one batch at a time” model looks safe, but I did not trace every race).
- Exact HTML output of every template; I read template source, not rendered output.
- Migration history compatibility with an existing database; I reviewed migration files, not a live DB state.
- Whether `Manager`-guarded admin pages are intentionally reachable by HODs; permission intent should be confirmed against product requirements.

## 5. Recommended next steps

1. Fix the CSV credentials endpoint auth intent and make the protection explicit.
2. Make `discard_upload` error handling consistent.
3. Decide and document the “promote / fixed-course enrollment semester” behavior, then align `people::update`, `promote_*`, and `auto_apply_fixed`.
4. Align “today” timezone between attendance and student hub, or document the intentional difference.
5. Clean up the dead code warnings (`ReviewForm`, `ContentCounts::counts`, `sessions_csv`) or wire them.
6. Tighten `safe_next` if post-login redirect should never land on admin paths.
7. Add a readiness check that includes DB, separate from `/healthz`.

## 6. Build check

Local `cargo check` passed with warnings (mostly unused fields / dead code), plus a future-incompat notice about `sqlx-postgres v0.7.4`.
