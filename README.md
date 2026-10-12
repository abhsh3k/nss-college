# NSS College Rajakumari: website, Student Hub and admin

Rust + Axum + Askama + HTMX (+ PostgreSQL/SQLx from Layer 3). Styling: Tailwind CSS.

## Run

1. Start PostgreSQL (Docker is the easiest way):

       docker compose up -d

   No Docker? Install PostgreSQL 16 yourself, then create the database and user:

       CREATE USER nss WITH PASSWORD 'nss';
       CREATE DATABASE nss_college OWNER nss;

2. Configure and run:

       cp .env.example .env
       cargo run
        # public site: http://127.0.0.1:3000
        # management interface (loopback only): http://127.0.0.1:3001/login

On start-up the app connects using `DATABASE_URL`, applies every file in `migrations/`, and the site renders from the database. The first run loads starter content copied from the live site (programmes, departments, news, rank holders, pages, contact details).

Styling is compiled Tailwind CSS. Build it once before the first run (see the next section).

## Admin tools (Phase 4b-1)

Sign in as the IT admin, then use the sidebar:

- **People**: add students, teachers and office staff; edit them; issue a new temporary password; deactivate an account.
  "Import students" accepts a pasted class list (comma- or tab-separated, so Excel rows work).
  Temporary passwords are shown once on a printable page.
- **Courses and programmes**: add the courses of each semester and assign a teacher. New students are enrolled in their
  semester's courses automatically; use "Enroll students" after adding courses later.
- **Timetable**: choose programme and semester, then add periods per day. Clashes (same teacher, room or class at overlapping
  times) are refused with an explanation. Everything updates without reloading the page.
  A period must name a validated teacher or be explicitly marked **teacher to be assigned**: the missing teacher
  is then a recorded gap, shown in the work queue, never a silently incomplete row.
  A published course offering is scheduled from the same page (the offering picker): the panel lists that
  offering's periods and adds or removes them in place, with the same clash rules — the offering page's
  "add period" form stays as the secondary path.
- **Work queue** (`/admin/work-queue` in the management workspace, `/hod/work-queue` for a head of
  department — see "Three workspaces" below): the real work in one list —
  pending external offering approvals, unanswered change requests, cohort decisions that are not finalised,
  published offerings with no timetable periods, timetable periods with no teacher, and offerings with no
  seats left. Every row links to the catalogue, offering-detail, timetable or selections page that handles
  it; those pages are unchanged.
- **Notifications** (`/admin/notifications`): a per-user, actionable feed. An HOD appointment request
  notifies the Principal and IT admins; a result correction request notifies the IT admins and the
  Principal. Each item is a pointer to the page that owns the decision, and that page re-checks access,
  so a notification never grants anything. Items can be marked read individually or all at once.

After pulling this update, rebuild the CSS (new classes were added) and restart:

    powershell -ExecutionPolicy Bypass -File .\build-css.ps1
    cargo run

## Teacher tools and attendance (Phase 4c)

- **Today** and **Mark attendance**: a teacher sees their periods (and any they are covering), taps one, and marks each
  student present, absent or on leave. Saving happens in place, without reloading. Periods can be taken or corrected up to
  5 days after the class date (setting `attendance_edit_window_days`).
- **My timetable** and **Attendance reports**: weekly timetable, classes being covered, and a per-course table with each
  student's percentage. Below 75% is flagged; e-grants students are also checked for the current month.
- **Substitutions** (IT admin): choose a date and an absent teacher, assign a substitute for each period. Only the substitute
  can then take that period's attendance. Overlaps with the substitute's own classes are refused.
- A teacher account needs a teacher profile. Create teachers under People (not with the command line) so the profile exists.
- "Today" always means the college's local date (India time), not the server's UTC date.
- Whether **leave** counts as present is the setting `attendance_leave_counts_as_present` (default false). Change it in the
  `site_settings` table until the settings screen exists.

## Sign in and accounts (Phase 4a)

Create the first Super Admin and the first IT administrator (you will be asked for a password at a hidden prompt):

    cargo run -- create-user super_admin owner@example.com "Site Owner"
    cargo run -- create-user admin you@example.com "Your Name"

Create test accounts for the other roles. These must choose a new password at first sign-in:

    cargo run -- create-user staff office@example.com "Office Staff"
    cargo run -- create-user faculty teacher@example.com "Test Teacher"
    cargo run -- create-user student 1001@college.local "Test Student"

Then run `cargo run` and open http://127.0.0.1:3001/login for staff/admin management accounts. Students and teachers can sign in through the public listener at http://127.0.0.1:3000/login. Each role lands on its own dashboard:
IT admin and the Principal on `/admin`, office staff on `/staff`, teachers on `/teacher`, students on `/hub`.

### Three workspaces: `/admin`, `/staff` and `/hod`

The management area is served on both listeners, so the roles that sign in on the public listener also
reach it. **Root authority is the exception**: a Super Admin session is refused on the public listener
and works only on the loopback management listener. The workspaces are built from the same handlers, with
identical per-request guards:

| workspace | who | listener |
| --- | --- | --- |
| `/admin` | IT admin, Principal (and Super Admin) | both; **root-only pages loopback `:3001` only** |
| `/staff` | office staff | both (`:3000` and `:3001`) |
| `/hod` | head of department, approved delegate | both (`:3000` and `:3001`) |

Every page, redirect and sidebar link is built from the caller's workspace, so an HOD or an office-staff
member never sees an `/admin` URL, and the IT admin never sees an `/hod` or `/staff` one. The public
`/admin` and `/staff` mounts are wrapped by `auth::management_area_on_public`, which refuses a `faculty`
account (its workspace is `/hod`) and, above all, **root authority** — so `/admin/roles`, recovery and the
emergency reopening of a finalized result are reachable only on the loopback listener. Mounting a handler
set a second time grants nothing: the guards run per request, so a caller who is not allowed is refused
wherever they try.

The **Principal** (`create-user principal ...`) holds institution-wide *academic* authority — the same
cross-department reach as the IT admin on the academic tools, but no technical powers. The IT admin
creates each account and **requests** an HOD appointment at `/admin/appointments`; the Principal
**approves** it before it takes effect, and a person cannot approve their own appointment.

The **Super Admin** is the root account role and is only reachable on the loopback management listener.
It holds every IT-admin power plus `/admin/roles`, where it gives or removes the `admin` and
`super_admin` roles, clears a locked-out account, appoints and ends the **Principal** appointment
(with effective dates), and can reopen an already-finalized course as an audited emergency
intervention. An IT admin cannot reach that screen, cannot promote itself, and cannot demote or
deactivate the last active Super Admin — so the college cannot be locked out of role management.
Bootstrap the first root account with `create-user super_admin ...` (or `seed-demo-users`, which adds
`owner@college.local`).

If no account can reach `/admin/roles` — the only root account was locked out, or its password is lost —
the CLI is the way back in:

    cargo run -- set-role super_admin someone@example.com   # give an existing account root authority
    cargo run -- set-password someone@example.com           # replace the password and clear any lockout

`set-role` accepts only `super_admin` and `admin` and runs the same rule as the screen, so it can promote
the last root account but never demote it, and it records the change in the audit log.

Rules built in: five wrong passwords lock an account for 15 minutes; sessions last 8 hours of inactivity;
every form carries a CSRF token; passwords are stored with Argon2.
The management listener is hard-bound to loopback by the application and must not be exposed by a reverse proxy. The management area is now also served on the public listener for the IT admin, the Principal and office staff — never for root authority — so the loopback listener is the boundary that keeps `/admin/roles` off the network. In production set `COOKIE_SECURE=true`, serve the public site over HTTPS, and proxy management access only through a controlled local administration path.

## Tests

    docker compose up -d          # the tests need PostgreSQL
    DATABASE_URL=postgres://nss:nss@localhost:5432/nss_college cargo test

Three layers run:

- **Unit tests** (`cargo test --lib`): pure rules — appointment expiry, date validation, input limits,
  the login limiter, CSV parsing, the navigation model. No database needed.
- **Database integration tests** (`tests/database.rs`): `#[sqlx::test]` creates a fresh database per test
  and applies every migration, so they check the constraints themselves — one active head per department,
  the append-only audit log, course versions and offering pinning, the marks/result-import states, and the
  appointment/delegation services. The role needs `CREATEDB` (the `nss` role has it).
- **End-to-end tests** (`tests/app.rs`): requests are sent through the real layered router with `tower::oneshot`, so
  routing, sessions, CSRF, templates and the response headers are all exercised without a socket.

The database tests are the slow ones (each test creates and migrates a database); `cargo test --lib` is instant.

## Backups and housekeeping

The server runs an hourly housekeeping task (`services::maintenance`) that

- deletes expired `tower_sessions` rows (the store never removes them itself),
- runs the existing student-import purge, and
- expires staged result imports past their window — dropping the raw uploaded text they hold — then
  deletes batches older than 90 days.

It also takes a backup when the last one is older than `BACKUP_INTERVAL_HOURS`. Each run writes
`BACKUP_DIR/backup-<UTC stamp>/` containing `database.dump` (custom format), `uploads.tar.gz` and a
`manifest.json` with sizes and SHA-256 sums, then keeps the newest `BACKUP_KEEP` sets. A failed dump is
reported and never leaves a half-set behind; the uploads archive is best-effort and the manifest records
whether it was produced. Backups may not be written inside `static/` or the upload directory, and the run
directory is created `0700`.

Manual runs:

    cargo run -- backup            # take one now
    cargo run -- backup --list     # what a restore could use
    cargo run -- maintenance       # run the purge once

`pg_dump` on `PATH` is used when present; otherwise set `BACKUP_DOCKER_SERVICE=db` and the dump is taken
with `docker compose exec`.

Restore (deliberately manual — it overwrites live data):

    # database (custom format; --clean drops and recreates the objects)
    docker compose exec -T db pg_restore --clean --if-exists --no-owner \
        -U nss -d nss_college < backups/backup-<stamp>/database.dump
    # uploads
    tar -xzf backups/backup-<stamp>/uploads.tar.gz -C .

Verify a dump before relying on it: `docker compose exec -T db pg_restore --list < .../database.dump`
lists its table of contents.

## Security headers and the Content-Security-Policy

Both listeners send `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`,
`Referrer-Policy: strict-origin-when-cross-origin`, a `Permissions-Policy` and a CSP from
`apply_layers` (`src/lib.rs`). HSTS is added only when `COOKIE_SECURE=true`, since pinning a browser to
https:// on a plain-HTTP development server would break it.

The policy is strict for scripts: `script-src 'self'`. HTMX is therefore served from
`static/js/htmx.min.js` rather than a CDN, and every inline `on*` handler was replaced by a `data-*` hook
handled in `static/js/app.js` (`data-confirm`, `data-autosubmit`, `data-select-all`,
`data-select-all-toggle`, `data-print`, `data-mark-all`). Keep it that way: adding an `onclick=` or an
inline `<script>` will silently stop working in the browser. `style-src` keeps `'unsafe-inline'` because a
few templates set a per-row CSS value (the homepage carousel and the attendance bars) that cannot move
into a class.

## Building the CSS

Windows (PowerShell, from the project folder):

    powershell -ExecutionPolicy Bypass -File .\build-css.ps1          # one-off build
    powershell -ExecutionPolicy Bypass -File .\build-css.ps1 -Watch   # rebuild while you edit templates

The script downloads the standalone Tailwind CLI (v3.4.17, no Node needed) on first use and writes `static/css/tailwind.css`.
Rebuild whenever you add new Tailwind classes to a template. Colours and fonts live in `tailwind.config.js`.

On Linux or macOS, download the matching `tailwindcss` v3.4.17 binary from the Tailwind releases page and run:

    ./tailwindcss -c tailwind.config.js -i assets/input.css -o static/css/tailwind.css --minify

## Production CSS

    # standalone CLI: https://github.com/tailwindlabs/tailwindcss/releases
    tailwindcss -i assets/input.css -o static/css/tailwind.css --minify

Then replace the contents of `templates/partials/styles.html` with:

    <link rel="stylesheet" href="/static/css/tailwind.css">
    <link rel="stylesheet" href="/static/css/app.css">

Keep the colours and fonts in `tailwind.config.js` and `styles.html` identical.

## Layout

    src/routes/     public pages and HTMX fragment endpoints
    src/models.rs   read models (sqlx::FromRow)
    src/services/   database queries, one module per area
    src/error.rs    AppError and the 404 / 500 pages
    templates/      layouts/ partials/ public/  (hub/ and admin/ come later)
    static/         app assets: css, js, img
    uploads/        user content: notices, events, faculty, documents, news
    src/lib.rs      the application as a library (main.rs is only process setup and the listeners),
                    including the shared middleware and the security headers
    src/dates.rs    the one calendar-date validator every form uses
    src/validate.rs the shared input-length limits
    tests/          database integration tests and end-to-end router tests
    migrations/     0001 core, 0002 academics, 0003 content, 0004 triggers, 0005 starter content,
                    … 0013 course offerings (courses, offerings, targets, approvals, selections,
                    change requests), 0014 updated_at columns, 0015 studash, 0016 page menu,
                    0017 enrollments.offering_id, 0018 import PRN, 0019 exam result kinds,
                    0020 teaching departments, 0021 authorization lifecycle (role_assignments,
                    delegations, handover tasks, append-only audit), 0022 result workflow (marks
                    states and mark_history, staged result imports), 0023 appointment reconciliation,
                    0024 result-import payload, 0025 guided result import, 0026 course versions
                    (course_versions, offerings pinned to the version they were built from),
                    0027 result corrections (reasoned, separately-approved correction of finalized
                    marks), 0028 principal role (a distinct 'principal' account role),
                    0029 notifications (a per-user actionable feed), 0030 content scheduling
                    (`unpublish_at` on notices/news/events/documents/pages, `pages.published_at`),
                    0031 super admin (the `super_admin` account role, one live and one pending
                    Principal appointment), 0032 timetable teacher TBD, 0033 audit correlation
                    ids, 0034 `events.published_at`, 0035 result-import names (the student-name
                    label a file may carry) and the course-wise import

## Layers

1. Design system and templates (done)
2. Routing structure for all public pages (done: pages registry, programme/department/news/notice routes, sitemap.xml, robots.txt)
3. PostgreSQL schema, migrations, SQLx services (done)
4. Authentication, Student Hub, teacher tools, admin
   - 4a (done): sign-in, sessions, CSRF, roles, dashboard shells, `create-user` command
   - 4b-1 (done): admin tools for people, programmes/courses, enrollment and the timetable builder
   - 4b-2 (done): publishing tools (notices, news, events, documents, pages, settings) for IT admin and office staff
   - 4c (done): teacher dashboard, per-period attendance, substitutions, attendance reports
   - 4d (done): student views — the Student Hub at `/hub` shows the announcements feed, today's
     timeline, weekly timetable matrix, enrolled courses with live attendance percentages, and the
     e-grants warning card when the monthly attendance drops below the configured threshold
5. HTMX interactions and polling, ETag caching
6. Security, performance, tests, deployment

## Notes

- An enrollment records which offering produced it (`enrollments.offering_id`, migration 0017, back-filled
  from confirmed selections). Seat counts, the capacity check at confirm/assign, and an offering period's
  attendance roster all read that column: the roster for an offering period lists only the students holding
  a confirmed or locked selection for that offering, never every student ever enrolled in the course.
- Informational pages (about, IQAC, fees, ...) live in the `pages` and `page_sections` tables and are served by path, so a page added to the database works without a restart. Pages with no sections show a short "still preparing" message.
- Queries use `sqlx::query_as` at runtime rather than the compile-time macros, so the project builds without a database.
- Students, the timetable and attendance have models and queries in `services/hub.rs` (Student Hub); the
  marks, exam and result-import tables have their own services (`services/marks.rs`, `services/exams.rs`,
  `services/result_import.rs`).
- The header and footer still carry the college phone number as fixed text. Contact details on the home and contact pages come from `site_settings`.
- Set `SITE_URL` in `.env` so `sitemap.xml` and `robots.txt` use your real domain.
- Images on the home page currently hotlink the live site and move to `uploads/` when the content is migrated.

## Known limitations (to do later)

`remaining.md` is the handover file: it explains the architecture and structure, lists everything still open with
where to change it and how to verify it, and is the CLI/operations cookbook (commands, environment,
troubleshooting). This section is the behaviour-level list; `remaining.md` Part 2 is the implementation view.

Review of the course management & selection feature (branch `feature/curriculamv2`, first verified
6 Oct 2026 and re-checked against the current code). The feature works end to end — catalogue → offering →
publish → cross-department approval → selection (FIXED / HOD-assigned / individual choice / cohort
choice) → enrollment → timetable and attendance. The items struck through below have since been fixed;
the evidence (a test name, or "by inspection" where the code already handled it) is named with each one.

1. ~~**Results section never shows offering courses.**~~ **Fixed**: `exams::results_for` and
   `exams::result_semesters` read the semester through `COALESCE(e.semester, c.semester)`, so a catalogue
   course carried by an offering appears in `/hub` Results. Regression test:
   `a_catalogue_course_offering_shows_in_the_student_results`.
2. **Marks and exams** — **the "no UI" half is fixed**: `/admin/marks` is the entry grid, `/admin/exams`
   the timetable, and results arrive through the guided sheet, the staged import
   (`/admin/exams/results/*`) or the CSV/Excel push, with the pickers resolving an offering course through
   `academics::applicable_course`. A bulk upload may be written either as one row per result or as a
   course-wise sheet (one line per student, one column per course); both stage into a review where each
   course's maximum marks is set, and the course-wise blank form is downloadable from `/admin/marks` for
   the whole semester or one course. What remains is structural: `exams.programme_id` is still `NOT NULL`
   (migration 0002), so an exam row belongs to a programme and semester rather than to the course version
   an offering ran as.
3. ~~**Substitutions skip offering periods**~~ **Fixed** (by inspection): `attendance::teacher_day`, which
   the substitutions page uses for candidates, resolves an offering period's programme from
   `course_offering_targets`.
4. ~~**Clash detection misses offering-vs-offering overlap**~~ **Fixed** (by inspection):
   `academics::offering_clashes` compares two offerings that target the same programme in the same
   semester, so they clash.
5. ~~**Programme timetable grid missing offering periods**~~ **Fixed**: offering periods that target the
   programme appear in the grid rows, marked read-only (added and removed from the offering panel on the
   same `/admin/timetable` page, offering picker), and are also visible on the offering page, `/hub` and
   the teacher pages.
6. ~~**Attendance reports disagree under a programme filter**~~ **Fixed**: `by_session`, `totals` and
   `by_student` all scope by the *student's* programme, and `reports::course_options` lists a course when
   it belongs to the programme *or* an active student of the programme is enrolled in it (catalogue
   courses have `programme_id IS NULL`). Regression test:
   `the_attendance_course_filter_includes_catalogue_courses_taught_to_the_programme`.
7. ~~**Capacity is enforced only at student confirm**~~ **Fixed**: confirmation is race-safe
   (`SELECT … FOR UPDATE` plus a fresh enrolment count), HOD assignment goes through
   `offering_for_confirm`, and cohort finalization refuses when the cohort does not fit. Regression test:
   `cohort_finalization_refuses_when_the_offering_cannot_hold_the_cohort`.
8. ~~**Catalogue course editing has no UI**~~ **Fixed**: `GET /admin/courses/:id/edit` renders the form and the
   page shows the course's append-only version history; saving appends a new version and never rewrites the
   previous one, and existing offerings stay pinned to the version they ran as.
9. ~~**Student actions are not audited**~~ **Fixed**: `select`, `withdraw`, `confirm` and `request_change`
   on `/hub/courses` each write an `audit_log` row. Bulk `auto_apply_fixed` rows are still only covered by
   the publish/status audit entry.
10. **Category taxonomies**: the *course* category list now lives once in `services::courses::CATEGORIES` and
    both the add and edit forms are generated from it (the database CHECK is unchanged and matches it).
    An *offering's* `course_type` is still free text, deliberately, so a new university category is data
    rather than a migration; a categories table with labels would be the next step if it needs policing.
11. Minor: the navigation now badges pending HOD appointments for the roles that decide them
    (`Shell::pending_approvals`); a combined count of pending selections and result corrections is still
    absent. `student_course_selections.state = 'submitted'` is reserved but unused (selections go
    draft → confirmed directly).

### Still open on the redesign brief

`done.md` tracks the brief section by section; these are the items it lists as unfinished, so this file is
not read as "nothing left".

12. ~~**Timetable integrity (brief §7, P0.10).**~~ **Fixed.** A period is now either taught by a validated
    teacher or explicitly marked `teacher_tbd`; a table CHECK (migration `0032`) makes that a schema
    invariant, the timetable and offering forms offer the choice, and the outstanding periods are a
    work-queue section. The two write paths that create a period share one service. Evidence:
    `tests/database.rs` — `a_period_is_taught_by_a_validated_teacher_or_explicitly_tbd`,
    `an_offering_period_is_queued_under_its_offering_department`; e2e
    `a_timetable_period_can_be_left_for_a_teacher_to_be_assigned`.
13. **Internals contributing to an official result (brief §7).** Internal assessments and university
    results are kept as separate rows keyed by `exam_kind`, and the semester lives on the row, but
    "whether an internal assessment contributes to the official result" is not modelled as a flag —
    today it is a matter of which rows are published.
14. **Audit detail (brief §6) — improved, still partial.** Rows carry
    actor/action/entity/entity_id and the log is append-only by trigger; every request now also carries a
    server-generated `correlation_id` (migration `0033`, `src/correlation.rs` + middleware) shared by every
    row it writes. Structured `details` now cover result corrections, HOD appointments, role changes,
    account activation/deactivation, the bulk archive, appointment revoke, delegation grant/revoke, CMS
    publish/archive, settings save and backups. Remaining: many older calls still pass no detail, a few
    still discard their error with `let _ =`.
15. **Reusable UI components (brief §8) — started.** `src/ui.rs` is now the single definition of the status
    badge (markup *and* colour), rendered `|safe` and reached through a row method; `partials/empty_state.html`
    and `partials/form_errors.html` are shared partials reading a template field. Limitation: Askama 0.12 has
    no macros and its `{% include %}` cannot take parameters, so a *per-row* fragment must be built in Rust
    rather than shared as a partial — tables therefore remain consistent by convention.
16. **Authorization test depth (brief §9) — extended.** `tests/app.rs` now also drives, through the real
    router: `one_student_cannot_withdraw_another_students_selection`,
    `a_student_reaches_their_own_hub_but_not_the_staff_tools`,
    `a_teacher_cannot_mark_attendance_for_another_teachers_class` and
    `only_the_it_admin_publishes_or_deletes_cms_content`. The student-vs-student case proves the route scopes
    the write to the session student by asserting the other student's row survives.
17. ~~**JS behaviour coverage (brief §9).**~~ **Fixed.** `tests/js/app_hooks.test.mjs` loads the real
    `static/js/app.js` into a fake DOM and fires each `data-*` hook, asserting its effect (mark-all radios,
    select-all and its toggle, autosubmit, confirm, print). `cargo test --test js_hooks` runs it under Node;
    if Node is absent the test reports that and passes.

Previously flagged bugs that have since been fixed:

- `/admin/departments/attendance` now uses the `/.../:entry_id/:date` routes, matching the
  `Path((entry_id, date))` the handlers take. (The save handler's error redirect was pointing at a
  query-string URL that no route matched and has been corrected to a path.)
- `src/cli.rs` `create-user` no longer double-logs `user_created_cli`, and `prompt_new_password` returns
  its error instead of panicking.
- Study materials now have a service, routes and UI: a head of department (or the IT admin) uploads a
  file for a course at `/admin/materials`, and an enrolled student downloads published material from
  `/hub/materials` (gated at the upload-serving boundary too).

## Changes queued for later (student dashboard rework)

Written down so they can be picked up in a later pass.

Attendance
- The third attendance option is now called **Special** in the marking UI
  (teacher sheet, HTMX badge, radio forms) and is drawn in yellow/amber;
  it replaces the old "Leave" wording. Saved summaries also say "special".
- The stored status value in the database is still `leave` (`attendance.status`
  CHECK), and the admin report tables/columns still say "Leave". Renaming the
  stored value needs a migration plus query changes — do it later, or leave the
  internal name as an implementation detail.

Still to build (from the dashboard rework request)
- Teaching staff page (`/about/staff`): cards generated from existing faculty
  data, grouped by department, HOD listed first, with the rank-holders style
  options.
- Non-teaching staff page: manual data with an admin edit UI and staff profile
  photo upload; also add the profile-photo capability for students at the
  model/service level only (no student UI yet).
- CSV student upload: header `admissionno,prn,email,phone` — uploaded rows are
  sent as a verification request to the student's HOD (or an approved approver)
  who reviews and confirms before the accounts are created.
- Review the e-grants removal leftovers: the `egrants`/`import_rows.egrants`
  columns remain in the database (unused) and could be dropped in a cleanup
  migration.

Already done in this pass
- Home page hero: crossfading slideshow of `static/img/carousel` photos
  (WebP slides generated as `slide-1..5.webp`), fixed stacking order.
- Pages admin: site menu is database-driven (nav group/label/order per page),
  sections and home blocks have an "Online" toggle and a photo caption,
  public pages send `Cache-Control: no-cache`.
- E-grants removed from the UI and attendance checking.
