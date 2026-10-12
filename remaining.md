# What is left, how the app is put together, and how to operate it

Three things in one file, for whoever picks this up next:

1. **Part 1 — Architecture and structure**: the shape of the process, the routers, the authorization
   model, the data layer, the templates and the tests. Read this before changing anything.
2. **Part 2 — The remaining work**: every item still open, with where to change it and how to prove it.
3. **Part 3 — Recipe: adding a page** and **Part 4 — CLI and operations**: the exact commands.

Companion documents, and what each one is for:

| file | what it is |
| --- | --- |
| `redesign.md` | the original brief (requirements, sections quoted as §n throughout the code comments) |
| `done.md` | the brief tracked section by section, with the evidence for each "done" |
| `fixes.md` | the *first* audit — history, not a current status list |
| `README.md` | how to run it, the feature tour, and the known limitations list |
| `remaining.md` | this file: architecture, what is left, how to implement it, how to operate |

Current state: **`cargo test --no-fail-fast` → exit 0**; 83 unit + 40 database + 31 end-to-end + 1 Node
test, 0 failures.

---

## Part 1 — Architecture and structure

### 1.1 The process: two listeners

`src/main.rs` is deliberately thin: environment → pool → migrations → `site::get` + refresher →
maintenance/backup task → two routers → two listeners.

| listener | address (default) | serves |
| --- | --- | --- |
| public | `HOST`/`PORT` → `127.0.0.1:3000` | the public site, `/login`, `/logout`, `/account/*`, the teacher pages `/teacher*`, the student hub `/hub*`, `/dashboard/teacher/*`, the **management area `/admin/*`** (for the IT admin, the Principal and office staff), the **office-staff workspace `/staff/*`**, and the **department workspace `/hod/*`** |
| management | `MANAGEMENT_HOST`/`MANAGEMENT_PORT` → `127.0.0.1:3001` | `/login`, `/logout`, `/account/*`, `/admin/*` including root authority, `/staff/*`, and `/hod/*` |

The management address is validated at startup and the process **panics** on a non-loopback bind
(`src/config.rs`), so the boundary is a network fact rather than a navigation convention. Because a head of
department is a `faculty` account, office staff and the IT admin sign in on the *public* listener, the
management and department pages are mounted on both listeners (see 1.3). The one role that stays
loopback-only is the **Super Admin**: the public mounts are wrapped by
`auth::management_area_on_public`, which refuses a `faculty` account and root authority there.

### 1.2 Middleware and configuration

`apply_layers()` (`src/lib.rs`) wraps both routers, so the two listeners behave identically apart from their
route sets:

- **session**: `tower-sessions` with the PostgreSQL store (`nss_session` cookie, `SameSite=Lax`, 8 hours of
  inactivity, `Secure` when `COOKIE_SECURE=true`).
- **correlation id**: `axum::middleware::from_fn(with_correlation)` stamps each request from
  `src/correlation.rs` — generated server-side, never read from the request — and every audited row written
  while handling it carries that id (migration `0033`).
- **security headers**: `X-Content-Type-Options`, `X-Frame-Options`, `Referrer-Policy`, the CSP (scripts
  `'self'` only: no CDN, no inline handlers — every `on*` was replaced by a `data-*` hook in `app.js`), and
  `Permissions-Policy`; `Strict-Transport-Security` only when `COOKIE_SECURE=true`.
- **compression** and **tracing**, plus `/static` served from disk on both listeners.
- `Cache-Control`: `no-store` on the signed-in and admin trees (handler-set, with the public site
  `no-cache`), so the back button after sign-out shows nothing private.

`src/config.rs` reads the environment once. `src/site.rs` caches `site_settings` in memory (the error pages
and the dashboard chrome have no database handle of their own) and refreshes it on a timer.

### 1.3 Routers: one handler set, three workspaces

```
public_router()                    management_router()
├── public site pages              ├── /admin dashboard (AdminDashboard)
├── auth_routes()                  ├── admin::routes()
├── signed_in_public_…             │   ├── technical routes   /admin/…      (AdminOnly, OfficeOrAdmin, …)
│   /teacher* /hub* …              │   └── .nest("/admin", manager_routes().merge(content_routes()))
├── /admin + /staff                ├── /staff  (admin::staff_routes())
│   + /hod  (from `routes/mod.rs`) └── /hod    (department_routes())
└── wrapped by `auth::management_area_on_public`
```

- `admin::routes()` (`src/routes/admin/mod.rs`) holds the **technical** pages — People, Settings, the
  college-wide attendance report, appointments, root authority — at their absolute `/admin/...` paths, plus
  `.nest("/admin", manager_routes().merge(content_routes()))`.
- `admin::manager_routes()` holds every **department** page whose handler takes `Manager` — the work queue,
  the section index (`/manage/:key`), departments + attendance, academics/enrol, timetable, substitutions,
  courses/offerings/external/selections, exams, marks + corrections + imports, materials — plus the
  per-user notification feed, written with **relative** paths. It is mounted at `/admin` and, via
  `routes::department_routes()`, at `/hod` on both listeners.
- `admin::content_routes()` holds the CMS tools (notices, news, events), also with **relative** paths, so
  the same handlers serve `/admin/notices` for the IT admin and `/staff/notices` for office staff.
- `admin::staff_routes()` is the `/staff` office-staff workspace: the dashboard, the section index, the
  notification feed and `content_routes()`. It is mounted on both listeners by `routes::staff_routes()`.
- Mounting a second time grants nothing: the guards run per request and are identical, so a caller who is
  not allowed is refused wherever they try. The public `/admin` and `/staff` mounts are additionally
  wrapped by `auth::management_area_on_public`, which refuses a `faculty` account and a Super Admin there.

**The workspace is one value, not a duplicated URL table.** Every management link is written once in its
canonical `/admin…` form and rewritten on render:

| piece | file | rules |
| --- | --- | --- |
| `ADMIN_BASE` / `HOD_BASE` / `STAFF_BASE`, `base_for(user)` | `src/sections.rs` | `/hod` for a `faculty` account, `/staff` for office staff, `/admin` otherwise |
| `in_workspace(base, path)` | `src/sections.rs` | swaps a leading `/admin`; anything else (`/teacher/reports`) is untouched |
| `Link::url(base)` | `src/sections.rs` | the area catalogue's links, absolute hrefs kept canonical |
| `NavItem::url(base)` | `src/shell.rs` | the sidebar, rendered in `layouts/app.html` |
| `Notification::href_in(base)` | `src/services/notifications.rs` | a feed row's target page |
| `Shell::base` / `Shell::home` | `src/shell.rs` | the workspace and the caller's own dashboard (breadcrumbs) |
| `ws(&manager, path)` | marks, exams, results_import, course_offerings | redirects and `back` fallbacks |
| `{{ shell.base }}` / `{{ panel.base }}` | the department templates | form actions, links, HTMX targets |

Two consequences worth remembering: a new department page must build its URLs from `shell.base` (or a
`ws()`-style helper), and the timetable's HTMX panels carry their own `base` field because they are rendered
standalone (`PanelTemplate`) as well as included in the page.

### 1.4 Authorization

`src/auth/mod.rs` is the whole model. Guards are axum extractors, so a missing one is a compile error and a
guard is impossible to forget on a handler that takes it:

| extractor | allows | notes |
| --- | --- | --- |
| `AuthUser` | anyone signed in | session → `users` row; a `session_version` mismatch flushes the session (role change, password reset, deactivation) |
| `Ready` | a signed-in user who replaced their temporary password | otherwise redirects to `/account/password` |
| `AdminOnly` | `admin`, `super_admin` | the IT admin's technical powers |
| `AdminOrPrincipal` | + `principal` | the appointments screen |
| `AdminDashboard` | + `staff` | the `/admin` overview |
| `OfficeOrAdmin` | `admin`, `super_admin`, `staff` | CMS authoring (notices, news, events) — create, edit, publish, archive, delete |
| `StaffOnly` | `staff` | the `/staff` workspace overview |
| `PrincipalOnly`, `TeacherOnly`, `StudentOnly`, `SuperAdminOnly` | one role each | `SuperAdminOnly` is root authority: `/admin/roles`, recovery, reopening finalized marks |
| `Manager` | `admin`, `super_admin`, `principal` → `Scope::All`; a `faculty` account with an **effective** HOD appointment or delegation → `Scope::Department(id)` | resolved per request from `services::appointments`, not from a session flag; no appointment (or no department) is a denial |
| `management_area_on_public` (middleware) | `admin`, `principal`, `staff` | mounted only on the public `/admin` and `/staff` mounts; refuses a `faculty` account and root authority, using the role recorded in the session at sign-in |

Rules that hold everywhere: authorization is server-side and derived from the session, never from a
form field; a department-scoped write re-checks that the row belongs to the guard's department (the id in
the form is never trusted); `Manager::department()` returning `None` means "the whole college" *only* for
the IT admin, while a missing department on an appointment is a denial.

### 1.5 Data layer

- **Migrations**: `migrations/NNNN_name.sql`, 33 of them, applied by `migrator()` (`sqlx::migrate!`) at
  startup and by every `#[sqlx::test]`. **Never edit an applied migration** — SQLx checksums them and
  startup fails with the applied set printed. Add a new numbered file instead.
- **Services** (`src/services/*.rs`) own every SQL statement and the business rules; route handlers are thin:
  parse the form, call the guard, call a service, audit, flash a message, redirect.
- Queries are runtime (`sqlx::query_as`), not the compile-time macros, so the project builds without a
  database. A typo is caught by a test, not by the compiler — which is why the database tests matter.
- **Audit**: `users::audit` / `users::audit_detail` write `audit_log` (append-only by trigger), each row
  carrying the request's correlation id. Sensitive operations must be audited.

### 1.6 Presentation

- **Askama 0.12** templates in `templates/`, 100 files: `{% extends "layouts/app.html" %}` for the signed-in
  areas, `{% include %}` for shared fragments. **There are no macros**, and `{% include %}` cannot take
  parameters, so a per-row fragment that needs one cannot be a partial (see Part 2, item 14).
- `src/ui.rs` is the single definition of the status badge (markup *and* colour), rendered `|safe`.
- `src/sections.rs` holds the three area catalogues (`ADMIN`, `STAFF`, `HOD`) that the dashboards, the work
  queue and `/admin/manage/:key` render as cards; the catalogues only *mirror* the guards.
- **Tailwind v3.4.17** via the standalone CLI (`build-css.ps1` / `build-css.sh`). `tailwind.config.js`
  scans `templates/**/*.html`, `static/js/**/*.js` **and `src/**/*.rs`** (because `src/ui.rs` builds class
  names in Rust). After touching classes anywhere, rebuild — and note a class that exists only inside a Rust
  string literal is only emitted because of that `src/**/*.rs` entry.
- Client behaviour is `static/js/app.js` (menu, nav highlighting, mark-all, select-all, autosubmit,
  confirm, print). No inline script, no CDN: the CSP is `script-src 'self'`.

### 1.7 Tests: four layers

| layer | command | what it is for |
| --- | --- | --- |
| unit | `cargo test --lib` | pure rules: date validation, reauth window, `Role` parsing, section catalogues, password rules, checkbox parsing, `ui` badge colour — no database |
| database | `cargo test --test database` | `#[sqlx::test]` gives each test a fresh migrated database: services, constraints, workflows (appointments, capacity, corrections, migrations themselves) |
| end-to-end | `cargo test --test app` | the real routers through `tower::oneshot` (routing, session, CSRF, templates, headers) — including the authorization matrix and the `/hod` workspace |
| JS | `cargo test --test js_hooks` | runs `tests/js/app_hooks.test.mjs` under `node --test` against the real `app.js` in a fake DOM; skips with a note if Node is missing |

Everything needs PostgreSQL: `docker compose up -d` first, then
`DATABASE_URL=postgres://nss:nss@localhost:5432/nss_college cargo test`.

### 1.8 Directory map

```
src/
  main.rs            process setup, the two listeners
  lib.rs             app as a library: connect, migrator, init_session_store, apply_layers
  config.rs          environment → Config (and the loopback panic)
  state.rs           AppState (db, upload_dir, login_limiter)
  error.rs           AppError → 404/403/400/500 templates
  auth/              guards, CSRF, password hashing, reauth        sections.rs  area catalogues, workspace base
  shell.rs           nav + Shell (user, base, home, csrf, flash, badge)
  ui.rs              shared UI fragments built in Rust            correlation.rs  per-request audit id
  routes/
    public.rs about.rs htmx.rs seo…            the public site
    auth.rs                 login/logout/password/reauth
    dashboards.rs teacher.rs hub_pages.rs hub_courses.rs
    admin/                  mod.rs (route sets) + one file per screen
  services/                 all SQL and business rules, one file per domain
templates/   layouts, partials, public, auth, account, dashboard, teacher, hub, admin
migrations/  33 SQL files        static/  css (tailwind), js, img        tests/  app, database, js_hooks, js/
```

---

## Part 2 — The remaining work

Ordered so that the cheap, self-contained items come first and the structural ones last. Each item says
**what is missing**, **where to change it**, and **how to verify it**. Nothing here is invented: item
numbers in parentheses point at `done.md` §3 or the README's lists, where the evidence for the finished
half is recorded.

### A. Start here — small and self-contained

**1. Two missing regression tests (done.md §3.B.13, §3.B.14).** Both features were "already handled by
inspection" and neither has a test: a substitution candidate resolving an offering period's programme from
`course_offering_targets`, and two offerings that target the same programme/semester clashing.
*Where:* `tests/database.rs` — build a programme, a catalogue course (`programme_id IS NULL`) and an
offering targeting the programme, then assert `academics::offering_clashes(db, &NewOfferingSlot)` reports
the second offering and `attendance::teacher_day(db, teacher_id, date)` names the programme (it returns
`Vec<CoverCandidate>`, the same rows the substitutions page lists). *Verify:* the two new tests; they are
the point.

**2. An appointed HOD still lands on `/teacher` after sign-in.** The sidebar gets them there, but the
landing page does not mention their department work. *Where:* `src/routes/auth.rs` (the post-login
redirect) or a new `home_for(&AuthUser)` helper next to `Role::home()` in `src/auth/mod.rs`, so a `faculty`
account with `manages_department()` goes to `/hod/work-queue`. *Verify:* a `tests/app.rs` test that signs in
through `/login` and asserts the redirect `Location`.

**3. ~~A management role that signs in on the public listener lands on a 404.~~ Resolved.** The
management area (`/admin`, including the technical tools) and the office-staff workspace (`/staff`) are now
served on both listeners, so an IT admin, Principal or staff member reaches them from `:3000`. Office staff
have their own workspace (`/staff`, `sections::STAFF_BASE`) rather than `/admin`, and `Role::home()` sends
them there. Root authority is the deliberate exception: `auth::management_area_on_public` reads the role
recorded in the session at sign-in and refuses both a `faculty` account and a Super Admin on the public
mounts, so `/admin/roles`, recovery and the emergency reopening of a finalized result remain loopback-only.
*Verify:* e2e `an_it_admin_works_on_the_public_listener_but_root_authority_does_not`,
`office_staff_work_from_the_staff_workspace_on_the_public_listener`, and
`an_hod_works_from_the_public_listener_under_hod` (which now asserts `/admin/marks` → 403 on `:3000`).

**4. The scheduled backup fails on a host without `pg_dump` on PATH** (pre-existing; unrelated to the
redesign work). Today the maintenance task logs an error every interval and nothing surfaces it.
*Where:* `src/services/backup.rs` (detect the missing binary once and fail with one actionable message;
`BACKUP_DOCKER_SERVICE` already supports dumping through Compose), `src/services/maintenance.rs` (report the
last outcome), and optionally the work queue (`src/routes/admin/work_queue.rs`) so an operator sees "last
backup failed" instead of discovering it. *Verify:* a unit test for the "no pg_dump" decision, plus
`cargo run -- backup --list` on a machine with and without the binary.

**5. `create-user` usage line omits `alumni` (one-line fix).** The command accepts it (`Role::parse`
returns `Alumni`, and the error message lists it), the usage block printed for a bad command does not, and
the unit test `create_user_accepts_every_role_its_usage_line_documents` asserts all seven roles parse — so
the test name and the usage string disagree with each other. *Where:* the usage strings in `src/cli.rs`
(the `run` fallback and `create_user`). *Verify:* update the test to compare against the same list the usage
line prints, so the two cannot drift again.

**6. Documentation staleness (done.md §3.C).** README items #1–#7 map onto `done.md` §3.B; `fixes.md` §1–§12
are the original audit and only §13–§15 were implemented. *Where:* fold the two lists into one status map
(or mark them as history) so the next reader does not treat either as current.

### B. Medium — one screen or one service at a time

**7. Audit detail is still partial (done.md §3.B.16, README #14).** `audit_detail` writes structured JSONB
for corrections, appointments, roles, activation/deactivation, the bulk archive, revokes, delegations, CMS
publish/archive, settings and backups — other calls still pass no `details`, and a few discard their error
with `let _ =`. *Where:* `grep -rn "users::audit(" src/` for the call sites; give each one the fields that
make the row useful (old/new value, ids, counts), and replace `let _ =` with `?` (an audit write that fails
should fail the operation it describes) or with an explicit `tracing::error!` where best-effort is
deliberate. *Verify:* a `tests/database.rs` assertion per newly detailed action, like
`an_audit_detail_row_keeps_its_structured_payload`.

**8. Notifications for deadlines and failed imports (done.md §3.A.4 remaining).** The feed covers
appointment requests and correction requests only. *Where:* `services::notifications::create` at the
failure points — a rejected student-import batch (`services/import_students.rs`), a rejected result-import
commit (`routes/admin/results_import.rs`) — plus a sweep in `services/maintenance.rs` (already scheduled)
that raises "published offering with no timetable", "correction waiting longer than N days" and "period with
no teacher" to the department's HOD. *Verify:* database tests for each new notification, and one e2e test
that the feed renders its rewritten target inside `/hod`.

**9. The navigation badge counts appointments only (README #11, minor).** *Where:* replace
`Shell::pending_approvals: usize` with a small struct (`approvals`, `selections`, `corrections`) or a summed
`pending_work`, filled in `Shell::build` from the three counts, and render them on their own nav items in
`templates/layouts/app.html`. *Verify:* unit test on the counting helper; e2e test that the badge appears
for the roles that decide each kind.

**10. An exam row belongs to a programme, not to the course version an offering ran as (README #2).**
`exams.programme_id` is `NOT NULL` (migration `0002`), so a catalogue course carried by an offering cannot
be anchored the way marks and enrolments now are. *Where:* a migration adding `course_id` (and
`course_offering_id` where relevant) with a backfill from the programme's course, relax `programme_id`-only
reads in `services/exams.rs` and `routes/admin/exams.rs`, and keep `academics::applicable_course` as the one
applicability rule. *Verify:* a database test that an offering's catalogue course appears in the exam picker
and in the student result read; the existing timetable/exam tests must keep passing.

**11. "Does this internal assessment contribute to the official result?" (README #13).** Today it is
implicit in which rows are published. *Where:* a column on the assessment/exam-kind definition (or a small
`assessment_kinds` table like `services::courses::CATEGORIES`), the marks page, and wherever a semester
result is assembled (`services/exams.rs`). *Verify:* a database test that an unpublished internal row is
excluded and a flagged one included, plus the decision being visible in the UI.

**12. Offering `course_type` is free text (README #10).** Deliberate so far; the next step if it needs
policing is a `course_types` table with labels, an FK replacing the free-text column, and admin CRUD.
*Verify:* the existing offering tests plus one for an unknown type being refused.

**13. Attendance status naming: `special` in the UI, `leave` in the database (README, queued).**
*Where:* one migration (`ALTER TABLE … CHECK`, `UPDATE` the stored value) plus the query and report wording
that still says "Leave". *Verify:* the attendance report tests and an e2e marking test with the new value.

### C. Larger — infrastructure or a new feature

**14. Per-row fragments cannot be partials (done.md §3.B.17, README #15) — the Askama limit.**
`src/ui.rs` builds the status badge in Rust because Askama 0.12 has no macros and parameterless-only
includes. *Options:* (a) leave it — one Rust renderer per shared row fragment is working and testable;
(b) upgrade Askama to a version with `{% macro %}`/`{% call %}` and move the fragments into
`templates/partials/`, accepting a template-wide review (100 files render in the test suite, so the
regression safety net exists) and a check that `|safe` behaviour is unchanged. *Verify:* the full suite,
plus a diff of the rendered badge markup before/after.

**15. No real-browser test (done.md §3.B.19).** The JS test uses a fake DOM, so layout and `hx-*` behaviour
are untested. *Where:* a `tests/browser/` Node+Playwright suite driven by a Rust test that starts the app on
a test port (the in-process routers cannot be used over Playwright), gated so `cargo test` still passes
where no browser is installed. *Verify:* the new suite against the two listeners with the seeded demo
accounts (`seed-demo-users`), covering the HTMX attendance sheet, the timetable panel and the mobile nav.

**16. Student account creation with HOD verification (brief §5; README "still to build").** A CSV
(`admissionno,prn,email,phone`) should become a *verification request* to the student's HOD, who confirms
before accounts exist. *Where:* a `verification_requests` table (migration), a service reusing
`services/import_students.rs` for parsing, a notifications hook (item 8), an HOD screen under the
department workspace following Part 3's recipe, and account creation only on confirm
(`services/people.rs`). *Verify:* database tests for request→confirm→account, an authorization test that
another department's HOD cannot confirm it, and an e2e test of the HOD screen.

**17. Non-teaching staff page + profile photos (README "still to build").** A CMS-managed staff page
(cards from existing faculty data, grouped by department, HOD first, with the rank-holders display options
reused), a manual non-teaching staff section with photo upload, and `students.photo_path` at model/service
level. *Where:* `services/faculty.rs` exists for the teaching list; photos ride the existing `uploads`
service and `content_admin`-style admin forms; `uploads` authorization in `routes/public.rs` must cover the
new folders. *Verify:* an e2e upload test (the file is written, the row points at it, the public route
serves it only when it should) and a public render test.

**18. Housekeeping migrations.** Drop the unused `egrants` / `import_rows.egrants` columns once no read
path mentions them; keep `fixes.md`'s remaining items in view. *Verify:* grep, then the full suite.

**19. Dependency health.** `sqlx-postgres v0.7.4` is flagged as future-incompatible
(`cargo report future-incompatibilities`). A SQLx upgrade is its own task: queries are runtime, but the
session store (`tower-sessions-sqlx-store`) and the migrator are pinned alongside it, so upgrade both and
re-run every layer.

**20. Known limitation of the current split, recorded in `README.md`.** A management role on the public
listener (item 3) and the absence of a combined pending-work badge (item 9) are the two consequences of the
listener design that a reader is most likely to trip over; whatever you decide there, write it down.

---

## Part 3 — Recipe: adding a page

The shape is always the same five steps. Example: "the department's substitution history" at
`/hod/substitutions/history`.

1. **Pick the guard first.** Ask what the *narrowest* authority is: `Manager` if a head of department and
   the IT admin both use it, `AdminOnly` if it is technical. The guard is an extractor, so nothing renders
   or runs before it passes.
2. **Choose the router.** A `Manager` page goes in `src/routes/admin/mod.rs::manager_routes()` with a
   **relative** path (`/substitutions/history`) — it then exists at `/admin/...` and `/hod/...` with the
   same guards. A technical page goes in `routes()` with its absolute `/admin/...` path.
3. **Handler** in the matching file under `src/routes/admin/`, thin: parse the form/query, call a service in
   `src/services/`, `users::audit` the write, `shell::flash` a message, redirect **inside the workspace**
   (`format!("{}/substitutions/history", sections::base_for(&manager.user))`, or a `ws()` helper).
4. **Template** in `templates/admin/…` extending `layouts/app.html`, and **every URL built from
   `{{ shell.base }}`** — never a literal `/admin/`. Data tables use `partials/empty_state.html` and the
   `src/ui.rs` badge for their status column.
5. **Tests**: a unit test if a new rule is pure; a `tests/database.rs` test for the service; a
   `tests/app.rs` test for the authorization (who is allowed, who is refused) — and if the page is in a
   catalogue or the sidebar, add it to `src/sections.rs` / `src/shell.rs` so the nav exposes it, which the
   existing consistency tests then check.

Then: rebuild the CSS if any class is new (`.\build-css.ps1`, `bash build-css.sh`), and run
`cargo test --no-fail-fast`.

---

## Part 4 — CLI and operations

### 4.1 Prerequisites

```bash
docker compose up -d                 # PostgreSQL 16 on localhost:5432
cp .env.example .env                 # DATABASE_URL etc. (see the table below)
cargo run                            # migrations run automatically, then both listeners start
```

`src/cli.rs` runs **instead of** the server when the first argument is a known command; anything else prints
the usage block and exits 2. Passwords are typed at a hidden prompt (`rpassword`) and never appear in the
shell history, unless the optional password argument is given (automation only).

### 4.2 The commands

```bash
# Create an account. Role is one of super_admin|admin|principal|staff|faculty|student|alumni.
# The password is prompted (min 8 characters, twice). The 4th argument is for
# automation/bootstrapping only — it is validated against the same rule.
cargo run -- create-user admin you@example.com "Your Name"
cargo run -- create-user super_admin owner@example.com "Site Owner"
cargo run -- create-user faculty teacher@example.com "Test Teacher" 'Tmp-Passw0rd'
# must_change_password is false for super_admin and admin (they set their own here),
# true for everyone else — the first sign-in forces a new one.
```

```bash
# Give or remove root/technical authority. Only super_admin and admin are accepted:
# academic authority (principal) is an appointment, and departmental roles live on the
# appointments screen. Runs the same rule as /admin/roles, so the last active Super Admin
# can be promoted but never demoted, and the change revokes the account's live sessions.
cargo run -- set-role super_admin someone@example.com
cargo run -- set-role admin someone@example.com

# Replace a password (the break-glass path when a reset would need an email flow).
# Also clears any failed-sign-in lockout, so the new password works at once.
cargo run -- set-password someone@example.com
```

```bash
# Demo accounts for local work. Prints a ROLE/NAME/LOGIN/PASSWORD table and rewrites the
# password of any account that already exists, so the printed credentials always work.
# Students sign in with their admission number, not the email.
cargo run -- seed-demo-users
cargo run -- seed-demo-users --credentials-file /tmp/demo.txt   # created 0600, keeps passwords out of the scrollback
```

Seeded accounts: `owner@college.local` (super_admin), `admin@college.local` (admin),
`office@college.local` (staff), `hod@college.local` (faculty, head of the first department),
`teacher1@college.local`, `teacher2@college.local`, `student1..5@college.local`.

```bash
# Backups: one run now, or list what a restore could use. Retention and scheduling come
# from the environment (BACKUP_DIR, BACKUP_KEEP, BACKUP_INTERVAL_HOURS, BACKUP_ENABLED,
# BACKUP_DOCKER_SERVICE). The scheduled task runs inside the server process.
cargo run -- backup
cargo run -- backup --list

# Housekeeping: expired sessions, stale student-import batches, expired and old staged
# result imports. Prints how many rows of each were purged. Runs on a timer too.
cargo run -- maintenance
```

On a deployment, the same seeding can happen at start-up instead of by hand, writing the credentials to a
file (0600) rather than the log — unset both variables once the accounts exist:

```bash
SEED_DEMO_USERS=true SEED_CREDENTIALS_FILE=/run/secrets/demo-credentials.txt cargo run
```

### 4.3 Migrations

Add `migrations/NNNN_short_name.sql` (next number; 33 exist today). They are applied automatically at
startup and by every `#[sqlx::test]`; there is no separate migrate command. Two rules: **never edit an
applied migration** (SQLx checksums the files — startup fails and prints the applied set with the
mismatched checksum), and a destructive change needs a `--` comment at the top saying why, because this is
the file someone will read years later.

If startup does fail on migrations, the error log lists `v<version> <description> success=<bool>
checksum=<hex>` for what the database already has.

### 4.4 Environment variables

| variable | default | meaning |
| --- | --- | --- |
| `DATABASE_URL` | — (required) | PostgreSQL connection string |
| `HOST` / `PORT` | `127.0.0.1` / `3000` | the public listener |
| `MANAGEMENT_HOST` / `MANAGEMENT_PORT` | `127.0.0.1` / `3001` | the management listener; a non-loopback host **panics** at startup |
| `UPLOAD_DIR` | `uploads` | where uploaded files are written |
| `SITE_URL` | — | canonical URL for `sitemap.xml` / `robots.txt` |
| `COOKIE_SECURE` | `false` | `Secure` cookie + HSTS; set it (and serve HTTPS) in production |
| `RUST_LOG` | `nss_college=debug,tower_http=info` | tracing filter |
| `BACKUP_ENABLED` / `BACKUP_DIR` / `BACKUP_KEEP` / `BACKUP_INTERVAL_HOURS` | `true` / `backups` / `7` / `24` | scheduled backups |
| `BACKUP_DOCKER_SERVICE` | — | dump through `docker compose exec <service>` when `pg_dump` is not on PATH |
| `SEED_DEMO_USERS` / `SEED_CREDENTIALS_FILE` | — | one-off demo seeding on start-up |

### 4.5 Day-to-day commands

```bash
cargo run                                   # server (both listeners)
cargo test                                  # needs PostgreSQL; all four layers
cargo test --lib                            # unit only, no database
cargo test --test database                  # services, constraints, workflows
cargo test --test app                       # through the real routers
cargo test --test database some_test_name   # one database test
powershell -ExecutionPolicy Bypass -File .\build-css.ps1   # rebuild static/css/tailwind.css
bash build-css.sh --watch                                   # or watch while editing templates
```

### 4.6 Troubleshooting

| symptom | cause and fix |
| --- | --- |
| startup panics on migrations and prints the applied set | an applied migration was edited, or the database is ahead of the code. Restore the file, or add a new migration. |
| `could not connect to PostgreSQL` | `docker compose up -d` (the tests and the server share the dev database) |
| `MANAGEMENT_HOST must resolve to a loopback address` | the management listener was pointed at a non-local address; that boundary is deliberate — change the deployment, not this line |
| a page 404s only for one role | check the workspace: `/admin` is served on the management listener only, `/hod` on both. A management role on `:3000` sees no `/admin` (Part 2, item 3) |
| an HOD is refused (403) on a department page | they have no *effective* appointment: `Manager` reads `role_assignments`/`delegations`, so an expired appointment denies. Grant or renew it at `/admin/appointments` |
| "the scheduled backup task fails" in the log | `pg_dump` is not on PATH; set `BACKUP_DOCKER_SERVICE=db` (Part 2, item 4) |
| classes missing from the CSS | the class exists only in Rust (`src/ui.rs`) or a fresh template: rebuild the CSS; `tailwind.config.js` scans `src/**/*.rs` for exactly that reason |
| students cannot sign in by email | students sign in with their **admission number**; `owner/admin/office/teacher…` use the email |

---

## Part 5 — Before you call anything done

The standard this repository holds itself to, in order:

1. **Name the acceptance criterion first** — the exact path, payload, file name or status code the request
   asked for — then check the artifact against it.
2. **Run the full suite and read the exit status**, not a filtered tail:
   `cargo test --no-fail-fast > /tmp/test.log 2>&1; echo $?` — then look for `FAILED`/`error` in the log.
   A green build is not a green behaviour.
3. **Exercise the real interface.** For a route, that means a `tests/app.rs` request through the router
   (or a curl against a running server for the routing/listener boundary itself), not a service call.
4. **Rebuild the CSS** if any class changed, and confirm the new classes are in `static/css/tailwind.css`.
5. **Say what was not verified.** A skipped check, a missing browser, a fake DOM, an unmeasured claim — list
   them rather than letting a passing line imply more than it proves.
6. **Record it**: `done.md` for brief sections, the README's limitation lists for behaviour, and this file
   for whatever moves out of Part 2.
