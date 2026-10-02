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

Dev styling uses the Tailwind Play CDN (see `templates/partials/styles.html`), so no Node step is needed yet.

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
4. Authentication, Student Hub, admin
5. HTMX interactions and polling, ETag caching
6. Security, performance, tests, deployment

## Notes

- Informational pages (about, IQAC, fees, ...) live in the `pages` and `page_sections` tables and are served by path, so a page added to the database works without a restart. Pages with no sections show a short "still preparing" message.
- Queries use `sqlx::query_as` at runtime rather than the compile-time macros, so the project builds without a database.
- The academic tables (students, attendance, marks, exams, timetable) exist but have no Rust models yet; they arrive with the Student Hub in Layer 4.
- The header and footer still carry the college phone number as fixed text. Contact details on the home and contact pages come from `site_settings`.
- Set `SITE_URL` in `.env` so `sitemap.xml` and `robots.txt` use your real domain.
- Images on the home page currently hotlink the live site and move to `uploads/` when the content is migrated.
