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
       # http://127.0.0.1:3000

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
  A published course offering is scheduled from the same page (the offering picker): the panel lists that
  offering's periods and adds or removes them in place, with the same clash rules — the offering page's
  "add period" form stays as the secondary path.
- **Work queue** (`/admin/work-queue`, first item for a head of department): the real work in one list —
  pending external offering approvals, unanswered change requests, cohort decisions that are not finalised,
  published offerings with no timetable periods, and offerings with no seats left. Every row links to the
  catalogue, offering-detail or selections page that handles it; those pages are unchanged.

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

Create the first IT administrator (you will be asked for a password at a hidden prompt):

    cargo run -- create-user admin you@example.com "Your Name"

Create test accounts for the other roles. These must choose a new password at first sign-in:

    cargo run -- create-user staff office@example.com "Office Staff"
    cargo run -- create-user faculty teacher@example.com "Test Teacher"
    cargo run -- create-user student 1001@college.local "Test Student"

Then run `cargo run` and open http://127.0.0.1:3000/login. Each role lands on its own dashboard:
IT admin and office staff on `/admin`, teachers on `/teacher`, students on `/hub`.

Rules built in: five wrong passwords lock an account for 15 minutes; sessions last 8 hours of inactivity;
every form carries a CSRF token; passwords are stored with Argon2.
In production set `COOKIE_SECURE=true` and serve the site over HTTPS.

## Building the CSS

Windows (PowerShell, from the project folder):

    powershell -ExecutionPolicy Bypass -File .\build-css.ps1          # one-off build
    powershell -ExecutionPolicy Bypass -File .\build-css.ps1 -Watch   # rebuild while you edit templates

The script downloads the standalone Tailwind CLI (v3.4.17, no Node needed) on first use and writes `static/css/tailwind.css`.
Rebuild whenever you add new Tailwind classes to a template. Colours and fonts live in `tailwind.config.js`.

On Linux or macOS, download the matching `tailwindcss` v3.4.17 binary from the Tailwind releases page and run:

    ./tailwindcss -c tailwind.config.js -i assets/input.css -o static/css/tailwind.css --minify

## Sign in and accounts (Phase 4a)

Create the first IT administrator (you will be asked for a password at a hidden prompt):

    cargo run -- create-user admin you@example.com "Your Name"

Create test accounts for the other roles. These must choose a new password at first sign-in:

    cargo run -- create-user staff office@example.com "Office Staff"
    cargo run -- create-user faculty teacher@example.com "Test Teacher"
    cargo run -- create-user student 1001@college.local "Test Student"

Then run `cargo run` and open http://127.0.0.1:3000/login. Each role lands on its own dashboard:
IT admin and office staff on `/admin`, teachers on `/teacher`, students on `/hub`.

Rules built in: five wrong passwords lock an account for 15 minutes; sessions last 8 hours of inactivity;
every form carries a CSRF token; passwords are stored with Argon2.
In production set `COOKIE_SECURE=true` and serve the site over HTTPS.

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
    migrations/     0001 core, 0002 academics, 0003 content, 0004 triggers, 0005 starter content,
                    … 0013 course offerings (courses, offerings, targets, approvals, selections,
                    change requests), 0014 updated_at columns, 0015 studash, 0016 page menu,
                    0017 enrollments.offering_id

## Layers

1. Design system and templates (done)
2. Routing structure for all public pages (done: pages registry, programme/department/news/notice routes, sitemap.xml, robots.txt)
3. PostgreSQL schema, migrations, SQLx services (done)
4. Authentication, Student Hub, teacher tools, admin
   - 4a (done): sign-in, sessions, CSRF, roles, dashboard shells, `create-user` command
   - 4b-1 (done): admin tools for people, programmes/courses, enrollment and the timetable builder
   - 4b-2: publishing tools (notices, news, events, documents, pages, settings) for IT admin and office staff
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
- Students, the timetable and attendance now have models and queries in `services/hub.rs` (Student Hub).
  The marks and exam tables still have no Rust models yet; they arrive with the Student Hub's exam views.
- The header and footer still carry the college phone number as fixed text. Contact details on the home and contact pages come from `site_settings`.
- Set `SITE_URL` in `.env` so `sitemap.xml` and `robots.txt` use your real domain.
- Images on the home page currently hotlink the live site and move to `uploads/` when the content is migrated.

## Known limitations (to do later)

Review of the course management & selection feature (branch `feature/curriculamv2`, verified 6 Oct 2026).
The feature works end to end — catalogue → offering → publish → cross-department approval → selection
(FIXED / HOD-assigned / individual choice / cohort choice) → enrollment → timetable and attendance —
but these gaps remain:

1. **Results section never shows offering courses.** `hub::internal_results` filters on `courses.semester`,
   which is `NULL` for catalogue courses, so a confirmed offering appears in *Enrolled courses* but not in
   `/hub` Results ("not enrolled in any courses this semester"). Take the semester from the student's
   offering/selection instead of the catalogue row.
2. **No marks/exams entry UI exists at all** (tables exist since migration 0002), and `exams.programme_id`
   is `NOT NULL`, so an exam can never attach to an offering course. Marks themselves are course-based and
   would work once a UI and the Results query exist.
3. **Substitutions skip offering periods**: `attendance::teacher_day` joins `programme_id`, so an offering
   period never appears in the substitute-teacher candidate list.
4. **Clash detection still misses offering-vs-offering overlap**: a programme slot and an offering slot now
   see each other (`academics::clashes` and `academics::offering_clashes` share the same teacher/room/class
   rules, and the offering page's "add period" clash-checks too), but two *different* offerings that target
   the same programme can still be scheduled on top of one another — "same class" covers programme periods
   and the offering's own periods only.
5. **Programme timetable grid** (`academics::slots`) still filters `programme_id`, so offering periods are
   not in the grid rows; they now have their own panel on the same `/admin/timetable` page (offering picker),
   and are also visible on the offering page, `/hub` and teacher pages.
6. **Attendance reports disagree under a programme filter**: `by_session` filters `c.programme_id` while
   `totals`/`by_student` filter `st.programme_id` — the session table can be empty while the totals are not.
   The course-filter dropdown (`course_options`) also hides catalogue courses when a programme is selected.
7. **Capacity is enforced only at student confirm**; HOD assignment and cohort finalization can exceed it.
   Decide whether that override is intended, then enforce or document it.
8. **Catalogue course editing has no UI**: `POST /admin/courses/:id/edit` exists (with audit) but nothing
   posts to it — the catalogue only offers *Retire*, so typos in code/title/credits need SQL to fix.
9. **Student actions are not audited**: select/withdraw/confirm/change-request on `/hub/courses` write no
   `audit_log` rows (all 16 admin/HOD mutations do). Bulk `auto_apply_fixed` rows are only covered by the
   publish/status audit entry.
10. **Category taxonomies disagree**: `courses.category` is a fixed CHECK enum, the offering-create select
    hardcodes a different list, and offering-edit is free text; students only ever see `course_type`.
    A categories table would make this data-driven as intended.
11. Minor: no pending-approval badge in the nav; `student_course_selections.state = 'submitted'` is reserved
    but unused (selections go draft → confirmed directly); ~9 dead-code warnings from struct fields fetched
    but never rendered in `services/courses.rs`.

Pre-existing bugs, present before this branch (flagged during the same review):

- `/admin/departments/attendance/:entry_id` and `.../save` always return 500 — the route passes one path
  argument while `mark_form`/`mark_save` take `Path((entry_id, date))`.
- `src/cli.rs` `create-user`: a duplicated `users::audit` line logs `user_created_cli` twice; `maybe_pw` is
  bound but unused (the password is read from `args.get(3)`); `prompt_new_password(...).unwrap()` panics
  instead of returning the error.
- No study-materials UI either (`study_materials` table exists since 0002).

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
