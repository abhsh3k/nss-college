# Codebase Audit — nss-college

Date: 2026-10-09 (full rewrite — second, deeper pass)
Scope: Rust server (`src/`), Askama templates, SQL migrations, build/ops files.
Method: static reading of every module in `src/`, all migrations, template scan,
pattern searches (swallowed errors, dynamic SQL, guard usage, CSRF coverage),
`cargo check` and `cargo test`. **No live database was available**, so runtime
behaviour was inferred from code; items that need a live DB to confirm are
marked as such.

Build status during audit: `cargo check` passes with **16 warnings**; `cargo test`
passes (19 unit tests, all CSV-parsing only). `sqlx-postgres 0.7.4` emits a
future-incompatibility notice.

---

## 1. What this project is

Rust/Axum college portal:

- public marketing site rendered from DB rows (`pages`, `news`, `notices`, `site_settings`, home sections),
- signed-in dashboards for admin (IT), office staff, teachers (incl. HOD tools), students (hub),
- admin CRUD for people, academics, course offerings/selections, timetable, attendance, marks/exams, content,
- file uploads, CSV/Excel student import, marks/result push flows,
- session auth (Postgres-backed `tower_sessions`) with CSRF, Argon2 password hashing, role/scope guards.

## 2. Architecture summary

- Single Axum router (`src/routes/mod.rs`): public routes get `Cache-Control: no-cache`, signed-in routes `no-store`.
- Auth: `AuthUser` extractor re-reads the user from the DB per request; role guards (`AdminOnly`, `OfficeOrAdmin`, `TeacherOnly`, `StudentOnly`) plus a `Manager` guard that grants HODs/delegates department-scoped academic powers.
- Read models in `src/models.rs`; public queries in `services/content.rs`; admin queries spread across `services/*.rs`; templates are Askama.
- Config from env; migrations run at boot; site settings cached in memory with a 5-minute refresher.

## 3. What the previous audit flagged as fixed (verified in this pass)

1. CSV credentials endpoint: binding is now `user: AdminOnly` (`routes/admin/people.rs:1141`) — the guard was always enforced; the confusing `_user` name is gone. (But it now produces an unused-variable warning — see P3-2.)
2. `discard_upload` (`services/content_admin.rs:425+`) logs all three outcomes with `tracing::warn!` instead of silently swallowing DB errors.
3. Hub "today"/weekday: `/hub/timetable`, `/hub/results`, `/hub/exams` now derive date and weekday from the college-local (Asia/Kolkata) path (`routes/hub_pages.rs:38-46`). **Partial — see P1-2.**
4. Dead code removed: `ReviewForm`, `ContentCounts::counts`, `sessions_csv` are gone.

Also note: the old audit's headline "CSV credentials endpoint is unauthenticated" was a false alarm — `AdminOnly` always applied (a leading underscore on a binding does not disable an axum extractor). The rewrite corrects that record.

---

## 4. Findings

### P1 — security / correctness bugs to fix soon

**P1-1. HOD scope collapses to "all departments" when `faculty.department_id` is NULL.**
`Manager` builds `Scope::Department(p.department_id)` from the faculty row
(`src/auth/mod.rs:238-277`). `department_id` is `Option<i64>` and NULL is a legal
value (`migrations/0001_core.sql:49` even has `ON DELETE SET NULL`). But every
scope helper treats `None` as *unrestricted*:

- `may_manage_programme` / `may_manage_teacher` (`services/academics.rs:69-78, 209-223`):
  `let Some(want) = department else { return Ok(true); }` — **None = allow everything**,
- `departments::place` (`routes/admin/departments.rs:126-155`): `if let Some(own) = ...` skips the check entirely when None,
- `ensure_our_department` (`routes/admin/departments.rs:221-234`): same pattern, returns `Ok(())` for None.

Meanwhile the doc comment on `Scope::Department` (`src/auth/mod.rs:~196`) says
"`None` when the teacher has no department set yet, **which grants nothing to
manage**" — the implementation does the exact opposite. Consequence: a teacher
with `is_hod` or `can_manage` set and no department gets IT-admin-equivalent
power over marks, exams push, timetable, substitutions, department placement
and department attendance for **every** department. (`course_offerings` handlers
that compare `manager.department() != Some(o.offering_department_id)` are safe,
so the escalation is partial but severe.)

Two ways to reach the state, one of them silent:
- the People form allows ticking "head of department"/"can manage" with no department chosen (`department_id = NULLIF($4, 0)`, `services/people.rs:290`),
- **deleting a department** `ON DELETE SET NULL`s its faculty's `department_id` — every HOD of that department silently becomes a global manager.

Fix: make `Scope::Department(None)` deny (per the documented intent), and/or
refuse to save HOD flags without a department.

**P1-2. Timezone split is only half-fixed; student-facing "now" indicators are wrong by 5h30.**
The earlier fix moved the hub's *date/weekday* to Asia/Kolkata, but:

- `hub_pages::now_parts` (`routes/hub_pages.rs:38-46`) still takes `now_hm` from `OffsetDateTime::now_utc()` — so `is_now`/`is_past` in today's timeline compare **UTC** HH:MM against **IST** timetable times all day long,
- the `/hub` overview (`routes/dashboards.rs:254-349`) was never touched: `today_str`, `now_hm` and `weekday_today` all still come from `now_utc()` (line 260),
- the teacher sheet page (`routes/dashboards.rs:78-137`) also uses UTC date/weekday,
- **`get_attendance_sheet` (`routes/teacher.rs:47-128`) creates attendance sessions dated with the UTC date** — between 00:00 and 05:30 IST it files sessions under *yesterday*, diverging from `attendance::today` (the `Asia/Kolkata` constant at `services/attendance.rs:8`),
- the attendance edit-window check in `toggle_attendance_status` (`routes/teacher.rs:166-178`) uses `CURRENT_DATE` (DB session timezone) rather than the Kolkata `TODAY` constant used by `attendance::window_start`,
- the admin report's default month (`routes/admin/attendance.rs:34-41`) also starts from the UTC date.

Fix: funnel every "today"/"now" through one helper (the Kolkata SQL path or a
Rust equivalent) and use it in all six places.

**P1-3. The HTMX attendance endpoints have no ownership check — any teacher can mark any class.**
The classic flow (`teacher::mark_form` / `mark_save`, `routes/teacher.rs:253-365`)
correctly scopes through `attendance::period(fid, entry_id, ...)` so a teacher
can only touch their own or substituted periods. The newer HTMX sheet does not:

- `get_attendance_sheet` (`routes/teacher.rs:47-128`) takes any `entry_id`, reads its `course_id`, and **upserts an attendance session for it** — no check that the caller teaches that entry. It is also a *GET that mutates state* (no CSRF; a prefetch or img tag can create rows).
- `toggle_attendance_status` (`routes/teacher.rs:137-205`) only checks the date window, then upserts `attendance_records` for **any** `session_id` and **any** `student_id` (the FK only requires the student exists — not that they are on that course's roster).
- The sheet's roster query (`routes/teacher.rs:95-117`) also skips the offering narrowing that `attendance::roster` (`services/attendance.rs:~243-285`) applies, so offering periods list every student ever enrolled in the underlying course.

Any teacher can therefore view rosters of, create sessions for, and rewrite
attendance of every other teacher's classes. Fix: route both handlers through
the same ownership + roster checks as `mark_save` (session's entry must be the
caller's or a substitution covering them; student must be on the roster), and
make the session creation a POST or derive it inside `mark_save`.

**P1-4. `/admin/attendance` likely 500s whenever sessions exist in the period (needs live-DB confirmation).**
`reports::by_session` (`services/reports.rs:106-118`) selects `sess.on_date`
(DATE) **and** `to_char(sess.on_date, 'YYYY-MM-DD') AS on_date` — two result
columns named `on_date`. `SessionSummary.on_date: String` (line 39) is decoded
from the *first* match; sqlx cannot decode a Postgres `DATE` into `String`, so
the decode should fail at runtime the moment any row comes back. With zero
sessions in range there are no rows to decode, which would hide the bug in
casual testing. Fix: drop the raw `sess.on_date` column (the struct field is
already `#[allow(dead_code)]`) or alias it distinctly.

**P1-5. Promotion moves the semester but never enrolls the new semester's courses** *(carried over — still unfixed)*.
`promote_one` / `promote_cohort` (`services/people.rs:168-199`) only bump
`students.semester`. `ENROLL_SEMESTER_SQL` (`services/people.rs:97-113`) runs on
create/update only. Since timetable, attendance rosters and the hub follow
enrollments, a promoted class shows empty rosters until each student is
individually re-edited. Decide the intended behaviour and either call the
enrollment sync from `promote_*` or make the timetable fall back to
programme+semester courses.

**P1-6. `auto_apply_fixed` enrolls under the student's current semester, not the offering's** *(carried over — still unfixed)*.
`services/courses.rs:1287-1294` (and the other three enrollment upserts at
809-818, 1107-1116, 1438-1446) insert `(SELECT st.semester FROM students ...)`.
A FIXED offering published for semester N enrolls/promotes students under
whatever semester they happen to be in, and the `ON CONFLICT ... DO UPDATE SET
semester = EXCLUDED.semester` will *rewrite* an existing enrollment's semester
when the page re-applies. Confirm intent; the offering row has its own
`semester` column that is ignored here.

### P2 — should fix

**P2-1. Deleting a user permanently destroys academic history.**
`people::delete_users` (`services/people.rs:336-356`) deletes the `students`
row; `marks`, `enrollments`, `attendance_records`, `student_course_selections`
all `ON DELETE CASCADE` from it (`migrations/0002:21,50`, `0006:19,48`,
`0013:148`). The audit log keeps only the numeric id. Consider soft-delete
(`is_active = false`, which already exists) or archiving instead of hard delete,
and block deleting the **last active admin** (today an admin can delete another
admin; only self-delete is prevented, `routes/admin/people.rs:1216-1240`).

**P2-2. Duplicate audit row on every CLI user creation.**
`src/cli.rs:89` contains the same `users::audit(...)` statement twice on one
line ("defensive re-audit") — each CLI-created user writes **two**
`user_created_cli` audit entries, and the formatting (lines 89-92) shows a
mangled edit. Keep one call.

**P2-3. One-time passwords land in logs when seeding.**
`main.rs:76-87` logs every `seed_demo_users` line at `warn!`, and those lines
contain the generated temporary passwords. The env gate makes this acceptable
today, but log aggregation will retain them. Print to stdout or redact, and
never ship with `SEED_DEMO_USERS` set.

**P2-4. No SRI on CDN scripts, no CSP, no HSTS.**
`templates/layouts/base.html:16` and `app.html:12` load htmx from unpkg with no
`integrity` attribute (supply-chain exposure on every page, including admin);
Google Fonts is also third-party. `main.rs:104-130` sets nosniff,
X-Frame-Options and Referrer-Policy (good) but there is no
`Content-Security-Policy` and no `Strict-Transport-Security`. Minimum: pin htmx
with SRI or self-host it under `/static`, add HSTS when `COOKIE_SECURE=true`,
and consider a CSP that at least forbids `object-src`/`frame-src`.

**P2-5. No CI.** There is no `.github/workflows` (or other pipeline). The 19
unit tests cover only CSV parsing — nothing exercises the auth guards, the
scope logic (see P1-1), the timezone helpers (P1-2) or marks validation, i.e.
exactly the areas where the real bugs live. Even a `cargo check` + `cargo test`
+ `cargo clippy` workflow would have surfaced several items below.

**P2-6. Import staging keeps plaintext temp passwords longer than needed.**
`import_rows.temp_password` (`migrations/0008:43`) holds generated passwords
until the batch is discarded (`import_finished`, `routes/admin/people.rs:1185-1200`)
or swept by `purge_stale_import_batches` (7 days). But the sweep only runs when
a *new* import is uploaded (`routes/admin/people.rs:742`, `let _ =` — failure is
also silent). If nobody imports again, stale batches sit until they do. Schedule
the function (e.g. with the settings refresher task) and log sweep failures.

**P2-7. Sessions table grows forever.**
`main.rs:90-102` configures `Expiry::OnInactivity(8h)` but nothing ever calls
the store's `delete_expired`; expired rows accumulate in Postgres. Add a
periodic cleanup task (and consider an absolute session lifetime in addition to
the idle timeout).

**P2-8. Files orphaned when the DB insert fails.**
`uploads::save` (`src/uploads.rs:183-235`) writes the file to disk *before* the
`INSERT INTO uploads`; if the insert fails the file is left behind with no row.
Reverse the order (insert with a transaction, or delete the file on error).

**P2-9. CSRF is verified *after* the multipart body is fully parsed.**
Every multipart handler (`uploads::read` then `csrf::verify`, e.g.
`routes/admin/notices.rs:165-198`, `people.rs:706-710`) buffers up to 10 MB
before rejecting a bad token. Not exploitable for CSRF (the check still
happens), but it lets an attacker with a victim's cookie burn CPU/memory with
unauthenticated-shaped bodies. Extract the token from the first field and bail
early, or verify a header token first.

**P2-10. Error messages are logged but never shown for propagated `BadRequest`.**
`AppError::BadRequest(msg)` (`src/error.rs:44-52`) logs the message and renders
a generic 400 page. Most forms catch these themselves, but anything that
propagates (e.g. `uploads::read` failures) gives the user no reason. Include the
message in the 400 template (it is server-generated, not user input).

### P3 — cleanup / observations

1. **16 compiler warnings**, including two the previous fix *introduced* or left behind: unused `user` at `routes/admin/people.rs:1141` (rename back to `_user` with a comment, or pass it through), unused `maybe_pw` at `src/cli.rs:66`, an unused doc comment at `src/cli.rs:189`, and ~13 never-read struct fields across `services/*`. `cargo fix` offers 2 suggestions.
2. `password.rs` defines **two identical** functions, `temp_password()` (line 58) and `temporary()` (line 70); the import flow uses one, everything else the other. Delete one.
3. Two independent date validators have drifted: `attendance::valid_date` (`services/attendance.rs:49-64`, year range 2000-2100 enforced) vs `admin::is_valid_date` (`routes/admin/mod.rs:246-273`, no year bound). Keep one.
4. `safe_next` (`src/auth/mod.rs:280-286`) still accepts `/admin/...` — a student logging in via `?next=/admin` lands on a 403 page. Harmless but sloppy; allow-list by role home.
5. `finalize_cohort_specialization` passes a whole cohort as one `UNNEST($1::bigint[])` (`services/courses.rs`) — fine for a college, a memory/statement-size boundary to remember.
6. `enrollments` remains unique on `(student_id, course_id)` only; re-saving a marks sheet under a different semester *moves* the row because `marks_one_per_assessment` (`migrations/0019`) has no semester component. Both are deliberate (documented in-code) but worth a product-owner confirmation.
7. Login has no per-IP rate limit; the per-account lockout (5 tries → 15 min, `services/users.rs:62-76`) is the only throttle, so admission numbers are enumerable/lockable by an attacker who knows them. Fine for a small portal; note it.
8. `let _ =` silently ignores failures in a handful of non-critical calls (`purge_stale`, some audit writes) — acceptable, but audit-write failures are *fatal* (`?`) in other handlers after the main action already committed; a failed audit insert there returns a 500 for work that did succeed. Pick one policy.
9. `find_for_login` (`services/users.rs:29-44`) uses `LIMIT 1` without ORDER BY over a two-branch OR — a staff email that equals a student's admission number would match two rows nondeterministically. Practically impossible; noting it for completeness.
10. Attendance percentages (`hub::course_attendance`, `services/hub.rs:~141-160`) count *all* sessions of a course row with no semester/time filter, so historic cohorts sharing a course row dilute today's percentages. Verify against how courses are created per batch.

### Verified-good (keep doing this)

- **No SQL injection surface found**: every `format!`-built query interpolates only constants (`TODAY`, `COLUMNS`, `NEWS_COLUMNS`, `FROM_WHERE`); all user values go through bound parameters.
- **XSS**: Askama autoescapes everywhere; no `| safe` in any template; no `innerHTML`/`eval` in JS. DB HTML bodies are escaped (which also means no rich text — a product limitation, not a hole).
- CSRF tokens on every state-changing route checked (86 call sites verified); token rotated on login; session id cycled on login and password change (fixation-proof); logout flushes.
- Argon2 with `spawn_blocking`, dummy-hash timing equalisation for unknown accounts, forced password change (`Ready` guard) for temporary passwords.
- Uploads: extension chosen server-side from a MIME allowlist (no SVG), random filename, `..`-check before any disk delete, 8 MB/10 MB caps, body limits per router group.
- Cache policy split (`no-store` for signed-in, `no-cache` for DB-driven public pages); credentials CSV inherits `no-store`.
- `course_offerings` handlers consistently re-check department ownership server-side; marks save validates the roster and the semester's course list before filing.

## 5. What I did not verify

- Runtime behaviour against a live database (especially P1-4 — run `/admin/attendance` with data present).
- Rendered HTML of every template (source was read; Askama escapes by default).
- Migration compatibility with an existing production database (files reviewed, not applied).
- Concurrency of the import commit path (single-admin model looks safe; not load-tested).
- Whether the college actually wants staff (OfficeOrAdmin) to see `/admin` overview counts, and the promote/enrollment semantics of P1-5/P1-6 — both need a product decision.

## 6. Recommended order of work

1. P1-1 (scope collapse) — small patch in `Scope`/`may_manage_*`, plus a guard on saving HOD flags without a department.
2. P1-3 (HTMX attendance ownership) — reuse `attendance::period` + `roster` in the two handlers.
3. P1-2 (unify "today"/"now" on Asia/Kolkata everywhere, including `get_attendance_sheet`).
4. P1-4 (drop the duplicate `on_date` column) — one-line fix, verify with data.
5. Product decisions: P1-5, P1-6, P2-1.
6. P2-2 … P2-10 as a hardening pass; add the CI workflow (P2-5) first so it sticks.
7. Sweep P3 warnings and duplicates.

## 7. Build check (this pass)

- `cargo check`: OK, 16 warnings (listed in P3-1) + `sqlx-postgres v0.7.4` future-incompat notice.
- `cargo test`: 19 passed, 0 failed (CSV parsing only).
