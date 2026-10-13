# What is done, and what is left

A status map of `redesign.md` against the code as it actually is. Every "done" line below was verified
either by a test that runs or by a command whose output was inspected; the evidence is named so it can be
re-checked. Where something is partly built, both halves are stated.

Legend: **Done** · **Partial** (schema or part of the flow exists, the feature does not) · **Not started**

Test commands referenced throughout:

    docker compose up -d
    DATABASE_URL=postgres://nss:nss@localhost:5432/nss_college cargo test
    # 84 unit + 40 database + 32 end-to-end + 1 Node hook test, currently all passing
    # (the Node test shells out to `node --test tests/js/app_hooks.test.mjs`; it skips
    #  with a note if Node is not installed)

---

## 1. Scoreboard

| `redesign.md` section | Status |
|---|---|
| §3 Stack constraints (Rust/Axum/Askama/HTMX/Tailwind/SQLx/PostgreSQL) | Done — unchanged |
| §5 Super Admin (loopback-only listener, root authority) | Done |
| §5 IT Admin | Done |
| §5 Principal / Academic Admin | Done |
| §5 HOD, one per department, effective dates | Done |
| §5 Acting HOD | Done |
| §5 Delegation + non-carryover handover | Done |
| §5 Faculty / student scoping and account lifecycle | Done |
| §6 Route-by-route authorization audit | Done (every handler takes a role/scope extractor) / Partial (no per-route test matrix — see §3.B.18) |
| §6 Upstream login throttle, CSRF, session revocation, cookies | Done |
| §6 Re-authentication for critical operations | Done |
| §6 Append-only, complete audit log | Done (append-only) / Partial (no before/after or reason on every row) |
| §6 Unpublished CMS content cannot leak via direct URL | Done |
| §6 Backups protected, with retention and restore instructions | Done |
| §7 Appointment/approval lifecycle | Done |
| §7 Record states + no silent overwrite of finalized | Done |
| §7 In-app notifications and task lists | Done (feed) / Partial (the work queue is unchanged) |
| §7 Stable course identity and versions | Done |
| §7 Timetable/course relationship validation | Done (a period is taught or explicitly `teacher_tbd`, migration `0032`) / Partial (no broader validation) |
| §7 Internal vs university results kept separately | Done |
| §7 1–3 internal assessments without a schema change | Done |
| §7 CSV/Excel import with preview, validation, confirmation | Done (CSV and Excel, one row per result and a course-wise sheet) |
| §7 Correction of finalized results | Done |
| §7 Versioned migrations, history preserved | Done |
| §8 Role-specific workspaces, HTMX, responsive UI | Done / Partial (shared status badge, empty state and form error; per-row fragments cannot be partials under Askama 0.12) |
| §9 Automated test suite (unit / DB / authz / workflow / e2e / migration) | Done; the §3.B.18–19 gaps are now closed |
| §8, §9 README + architecture/permission notes | Done |
| §8, §6 Scheduled/timed CMS publishing | Done |
| §6 Reserved CMS page paths | Done |

---

## 2. What is done

### 2.1 Listener separation and the authority model (§5, P0.1–P0.3)

- Two listeners: `/` on `HOST`/`PORT`, management on `MANAGEMENT_HOST`/`MANAGEMENT_PORT`. The management
  address is validated at startup and the process **panics** on a non-loopback bind, so the boundary is
  network-level, not navigation-level. The management area (`/admin`, `/staff`) and the department
  workspace (`/hod`) are now mounted on **both** listeners for the roles that work on the public one; the
  routes kept loopback-only are root authority (`/admin/roles`, recovery, reopening a finalized record),
  refused on the public mount by `auth::management_area_on_public`.
- **The department workspace `/hod`** fixes the consequence of that split for an HOD: an HOD is a teacher,
  so they sign in on the public listener, and the public `/admin` mount refuses a `faculty` account. Every
  department link would be a 404, so `admin::manager_routes()` — every `Manager`-guarded handler, plus the
  per-user notification feed and the section index — is written with relative paths and mounted a second
  time at `/hod` (on both listeners; the same handlers, so the same guards — a teacher without an
  appointment is refused at `/hod` exactly as at `/admin`). The mount adds no authority; it makes the
  pages reachable from the listener the HOD is actually on. The IT admin, the Principal and office staff
  reach the management area on either listener; root authority is the deliberate exception.
- The workspace is a single value, not a duplicated URL table: `sections::base_for(user)` is `/hod` for a
  teacher and `/admin` for every other role, `Shell::base` carries it into every template, and
  `sections::Link::url` / `shell::NavItem::url` / `notifications::Notification::href_in` rewrite the one
  canonical `/admin…` path per link (`/teacher/reports` is left alone). Handler redirects use the same
  base, so an action taken under `/hod` returns to `/hod`.
- `role_assignments`, `delegations`, `appointment_handover_tasks` (migration `0021`) are the authority
  source. `services::appointments` resolves the effective department from an **active, in-date** head
  assignment or an **active, in-date** delegation whose granting assignment is still live.
- An expired appointment **denies**; it never falls back to the legacy boolean. Only an account with no
  appointment rows at all (predating the model) still reads `faculty.is_hod`.
- `one_active_department_head` (partial unique index) enforces one active head per department, including
  under concurrent requests; migration `0023` reconciled the legacy drift once, and
  `sync_hod_from_flags` keeps every writer of `is_hod` in step.
- Ending an appointment revokes it, clears the legacy flag, opens a handover task, and stops delegations
  derived from it. `NULL` department is a denial, never "all departments".
- Evidence: `tests/database.rs` — `a_department_can_have_only_one_active_head`,
  `an_expired_appointment_denies_instead_of_falling_back`,
  `a_delegation_follows_the_appointment_it_was_granted_from`, `an_acting_head_holds_the_same_scope_for_its_dates`,
  `ending_a_head_appointment_opens_a_handover_task`.

### 2.2 Security (P0.4–P0.7)

- Login rate limiting keyed on both account and address, checked before any account lookup, replying with
  the same generic message as a wrong password (3 unit tests).
- Session revocation by `session_version`: a deactivation, password reset or role change invalidates live
  sessions; a session predating the column fails closed.
- CSRF on every form, session-ID rotation on sign-in, hidden session table cleanup.
- Attendance IDORs and state-changing GETs removed: opening a sheet resolves the period through the same
  ownership/substitution query and is now read-only; the mutating endpoint validates the session against
  its timetable entry and the student against the offering roster.
- Uploads are no longer a raw `ServeDir` mount. `/uploads/*` authorizes the file against its owning content
  row and audience; drafts and archived files are a 404 for everyone but the owner, and served files are
  `private`, not public-cacheable.
- All responses carry `X-Content-Type-Options`, `X-Frame-Options`, `Referrer-Policy`, a `Permissions-Policy`
  and a CSP with **`script-src 'self'`**; HSTS only when `COOKIE_SECURE=true`. HTMX is self-hosted and no
  inline scripts or `on*` handlers remain.
- Input length limits on every form field (`src/validate.rs`), applied before the database.
- **Root authority** is a distinct account role (`super_admin`), not a navigation trick: it is only
  served on the loopback management listener, it is the only role that reaches `/admin/roles`, and the
  last active Super Admin can be neither demoted nor deactivated. See §3.A.2.
- `safe_next_for_role` allowlists the post-login redirect per role.
- **The IT admin is a site role with two academic tools, not an academic one.** `Manager` deliberately
  excludes `Role::Admin`, and the course structure sits behind `AcademicOnly` (`SuperAdmin`/`Principal`).
  Two tools are widened to the IT admin by their own guards instead: the **timetable** (`Scheduler` — one
  hand that reaches every department is what keeps the grid clash-free) and the **attendance report and its
  CSV** (`AcademicOrAdmin`, a read; correcting a register stays with a head of department). It keeps
  `AdminOnly` (accounts, settings, the CMS) as well, and its dashboard catalogue (`sections::areas_for`) and
  sidebar offer those two in a reduced form — no substitution, no department register — so it is offered no
  link that would answer 403. The Super Admin, as the superset, keeps everything. Evidence: e2e
  `the_it_admin_keeps_the_timetable_and_the_attendance_report`, `the_it_admin_can_schedule_a_period`,
  `the_principal_may_act_for_any_programme_but_the_it_admin_may_not`; unit
  `the_it_admin_is_offered_the_site_and_the_two_areas_it_keeps`,
  `the_it_admin_nav_is_the_site_the_schedule_and_the_attendance_report`,
  `the_it_admins_timetable_section_offers_no_substitutions`.

### 2.3 Academic integrity (P0.8–P0.12, P1.1–P1.5)

- **Non-destructive migrations.** No `DROP TABLE` on academic data; account removal is archival
  (`archive_users`), and the destructive cascade delete is gone.
- **Course identity and versions.** `course_versions` (migration `0026`) is an append-only history; each
  offering records the version it was built from (`course_offerings.course_version_id NOT NULL`), so a
  later catalogue edit cannot rewrite what an offering ran as. Catalogue editing has a UI
  (`/admin/courses/:id/edit`) with a history panel; an unchanged save is a no-op.
- **Result lifecycle.** `marks.workflow_state` (`draft → reviewed → finalized → published`, plus
  `correction_pending`/`superseded`), `mark_history`, and `result_import_batches`/`result_import_rows`
  (migration `0022`). Finalized rows are protected: `services::exams`, `services::marks` and the import
  all refuse to overwrite one, and the import preview blocks the whole commit while a finalized conflict
  exists.
- **Staged result import.** Upload (or the guided marks sheet) becomes a preview batch with row-level
  outcomes; the raw text is kept in `payload` so the commit re-validates exactly what was previewed, and
  students are never matched by name alone. Two shapes of upload are accepted from the marks sheet and
  both stage into the same review: **one row per result**, and a **course-wise sheet** — one line per
  student against a course per column, expanded by `exams::expand_course_sheet` into the same rows. Either
  may carry a `name` column beside the key: it is a label for the reviewer and is never matched against the
  database, and the commit's identity is the PRN/admission number. The review page sets **one maximum per
  course** (`result_import::courses`), which the commit applies to every row of that course, and the
  course-wise blank form itself is downloadable (`/admin/marks/template.csv` — every course of the
  semester, or one) pre-filled with the class list. Evidence: unit tests
  `a_name_column_is_read_as_a_label_and_never_as_an_identity`,
  `a_course_wise_sheet_expands_into_one_row_per_filled_cell`; e2e
  `a_course_wise_sheet_files_through_the_review_with_its_own_maximum`,
  `a_one_row_per_result_sheet_carries_its_name_and_maximum_into_the_review`.
- **Internal vs university results** are separate rows keyed by kind (`exam_kind`) and name
  (`exam_name`), with the semester stored on the row rather than read from the student's current semester,
  so advancing a student never deletes or moves past results. Several internal exams per course need no
  schema change.
- **Attendance/timetable defects**: one college-local time helper (Kolkata, not server UTC) is used for
  dates and edit windows; the department attendance route shape matches its handler; substitutions and
  teacher views include offering periods; offering clash detection was expanded.
- **Attendance correctness**: `tests/database.rs` and the unit tests cover the rules; `valid_date` is one
  shared validator (`src/dates.rs`).
- **Study materials (P1.5)** are complete: HOD/IT-admin upload and publish at `/admin/materials`, teacher
  drafts at `/teacher/materials`, student download at `/hub/materials`, with the course id re-checked
  server-side on every write.
- **Promotion** moves a student's semester in place and is audited (`student_promoted`, plus the
  cohort roll), and never touches past results. It does **not** yet create the next semester's
  enrolments — see §3.B.12. Enrolments themselves record the offering that produced them
  (`enrollments.offering_id`).
- **Timetable teacher integrity** (migration `0032`): a period is either taught by a validated teacher or
  explicitly marked `teacher_tbd`, enforced by a table CHECK (`teacher_tbd = (faculty_id IS NULL)`) rather
  than a route convention. The programme and offering forms offer the choice, both write paths go through
  one service (`academics::add_slot` / `add_offering_slot`), the grid shows a `Teacher TBD` badge, and the
  outstanding periods are a work-queue section (`academics::tbd_periods`). Evidence: `tests/database.rs`
  — `a_period_is_taught_by_a_validated_teacher_or_explicitly_tbd`,
  `an_offering_period_is_queued_under_its_offering_department`; e2e
  `a_timetable_period_can_be_left_for_a_teacher_to_be_assigned`.
- **Correlation ids** (migration `0033`, `src/correlation.rs` + middleware): every request is stamped with a
  server-generated id, and every audit row written while handling it carries it, so the rows from one
  action can be grouped. Background work (the backup scheduler) records NULL rather than a fabricated id.

### 2.4 CMS and content (§6 P1.1, P1.3)

- Content is authored in the workspace the author signs in to: notices, news and events are mounted at
  `/admin` for the IT administrator and at `/staff` for office staff, and both share the same
  `OfficeOrAdmin` handlers, so office staff may create, edit, publish, archive and delete. Root authority
  (roles, recovery, reopening a finalized record) stays loopback-only.
- Uploads are transactional: a file is written only once the metadata is known good, and a rejected write
  never leaves bytes behind. Staged import batches and their payloads are purged on a schedule.
- Audience-specific content (`public`/`students`/`faculty`) is enforced at read and at the download
  boundary.
- Page sections are constrained by both `page_id` and `section_id`.
- Course categories are data-driven from `services::courses::CATEGORIES` (one list, both forms).

### 2.5 Operations (§8 P2)

- **Backups**: `backup-<UTC stamp>/` with a custom-format dump, an uploads archive and a manifest of sizes
  and SHA-256 sums; retention keeps the newest `BACKUP_KEEP`; refuses a directory inside `static/` or the
  upload directory; `0700` run directories. Verified live — 191 690-byte dump, 56 080-byte archive,
  `pg_restore --list` read 442 TOC entries back, and the scheduler correctly skipped re-running inside the
  24-hour window. Restore commands are in the README.
- **Housekeeping** (`services::maintenance`, hourly): expired `tower_sessions` rows (they are never removed
  by the store itself), the student-import purge, result-import expiry — dropping the raw uploaded text —
  and 90-day batch cleanup.
- **CLI**: `create-user` (any role, including `super_admin` and `principal`), `set-role
  <super_admin|admin> <email>` (the break-glass path to root authority: it calls the same
  `super_admin::set_role`, so the last active Super Admin can be promoted but never demoted, and the
  change is audited), `set-password`, `seed-demo-users [--credentials-file]` (which seeds
  `owner@college.local` as the Super Admin), `backup [--list]`, `maintenance`.
- Demo passwords never reach the log: the seeder returns them and they go to a `0600` file or are
  discarded; the start-up path logs only the account count.
- `/healthz` returns a plain 200 liveness response without touching the database or exposing details;
  `robots.txt` disallows the private areas. There is no deeper readiness probe or migration report.

### 2.6 Architecture, tests, docs

- `main.rs` is only process setup and the listeners; the application is `src/lib.rs`, which is what the
  tests build against. `apply_layers`, `connect`, `migrator` and `init_session_store` are library
  functions.
- **Suite**: 84 unit, 40 database (`#[sqlx::test]`, a freshly migrated database per test), 32 end-to-end
  (`tower::oneshot` through the real layered router), and one Node hook test (`tests/js/app_hooks.test.mjs`,
  run by `tests/js_hooks.rs`). Currently all passing; the only build warning is the pre-existing
  `sqlx-postgres v0.7.4` future-incompatibility notice.
- **Shared UI**: `src/ui.rs` owns the status badge (markup and colour, rendered `|safe` via a row method);
  `templates/partials/empty_state.html` and `form_errors.html` are shared partials that read a template
  field. `Shell::pending_approvals` badges pending HOD appointments in the navigation.
- README documents the two listeners, roles, backups/restore, security headers, test commands, the layout
  and the migrations; `fixes.md` §13–§15 records each pass with its live verification.

---

## 3. What is left

This section is the brief-by-brief view. `remaining.md` Part 2 is the implementation view of the same
backlog: each item says where to change it and how to verify it, and `remaining.md` Parts 1/3/4 cover the
architecture, the recipe for adding a page, and the CLI/operations cookbook.

### A. Not started — no implementation exists

1. ~~**Principal / Academic Admin role** (brief §5).~~ **Done.** `Role::Principal` (migration `0028`
   widens the `users.role` CHECK) lands on `/admin`, gets `Scope::All` *academic* authority through
   `Manager` (cross-department, but no site/account powers), and its own sidebar. That authority is the
   Principal's alone: `Manager` no longer includes the IT admin (`AcademicOnly` and `Manager` guard the
   course structure, marks and results; the timetable and the attendance *report* are the two tools widened
   back to the IT admin under their own `Scheduler`/`AcademicOrAdmin` guards). The IT admin now **requests** an HOD
   appointment (`appointments::request_head`, status `pending`, grants nothing) and the
   Principal **approves** it (`appointments::approve_head`), with self-approval refused. Evidence:
   `tests/database.rs` — `a_head_appointment_is_pending_until_the_principal_approves`,
   `a_rejected_head_appointment_never_activates`; unit test `the_principal_is_a_distinct_role_with_an_academic_workspace`.
   The Super Admin's side of it — appointing and ending the Principal appointment — is in §3.A.2.
2. ~~**Super Admin as a distinct authority from IT Admin.**~~ **Done.** The listener boundary existed;
   the authority now does too. Migration `0031` widens the `users.role` CHECK to the `super_admin`
   account role (and adds the one-live / one-pending Principal appointment indexes). `Role::SuperAdmin`
   parses, homes to `/admin` and carries every IT-admin power (`AdminOnly`, `AdminOrPrincipal`,
   `AdminDashboard`, `OfficeOrAdmin`, `Manager` scope `All`); only `SuperAdminOnly` is exclusive to it.
   `services::super_admin` + `/admin/roles` (on the loopback listener) make the root powers real:
   * **Roles/permissions** — `set_role` gives or removes `admin`/`super_admin` only, refuses a
     Principal account (its authority is the appointment), and bumps `session_version` so a role change
     revokes live sessions. The IT admin cannot reach any of it and the people form never writes a
     `super_admin` role.
   * **Emergency recovery** — `unlock_account` clears a failed-sign-in lockout without touching the
     password; if the only root account is itself locked out or its password is lost, the CLI is the way
     back in: `set-role super_admin <email>` (new — the same rule as the screen, so it can promote the
     last root account but never demote it) and `set-password <email>` (which also clears the lockout).
   * **Emergency intervention in academic records** — `reopen_finalized` copies every locked mark into
     `mark_history` as `superseded` with a mandatory reason and only then returns the row to
     `reviewed`, from a list of the courses currently locked (`list_finalized_courses`).
   * **The last recovery account** — `set_role` and `deactivation_allowed` (called by
     `/admin/people/:id/active`) refuse to demote or deactivate the last active Super Admin.
   * **The Principal appointment** — `appoint_principal`/`revoke_principal` give the account the
     `principal` role and record an effective-dated `role_assignments` row in one transaction; the
     unique indexes allow one live appointment, a Super Admin cannot appoint itself, and ending the
     appointment returns the account to office staff. `create-user super_admin ...` and
     `seed-demo-users` (as `owner@college.local`) bootstrap the first root account.
   Evidence: `tests/database.rs` — `the_last_super_admin_cannot_be_demoted_or_deactivated`,
   `a_role_change_revokes_the_accounts_live_sessions`,
   `a_super_admin_appoints_and_revokes_the_principal`,
   `an_emergency_reopen_returns_a_finalized_course_to_review`,
   `a_locked_out_account_can_be_unlocked_without_touching_its_password`; e2e
   `root_authority_belongs_to_the_super_admin_alone` (the IT admin is 403 on the screen and on a
   self-promotion, the Super Admin's promotion applies); unit tests
   `the_super_admin_is_the_root_role_on_the_management_listener`,
   `the_super_admin_gets_the_admin_areas_plus_root_authority`.
3. ~~**Result correction workflow** (brief §7, P1.1).~~ **Done.** Migration `0027_result_corrections.sql`
   adds the proposed-value/decision columns (with a constraint that `correction_pending` is present iff a
   proposed value is). `services::marks` gains `finalize_course`, `request_correction` and
   `decide_correction`. The marks page has a **Finalize & correct** section, an **Corrections awaiting
   approval** panel, and a **Correction history** table; a finalized row offers a reason-bearing correction
   form. A correction must be approved by a *different* manager; applying it snapshots the replaced value
   into `mark_history` as `superseded` (the first writer of that state) and re-locks the row. Toggle/delete
   on a finalized or pending row is refused server-side. Evidence: `tests/database.rs` —
   `finalizing_a_course_locks_its_published_marks`, `a_correction_needs_a_second_manager_and_keeps_the_old_value`,
   `only_a_finalized_mark_can_be_corrected` (19 database tests now).
4. ~~**Notifications** (brief §7, §8).~~ **Done.** Migration `0029_notifications.sql` adds a per-user
   `notifications` table; `services::notifications` creates/lists them, counts unread, and marks them read
   (scoped to the owner). `/admin/notifications` (guarded by `Ready`, so any signed-in user reads only
   their own) is the feed, with per-item and Mark-all-read actions, linked from every workspace sidebar.
   The same feed is mounted at `/hod/notifications`, and each row's target is rewritten into the reader's
   workspace (`Notification::href_in`), so a head of department is not sent to an `/admin` URL.
   Notifications are created when an HOD appointment is requested (→ Principals and IT admins), when a
   result correction is requested (→ IT admins and the Principal). A notification is a pointer only: the
   page it opens re-checks authorization. Evidence: `tests/database.rs` —
   `notifications_are_per_user_and_marked_read_by_their_owner`, `notifying_roles_reaches_every_active_holder`.
   Remaining: the `/admin/work-queue` list itself is unchanged, and notifications are not yet raised for
   deadlines or failed imports.
5. ~~**Scheduled CMS publishing** (brief §7, §8).~~ **Done.** Migration `0030` adds `unpublish_at` to
   `notices`, `news`, `events`, `documents` and `pages` (plus the `published_at` column `pages` never had)
   and widens each status check to allow `scheduled`. `content::visible(alias)` is now the one visibility
   predicate — published, or scheduled with a time that has arrived, and not past `unpublish_at` — and every
   read path goes through it (`content.rs`, `hub.rs`, and the upload authorization in `public.rs` replaced
   its own hand-written copies). `content_admin::{publish_new_sql, publish_set_sql}` decide the stored
   publish time on write, so editing a published row keeps its original time and an explicit time wins; the
   status and `datetime-local` fields were added to the notice, news, event, document and page forms, and
   the page list's quick Publish stamps the time so it is live at once. Note: `dates::valid_date` bounds a
   publish time to 2000–2100, the same limit the other date fields already have. Evidence: `tests/database.rs`
   — `a_scheduled_notice_stays_hidden_until_its_publish_time`, `an_expired_notice_drops_out_of_the_public_list`,
   `a_scheduled_page_is_not_served_until_its_time`, `a_published_update_keeps_its_stored_publish_time`; e2e
   `the_notice_form_schedules_a_publish_time` (renders the field, posts a scheduled notice, asserts the row
   is hidden from the public list).
6. ~~**Excel result import.**~~ **Done.** `import_students::to_csv_text` flattens an uploaded
   `.xlsx`/`.xlsm`/`.xls`/`.ods` workbook to CSV text with the same `calamine` path the student import uses;
   the guided marks import, the staged result import and the paste/CSV push all go through it, so they
   accept the same file types. A plain CSV/TSV file is still decoded as before. Evidence: unit test
   `a_csv_upload_converts_to_csv_text_unchanged` (the workbook branch is the existing, already-shipped
   `spreadsheet` reader; no `.xlsx` fixture is checked in).
7. ~~**Re-authentication for critical operations** (brief §6).~~ **Done.** `/account/reauth` asks for the
   password again and records a timestamp in the session (`auth::SESSION_REAUTH`); a confirmation counts for
   15 minutes (`auth::reauth_valid`, unit-tested). `auth::require_reauth` gates the critical handlers:
   appointment request/approve/reject/revoke and delegations, marks finalize/request-correction/approve/
   reject, and the result-import commit — each redirects to the re-auth page (then back) when the check has
   lapsed. Backup/restore are CLI-only. Evidence: unit test `a_reentry_only_counts_within_the_window`,
   e2e `re_authentication_requires_a_signed_in_session`.
8. ~~**Reserved route paths for CMS pages.**~~ **Done.** `site_admin::reserved_path` rejects a page whose
   path is `/login`, `/logout`, `/admin`, `/teacher`, `/hub`, `/account`, `/static`, `/uploads`, `/healthz`
   or `/robots.txt` (whole-segment match, trailing slash ignored), enforced in `read_form` for a friendly
   message and again in `create_page`/`update_page` as defence in depth. Evidence: unit test
   `reserved_paths_cannot_be_shadowed_by_a_cms_page`.

### B. Partial — the data or half the flow exists, the feature does not9. ~~**Single applicability rule for exams/timetable** (P0.12).~~ **Done.** `academics::applicable_course`
    is the one rule (programme course, or a catalogue course carried by an offering that targets the
    programme); `course_in_class` now delegates to it, so the timetable validator and the exam/marks
    pickers (`exams::course_options_for_semester`, documented as the same rule) cannot drift. Exam and marks
    writes already rejected a course outside the picker. Evidence: `tests/database.rs` —
    `a_catalogue_course_is_applicable_through_its_offering`.
10. ~~**Timetable/course relationship enforcement** (P0.10).~~ **Done.** A period is taught by a validated
    teacher or explicitly `teacher_tbd` (migration `0032`, table CHECK), the forms offer the choice, the
    grid shows a `Teacher TBD` badge, and the outstanding periods are a work-queue section. Evidence:
    `a_period_is_taught_by_a_validated_teacher_or_explicitly_tbd`,
    `an_offering_period_is_queued_under_its_offering_department` (database),
    `a_timetable_period_can_be_left_for_a_teacher_to_be_assigned` (e2e), `a_checkbox_understands_every_spelling_a_browser_sends` (unit).
11. ~~**Capacity policy** (P0.11).~~ **Done.** Student confirmation is race-safe (`SELECT … FOR UPDATE`
    with a fresh enrolment count); HOD assignment enforces it through `offering_for_confirm`; and cohort
    finalization now locks the offering and refuses when the cohort does not fit
    (`taken + needing > capacity`), before any enrolment is written. Evidence: `tests/database.rs` —
    `cohort_finalization_refuses_when_the_offering_cannot_hold_the_cohort` (covers both the refusal and the
    fits-and-enrols path).
12. ~~**Offerings in the student results view** (P1.3).~~ **Fixed.** `exams::results_for` and
    `exams::result_semesters` read the semester through `COALESCE(e.semester, c.semester)`, so a catalogue
    course (`courses.semester IS NULL`) carried by an offering enrollment appears correctly. The read path
    was already correct on re-inspection; a regression test now pins it. Evidence: `tests/database.rs` —
    `a_catalogue_course_offering_shows_in_the_student_results`.
13. ~~**Offerings in substitution candidates** (P1.4).~~ **Already addressed by inspection.**
    `attendance::teacher_day` (which the substitutions page uses for candidates) already resolves an
    offering period's programme from `course_offering_targets` via `COALESCE(p.name, subquery, 'Offered
    course')`. No regression test covers it yet.
14. ~~**Clash detection between two offerings** (P1.4).~~ **Already addressed by inspection.**
    `academics::offering_clashes` compares two offerings that both target the same programme in the same
    semester (`EXISTS` over paired `course_offering_targets`), so they clash. No regression test yet.
15. ~~**Attendance report filters** (P1.4).~~ **Fixed.** `by_session`, `totals` and `by_student` all scope
    by the *student's* programme (`st.programme_id = $3`), so they agree. The remaining defect — the course
    dropdown hiding catalogue courses under a programme filter — is fixed in `reports::course_options`:
    a course is listed when it belongs to the programme *or* any active student of the programme is
    enrolled in it (catalogue courses have `programme_id IS NULL`). Evidence: `tests/database.rs` —
    `the_attendance_course_filter_includes_catalogue_courses_taught_to_the_programme`.
16. **Audit detail** (P0.7) — **improved, partial.** Rows carry actor/action/entity/entity_id and the log is
    append-only by trigger. `users::audit_detail` writes the structured `details` JSONB, used for result
    corrections, HOD appointments, role changes, account activation/deactivation, the bulk archive,
    appointment revoke, delegation grant/revoke, CMS publish/archive, settings save and backups. Every row
    now also carries a per-request `correlation_id` (migration `0033`), read from `src/correlation.rs` and set
    by middleware — background work records NULL. Remaining: other audit calls still pass no detail, and some
    still discard their error with `let _ =`. Evidence: `tests/database.rs` —
    `an_audit_detail_row_keeps_its_structured_payload`, `audit_rows_carry_the_correlation_id_of_their_request`;
    e2e `every_request_gets_its_own_audit_correlation_id`; unit `ids_do_not_repeat`,
    `the_id_is_visible_only_inside_its_scope`.
17. **Reusable UI components** (brief §8) — **started.** `src/ui.rs` is the one definition of the status
    badge (markup and colour), rendered `|safe` through a row method; the five content lists use it.
    `templates/partials/empty_state.html` and `form_errors.html` are shared partials reading a template
    field, used by those lists and forms. `Shell::pending_approvals` adds the pending-appointment badge to
    the navigation for the roles that decide appointments. Limitation: Askama 0.12 has no macros and its
    `{% include %}` cannot take parameters, so a per-row fragment cannot be a partial — tables stay
    consistent by convention. Evidence: unit `a_status_badge_is_consistent_and_escaped`,
    `every_known_status_has_its_own_colour`; e2e
    `the_notice_list_uses_the_shared_empty_state_and_status_badge`,
    `the_navigation_badges_appointments_waiting_for_a_decision`.
18. **Authentication test depth** (brief §9) — **extended.** `tests/app.rs` has a real sign-in helper (login
    through the router, session cookie reused) and now eight through-the-router authorization tests:
    `an_hod_cannot_touch_another_departments_programme`, `a_teacher_with_no_appointment_cannot_reach_the_academic_tools`,
    `the_it_admin_may_act_for_any_programme`, `root_authority_belongs_to_the_super_admin_alone`,
    `one_student_cannot_withdraw_another_students_selection`,
    `a_student_reaches_their_own_hub_but_not_the_staff_tools`,
    `a_teacher_cannot_mark_attendance_for_another_teachers_class` and
    `only_the_it_admin_publishes_or_deletes_cms_content`. The student-vs-student case proves the route scopes
    the write to the session student by asserting the other student's row survives.
19. ~~**JS behaviour coverage.**~~ **Done.** `tests/js/app_hooks.test.mjs` loads the real
    `static/js/app.js` into a fake DOM and fires each `data-*` hook (mark-all radios, select-all and its
    toggle, autosubmit, confirm, print), asserting its effect. `tests/js_hooks.rs` runs it under
    `node --test` so `cargo test` covers it, skipping with a note when Node is absent. There is still no
    real-browser test of layout or `hx-*` behaviour.

### C. Known stale or thin spots in the docs

- README items #8 and #9 (catalogue edit UI, student-action auditing) are now marked fixed. Items #1–#7
  remain accurate and map to §3.B above.
- `fixes.md` §1–§12 describe the *original* audit and are history, not a current status list; §13–§15 are
  the passes that were actually implemented.

---

## 4. Suggested order for what is left

Dependency-aware, highest value first:

1. ~~**Result correction workflow**~~ **Done** (migration `0027`, `services::marks`, marks page).
2. ~~**Offerings in the student results view** (§3.B.12)~~ **Done** — verified fixed with a regression test.
3. ~~**Principal role and the HOD appointment approval**~~ **Done** — pending request by the IT admin,
   Principal approval with self-approval refused, and the Super Admin appointing the Principal itself
   (§3.A.2).
4. ~~**Scheduled CMS publishing and reserved-path validation**~~ **Done** — reserved paths (§3.A.8,
   `site_admin::reserved_path`) and scheduled/time-limited publishing (migration `0030` plus the shared
   `content::visible` predicate).
5. ~~**Notifications/task list**~~ **Done** — notifications feed added (`/admin/notifications`); the
   work-queue list is unchanged.
6. ~~**Exam/timetable applicability helper** (§3.B.9) and **capacity everywhere** (§3.B.11).~~ **Done** —
   one `applicable_course` rule; cohort finalization enforces capacity.
7. ~~**Remaining P1.4 defects** (substitutions, offering-vs-offering clashes, report filters).~~ **Addressed**
   — substitutions (§3.B.13) and offering-vs-offering clashes (§3.B.14) were already handled in code;
   the attendance course filter (§3.B.15) is fixed with a regression test.
8. ~~**Super Admin distinct powers.**~~ **Done** — the `super_admin` account role, `/admin/roles`,
   the last-recovery-account guard, the Principal appointment and the audited reopening of a finalized
   course (§3.A.2). The audit-detail (§3.B.16), timetable-teacher (§3.B.10), UI-component (§3.B.17),
   authorization-matrix (§3.B.18) and JS-coverage (§3.B.19) items from the same pass are also now done or
   bounded, leaving only the items this document records as still partial. The one structural limit left in
   §3.B.17 is Askama's: per-row fragments cannot be partials without a template-macro upgrade.

---

## 5. This pass: website content and staff profiles

- **Photo placement per section** — migration `0036` adds `photo_position` (`above` / `below` / `left` /
  `right` / `inline`) to `page_sections`, `home_sections` and `section_display_defaults`. `layout::DisplayOptions`
  carries it, the editor offers it, and `partials/page_section.html` honours every value — including the
  `[photo]` line an author writes to place the picture inside the text. Verified by a unit test for the
  cleaning and round trip, and an e2e test that a new section stores the placement it was given.
- **Pages screen** — `/about/staff` and `/academics/rank-holders` are dropped from the admin Pages list
  (`site_admin::MANAGED_ELSEWHERE`), so the faculty and rank-holder screens stay the single place to manage
  them. Verified by an e2e test.
- **The website on the loopback port** — `management_router()` now merges `site_routes()` as well as the
  management area, so the whole public site is browsable on `:3001`. The loopback bind is unchanged and
  management stays reachable on both listeners (root authority excepted). Verified by an e2e test that
  `/`, `/academics`, `/departments`, `/notices` and `/about/staff` answer through the management router.
- **Staff profiles** — migration `0037` adds `staff_profiles`. At `/profile`, a teacher edits their own
  `faculty` row (name, designation, qualification, contact, biography, optional photo) while the Principal
  and office staff keep theirs in `staff_profiles`, keyed by account. It appears on `/about/staff` under a
  **Principal** or **Office staff** heading, and the photo is served through the authorization-aware
  `/uploads` route. Verified by e2e tests for the Principal and office-staff path, the teacher path, the
  group headings, and the `403` for a role with no profile.
