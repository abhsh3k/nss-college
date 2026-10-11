# What is done, and what is left

A status map of `redesign.md` against the code as it actually is. Every "done" line below was verified
either by a test that runs or by a command whose output was inspected; the evidence is named so it can be
re-checked. Where something is partly built, both halves are stated.

Legend: **Done** · **Partial** (schema or part of the flow exists, the feature does not) · **Not started**

Test commands referenced throughout:

    docker compose up -d
    DATABASE_URL=postgres://nss:nss@localhost:5432/nss_college cargo test
    # 65 unit + 37 database + 14 end-to-end, currently all passing

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
| §7 Timetable/course relationship validation | Partial |
| §7 Internal vs university results kept separately | Done |
| §7 1–3 internal assessments without a schema change | Done |
| §7 CSV/Excel import with preview, validation, confirmation | Done (CSV and Excel) |
| §7 Correction of finalized results | Done |
| §7 Versioned migrations, history preserved | Done |
| §8 Role-specific workspaces, HTMX, responsive UI | Done / Partial (reusable components incomplete) |
| §9 Automated test suite (unit / DB / authz / workflow / e2e / migration) | Done for what exists; gaps listed in §3.B.18–19 |
| §8, §9 README + architecture/permission notes | Done |
| §8, §6 Scheduled/timed CMS publishing | Done |
| §6 Reserved CMS page paths | Done |

---

## 2. What is done

### 2.1 Listener separation and the authority model (§5, P0.1–P0.3)

- Two listeners: `/` on `HOST`/`PORT`, management on `MANAGEMENT_HOST`/`MANAGEMENT_PORT`. The management
  address is validated at startup and the process **panics** on a non-loopback bind, so the boundary is
  network-level, not navigation-level. `public_router()` and `management_router()` register disjoint route
  sets.
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
  students are never matched by name alone.
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

### 2.4 CMS and content (§6 P1.1, P1.3)

- Publishing authority is with the IT admin: `set_status` (publish/unpublish/archive) and `destroy` are
  `AdminOnly`; office staff can author and edit but not publish or delete.
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
- **Suite**: 65 unit, 37 database (`#[sqlx::test]`, a freshly migrated database per test), 14 end-to-end
  (`tower::oneshot` through the real layered router). Currently all passing; the only build warning is the
  pre-existing `sqlx-postgres v0.7.4` future-incompatibility notice.
- README documents the two listeners, roles, backups/restore, security headers, test commands, the layout
  and the migrations; `fixes.md` §13–§15 records each pass with its live verification.

---

## 3. What is left

### A. Not started — no implementation exists

1. ~~**Principal / Academic Admin role** (brief §5).~~ **Done.** `Role::Principal` (migration `0028`
   widens the `users.role` CHECK) lands on `/admin`, gets `Scope::All` *academic* authority through
   `Manager` (cross-department, but no technical powers), and its own sidebar. The IT admin now
   **requests** an HOD appointment (`appointments::request_head`, status `pending`, grants nothing) and the
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
10. **Timetable/course relationship enforcement** (P0.10). Forms no longer treat a bad id as "unassigned",
    but a published period still does not require a validated teacher, and there is no explicit
    `teacher_tbd` state with a work-queue item.
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
    append-only by trigger. `users::audit_detail` now writes the structured `details` JSONB, used for result
    corrections (reason + proposed value, and the decision) and HOD appointments (department, acting,
    reason, appointed user). Remaining: other audit calls still pass no detail, and some still discard their
    error with `let _ =`; there is no correlation id. Evidence: `tests/database.rs` —
    `an_audit_detail_row_keeps_its_structured_payload`.
17. **Reusable UI components** (brief §8). Tables, empty states and form errors are consistent by
    convention, not by shared components; there is no pending-approval badge in the navigation.
18. **Authentication test depth** (brief §9) — **started, partial.** `tests/app.rs` now has a real sign-in
    helper (login through the router, session cookie reused) and four through-the-router authorization
    tests: `an_hod_cannot_touch_another_departments_programme` (cross-department mutation is 403),
    `a_teacher_with_no_appointment_cannot_reach_the_academic_tools` (403),
    `the_it_admin_may_act_for_any_programme` (positive control) and
    `root_authority_belongs_to_the_super_admin_alone` (403 for the IT admin, and the Super Admin's
    re-authenticated role change takes effect). Remaining: the same matrix for the other roles and
    resource kinds (student-vs-student, teacher-vs-assigned-course, CMS ownership).
19. **JS behaviour coverage.** The `data-*` hooks in `app.js` are not covered by automated tests (no
    browser in this environment); they were verified by inspecting rendered markup and reading the script.

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
   course (§3.A.2). With audit detail (§3.B.16), re-authentication (§3.A.7) and the through-the-router
   authorization tests (§3.B.18) done, the remaining work is the UI-consistency items (§3.B.17, §3.B.19)
   and the rest of the authorization matrix, all of which are recorded above rather than hidden.
