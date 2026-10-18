# Codebase Audit — NSS College (website, Student Hub & admin)

**Date:** 18 October 2026
**Audited ref:** branch `improv/adminhoddash` (working tree clean at audit time)
**Stack:** Rust 2021 · Axum 0.7 · Askama 0.12 · HTMX 1.9.12 · PostgreSQL 16 / SQLx 0.7 · tower-sessions · Tailwind (compiled)
**Scope:** whole repository — `src/` (~20k lines Rust), `templates/` (102 files), `migrations/` (38), `static/`, `tests/`, config/deploy files, git history, and on-disk workspace files.

**Method:** static review of every module in `src/`, automated cross-checks (CSRF coverage over all POST handlers, dynamic-SQL composition, `|safe` usage, JS sinks, git-history secret scan), plus a full build and test run against a live PostgreSQL. This is a single-pass review, not a dynamic penetration test: no live traffic was replayed, and `cargo audit`/`cargo deny` are not installed in this environment, so dependency CVEs were checked by version only.

---

## 1. Verdict

**The codebase is in good health and materially better than average for its size.** The security fundamentals are not just present, they are *systematically* applied: Argon2 hashing, session-id cycling at sign-in, per-request authorization extractors, CSRF on every state-changing endpoint, a strict `script-src 'self'` CSP with no inline handlers, server-generated upload filenames, and an append-only audit log with DB triggers. The whole test suite passes:

| Suite | Result | Time |
| --- | --- | --- |
| Unit (`cargo test --lib`) | **86 passed, 0 failed** | 0.03 s |
| End-to-end router (`tests/app.rs`) | **38 passed, 0 failed** | 106.5 s |
| Database/migrations (`tests/database.rs`) | **40 passed, 0 failed** | 93.6 s |
| JS hooks (`tests/js_hooks.rs`) | **1 passed, 0 failed** | 0.6 s |
| **Total** | **165 passed, 0 failed** (`EXIT=0`) | — |

`cargo check --all-targets` is clean; the only compiler output is the upstream
`sqlx-postgres v0.7.4` future-incompatibility notice.

Nothing found in this audit is an emergency. The two **High** items are about
*credentials lying on disk / weak dev DB exposure*, not about exploitable
application code. The Medium/Low items are hardening and hygiene.

---

## 2. What is done well (verified, not assumed)

**Authentication & sessions**
- Argon2id defaults (`auth/password.rs`), hashed on `spawn_blocking`; a *dummy hash* burns equal CPU when the account does not exist, so login timing does not enumerate accounts.
- `session.cycle_id()` on successful sign-in (fixation) and again on password change; CSRF token rotated at login.
- `session_version` on every session — password reset, deactivation or role change revokes live sessions (`a_role_change_revokes_the_accounts_live_sessions`).
- Lockout: 5 wrong passwords → 15 min (`users::record_failed_login`); plus an in-memory sliding-window limiter keyed on **both** account and client IP (unit-tested: `the_window_slides`, `one_address_cannot_be_used_to_grind_many_accounts`).
- Role-scoped post-login redirects (`safe_next_for_role`, tested) — no open redirect, no cross-workspace bounce.
- Re-authentication window (`REAUTH_WINDOW_SECS`, future timestamps rejected — tested) gates critical writes (roles, mark corrections).

**Authorization**
- Extractor-based guards (`AdminOnly`, `AcademicOnly`, `Manager`, `Scheduler`, `SuperAdminOnly`, …) applied per request; the HOD scope is read per request from the appointment model, and a missing department **denies** rather than meaning “all”.
- Root authority is bounded twice: `Config::from_env()` panics if `MANAGEMENT_HOST` is not loopback, and `management_area_on_public` refuses `super_admin`/`faculty` on the public listener — network boundary independent of navigation.
- IDOR coverage is tested end-to-end (`one_student_cannot_withdraw_another_students_selection`, `a_teacher_cannot_mark_attendance_for_another_teachers_class`, …).

**CSRF — audited exhaustively**
I extracted every handler registered with `.post(...)` (87) and checked each body for `csrf::verify`. **All state-changing handlers verify**, either directly or through a shared gateway (`decide()` in marks, `guard()` in super_admin). Tokens are session-bound, compared constant-time, and carried by hidden inputs or `hx-vals` on every HTMX POST (checked in templates).

**SQL / injection**
No user-controlled string ever reaches SQL text. All dynamic SQL composes *constant* fragments (`format!("… {}", visible("p"))`, `COLUMNS`, `TODAY`) — values are always `$n`-bound. I specifically searched for `format!("SELECT|INSERT|UPDATE|DELETE` fed by request data: the four hits are all internal constants.

**Uploads** (`src/uploads.rs`, `routes/public.rs::upload`)
- Path never derived from the client filename: extension comes from a server-side MIME allowlist table, the stored name is `<unix>_<16-hex>.<ext>` from `OsRng`.
- Body limits enforced at three layers (route `DefaultBodyLimit`, `MAX_BODY_BYTES` 10 MB, `MAX_FILE_BYTES` 8 MB).
- `/uploads/*path` rejects `.`/`..` segments and serves through a single authorization query that joins audience + publication-window rules — drafts/expired assets never leak their bytes, and study materials are gated on enrollment/teaching.

**Browser hardening**
- CSP: `script-src 'self'`, `object-src 'none'`, `frame-ancestors 'none'`, `form-action 'self'`; HTMX is self-hosted; every inline `on*` was replaced by `data-*` hooks (`static/js/app.js` has **zero** `innerHTML`/`insertAdjacentHTML`/`eval` sinks — only htmx.min.js contains such strings internally).
- `X-Content-Type-Options`, `X-Frame-Options: DENY`, `Referrer-Policy`, `Permissions-Policy` on every response; HSTS only when `COOKIE_SECURE=true`.
- Admin workspaces served `Cache-Control: no-store` (overriding), signed-in pages too.

**Database discipline**
38 sequential migrations; CHECK constraints on roles/statuses, unique partial indexes (`one_active_principal`, `one_pending_principal`), `audit_log` append-only via trigger (`the_audit_log_is_append_only`), per-request correlation ids (migration 0033). Constraints are asserted by `#[sqlx::test]` cases, not just documented.

**Ops**
Recurring backup with manifest + SHA-256, `0700` run dirs, refusal to write backups inside `static/` or `uploads/`; restore deliberately manual. Credentials seeding never writes passwords to logs.

---

## 3. Findings

Severity key: **H** high · **M** medium · **L** low · **I** info

### H1 — Production credentials stored in plaintext in the workspace (`username.txt`)

*Severity: High (credential exposure). Location: `username.txt` (working tree), `.gitignore`.*

The file contains a live-looking credential table headed
`PRODUCTION - https://nss-college-production.up.railway.app`, e.g. the seeded
production `admin@college.local` password in cleartext, plus staff/faculty/HOD
logins. Good news: `git log --all -- username.txt` is **empty** — it was never
committed, and `.gitignore` explicitly covers it. The risk is everything else:
any workspace copy, editor backup, sync tool, screen share or zip handed to
someone leaks root-adjacent production access, and the passwords are short
machine-generated words with no rotation story.

**Do:** rotate every password listed in that file *now*, move them to a
password manager, delete the file (or keep it outside the repo at `0600`), and
unset `SEED_DEMO_USERS` once seeding is done (README already says this — the
env var is the durable fix; the file should not be).

### H2 — PostgreSQL published on all interfaces with weak fixed credentials

*Severity: High when the host is not a dev laptop. Location: `docker-compose.yml`.*

```yaml
ports:
  - "5432:5432"     # 0.0.0.0 — reachable from the network
environment:
  POSTGRES_PASSWORD: nss
```

The compose file is the documented default path (`docker compose up -d`) and
binds the database to every interface with password `nss`. On a server or a
shared network this is an open database.

**Do:** bind explicitly to loopback (`"127.0.0.1:5432:5432"`), take the
password from an env file/secret, and never run this compose file anywhere a
network can reach it.

### M1 — No throttling on `/account/reauth` (lockout bypass)

*Severity: Medium. Location: `src/routes/auth.rs::reauth_submit`, `password_submit`.*

Login attempts go through the limiter **and** the 5-strike lockout. The
password *re-entry* endpoint does neither: `reauth_submit` calls
`password::verify_blocking` directly and never touches `login_limiter` or
`record_failed_login`; `password_submit` behaves the same for
`current_password`. An attacker who has obtained a valid session (XSS elsewhere,
a borrowed machine) can therefore guess the account password without limit and
without ever tripping the lockout the README advertises.

**Do:** run re-auth through the same limiter + `record_failed_login` (or count
failures per session and flush the session after 5).

### M2 — Backup passes the database password on the command line

*Severity: Medium on multi-user hosts. Location: `src/services/backup.rs:213`.*

`pg_dump … --arg(&cfg.database_url)` puts `postgres://nss:…@…` into argv, so
the password is visible in the process table for the duration of every backup
(and shell history is not involved only because args are used, not a shell —
that part is correct).

**Do:** export `PGPASSWORD` (or point `pg_dump` at a `0600` `.pgpass`) instead
of passing the URL as an argument.

### M3 — Audit writes are best-effort and silently droppable

*Severity: Medium (integrity of the audit trail). Location: 23 × `let _ = …`
across `routes/admin/people.rs`, `cli.rs`, etc.*

The audit log is append-only and carries correlation ids — but many call sites
discard its error (`let _ = users::audit(…)`), so a write failure means the
action still happens and only the *record* of it disappears, with no log line.
For a table whose whole purpose is accountability that is the one place an
error should never be swallowed.

**Do:** `if let Err(e) = … { tracing::error!(… "audit write failed") }` at
minimum; consider failing the request for the highest-privilege actions
(role change, password reset) — README §Known limitations already flags this
as partial.

### M4 — No CI: 165 tests exist but nothing runs them automatically

*Severity: Medium (process). Location: no `.github/`, no pipeline config.*

Branches (`railway/fix-deploy-*`, `gemini`, `improv/adminhoddash`) are pushed
without any gate. The suite is strong precisely because someone remembers to
run it. Same for dependency CVE scanning: `cargo audit` is not installed and
no workflow calls it.

**Do:** one workflow — `cargo check --all-targets` + `cargo test` (compose
service for Postgres) + `cargo audit` on push/PR.

### M5 — Whole-file-in-memory request handling

*Severity: Medium (resource). Location: `src/uploads.rs::read`,
`routes/public.rs::upload`.*

Every multipart upload is buffered fully in RAM (10 MB cap *per request*), and
every `/uploads/…` response is `tokio::fs::read` into a `Vec<u8>` (8 MB cap)
rather than streamed — no `Content-Length` pre-check, no Range support, no
backpressure beyond the 10-connection pool. Individually bounded; multiplied by
concurrent visitors it is a cheap way to grow RSS.

**Do:** stream responses (`tokio_util::io::ReaderStream` + `CONTENT_LENGTH`),
and reject bodies whose declared `Content-Length` exceeds the cap before
reading.

### L1 — Stale doc comment in `src/uploads.rs`

The module header still says “`/uploads` is already served by `ServeDir` in
`main.rs`”. It isn’t: an authorization-aware handler (`routes/public.rs::upload`)
serves it — `main.rs` explicitly comments that user uploads are *not* raw-mounted.
Docs that describe a stricter design than the code actually has are the ones
that cause the next regression.

### L2 — Page paths are barely validated and flow unescaped into `sitemap.xml`

*Location: `routes/admin/pages.rs::read_form`, `src/seo.rs`.*

Validation only requires “starts with `/`, not `/`, no spaces, not reserved”.
A path like `/<x>&evil` is accepted, rendered safely in HTML (Askama escapes)
but interpolated raw into `<loc>` in `sitemap.xml` — XML injection, gated on
admin rights, so Low. **Do:** restrict paths to `[a-z0-9/_-]` (or XML-escape in
`seo.rs` — one line either way).

### L3 — Ambiguous login lookup

*Location: `services/users.rs::find_for_login`.*

`(non-student email = X) OR (student admission_no = X) LIMIT 1` has no
`ORDER BY`, so an identifier that satisfies both halves resolves arbitrarily.
Practically unreachable, but the fix is one `ORDER BY u.id` (or two queries).

### L4 — Lockout/limiter lifecycle quirks (accepted risks, documented)

- The limiter is in-memory: a restart clears counters (`state.rs` explains the threat model — fine, but worth knowing).
- `record_failed_login` resets `failed_logins` to 0 when it locks, so every 15-minute lockout hands an attacker a fresh set of 5 tries. Rate is still bounded (≈20/hour/account); consider an exponential backoff if lockouts become common.

### L5 — File-upload errors show a bare 400 page

*Location: `AppError::BadRequest(msg)` in `error.rs`.*

The message is logged (`tracing::info!`) but the user only sees the generic
bad-request template — so a teacher who uploads a `.pptx` gets a blank 400
instead of “Use a JPG, PNG, GIF or WebP”. Several forms flash their own
messages, but the `uploads::save`/`read` paths don’t. **Do:** render the message
in `BadRequestTemplate`.

### L6 — Dependency currency

| Crate | Locked | Current line | Note |
| --- | --- | --- | --- |
| axum | 0.7.9 | 0.8.x | upgrade is a real migration (extractor signatures) |
| sqlx | 0.7.4 | 0.8.x | source of the future-incompat warning on current rustc |
| askama | 0.12.1 | 0.12+ (macros in newer) | no `{% macro %}` — known, documented in `remaining.md` item 14 |
| htmx | 1.9.12 (vendored) | 2.x | vendored on purpose for CSP; audit on bump |

Nothing here is urgent; `sqlx-postgres 0.7.4` is the only one a future toolchain
will refuse.

### L7 — Repository hygiene

- Commit messages on the current branch (`DOne brhh`, `hkhk`, `still the changes`, `DASHREWORK`) give no review trail.
- `server.log`/`server.err` were committed twice in history (4891805, 3e3fded). I inspected their contents: startup lines only, **no secrets** — but logs in history are permanent; `.gitignore` now covers them.
- Docs sprawl: `README.md` (~500 lines) + `remaining.md` + `done.md` + `fixes.md` + `redesign.md` overlap heavily; several statements must be maintained in 2–3 places (README already duplicates `remaining.md` “as the implementation view”).
- `tailwind.css` is committed deliberately (documented, correct for the deploy image) — but it means a template change without `build-css` ships unstyled; there is no check that they match.

### I1 — Accepted-by-design notes (no action required)

- CSP keeps `style-src 'unsafe-inline'` (homepage carousel custom property, attendance bar widths) — documented residual risk; fine while HTML stays escaped.
- Management and public ports **share a cookie** (cookies scope to host, not port): the loopback *bind* is the boundary, exactly as the README states.
- Login limiter is shared across both listeners by design.
- `.env` in the working tree contains only local dev values (no production secrets).

---

## 4. Prioritised action list

1. **Rotate the production credentials in `username.txt` and delete the file** (H1).
2. **Bind compose Postgres to `127.0.0.1` and change the password** (H2).
3. **Rate-limit `/account/reauth`** and count its failures (M1).
4. **Add a CI workflow** running `cargo check --all-targets`, `cargo test` (with Postgres) and `cargo audit` (M4).
5. Stop discarding audit-write errors; log them loudly (M3).
6. Keep the DB password out of `pg_dump` argv (M2); stream upload responses (M5).
7. Quick wins: fix the `uploads.rs` doc comment, restrict page-path characters, XML-escape `sitemap.rs`, surface `BadRequest` messages, tidy commit messages/docs (L1–L7).

---

## 5. Verification log

```
cargo check --all-targets
  → Finished `dev` profile; only: warning: sqlx-postgres v0.7.4 future-incompat

DATABASE_URL=postgres://nss:nss@localhost:5432/nss_college cargo test
  → 86 unit + 38 e2e + 40 database + 1 js = 165 passed, 0 failed, EXIT=0

git ls-files | grep -Ei '\.env|username|credential|secret|\.log'
  → only `.env.example` and `templates/admin/credentials.html` tracked
git log --all --oneline -- username.txt   → (empty: never committed)
git show <4891805|3e3fded>:server.log     → startup lines only, 0 secret matches
```

**Audit limitations:** single-pass static review of ~20k LOC + 102 templates;
no dynamic/penetration testing; dependency CVEs not machine-scanned
(`cargo audit` unavailable); git history reviewed only for the secret patterns
above.
