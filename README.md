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

After pulling this update, rebuild the CSS (new classes were added) and restart:

    powershell -ExecutionPolicy Bypass -File .\build-css.ps1
    cargo run

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
    migrations/     0001 core, 0002 academics, 0003 content, 0004 triggers, 0005 starter content

## Layers

1. Design system and templates (done)
2. Routing structure for all public pages (done: pages registry, programme/department/news/notice routes, sitemap.xml, robots.txt)
3. PostgreSQL schema, migrations, SQLx services (done)
4. Authentication, Student Hub, teacher tools, admin
   - 4a (done): sign-in, sessions, CSRF, roles, dashboard shells, `create-user` command
   - 4b-1 (done): admin tools for people, programmes/courses, enrollment and the timetable builder
   - 4b-2: publishing tools (notices, news, events, documents, pages, settings) for IT admin and office staff
   - 4c: teacher attendance
   - 4d (done): student views — the Student Hub at `/hub` shows the announcements feed, today's
     timeline, weekly timetable matrix, enrolled courses with live attendance percentages, and the
     e-grants warning card when the monthly attendance drops below the configured threshold
5. HTMX interactions and polling, ETag caching
6. Security, performance, tests, deployment

## Notes

- Informational pages (about, IQAC, fees, ...) live in the `pages` and `page_sections` tables and are served by path, so a page added to the database works without a restart. Pages with no sections show a short "still preparing" message.
- Queries use `sqlx::query_as` at runtime rather than the compile-time macros, so the project builds without a database.
- Students, the timetable and attendance now have models and queries in `services/hub.rs` (Student Hub).
  The marks and exam tables still have no Rust models yet; they arrive with the Student Hub's exam views.
- The header and footer still carry the college phone number as fixed text. Contact details on the home and contact pages come from `site_settings`.
- Set `SITE_URL` in `.env` so `sitemap.xml` and `robots.txt` use your real domain.
- Images on the home page currently hotlink the live site and move to `uploads/` when the content is migrated.
