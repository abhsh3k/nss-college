# What is done, and what is left

A status map of `redesign.md` against the code as it actually is. Every "done" line below was verified
either by a test that runs or by a command whose output was inspected; the evidence is named so it can be
re-checked. Where something is partly built, both halves are stated.

Legend: **Done** · **Partial** (schema or part of the flow exists, the feature does not) · **Not started**

Test commands referenced throughout:

    docker compose up -d
    DATABASE_URL=postgres://nss:nss@localhost:5432/nss_college cargo test
    # 57 unit + 16 database + 8 end-to-end, currently all passing

---

## 1. Scoreboard

| `redesign.md` section | Status |
|---|---|
| §3 Stack constraints (Rust/Axum/Askama/HTMX/Tailwind/SQLx/PostgreSQL) | Done — unchanged |
| §5 Super Admin (loopback-only listener) | Done |
| §5 IT Admin | Done |
| §5 Principal / Academic Admin | **Not started** |
| §5 HOD, one per department, effective dates | Done |
| §5 Acting HOD | Done |
| §5 Delegation + non-carryover handover | Done |
| §5 Faculty / student scoping and account lifecycle | Done |
| §6 Route-by-route authorization audit | Done (every handler takes a role/scope extractor) / Partial (no per-route test matrix — see §3.B.18) |
| §6 Upstream login throttle, CSRF, session revocation, cookies | Done |
| §6 Re-authentication for critical operations | **Not started** |
| §6 Append-only, complete audit log | Done (append-only) / Partial (no before/after or reason on every row) |
| §6 Unpublished CMS content cannot leak via direct URL | Done |
| §6 Backups protected, with retention and restore instructions | Done |
| §7 Appointment/approval lifecycle | Done |
| §7 Record states + no silent overwrite of finalized | Done / Partial (no correction path) |
| §7 In-app notifications and task lists | **Not started** (work queue only) |
| §7 Stable course identity and versions | Done |
| §7 Timetable/course relationship validation | Partial |
| §7 Internal vs university results kept separately | Done |
| §7 1–3 internal assessments without a schema change | Done |
| §7 CSV/Excel import with preview, validation, confirmation | Done for CSV, **not started** for Excel |
| §7 Correction of finalized results | **Not started** |
| §7 Versioned migrations, history preserved | Done |
| §8 Role-specific workspaces, HTMX, responsive UI | Done / Partial (reusable components incomplete) |
| §9 Automated test suite (unit / DB / authz / workflow / e2e / migration) | Done for what exists; gaps listed in §3.B.18–19 |
| §8, §9 README + architecture/permission notes | Done |
| §8, §6 Scheduled/timed CMS publishing | **Not started** |

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
- **CLI**: `create-user`, `set-password`, `seed-demo-users [--credentials-file]`, `backup [--list]`,
  `maintenance`.
- Demo passwords never reach the log: the seeder returns them and they go to a `0600` file or are
  discarded; the start-up path logs only the account count.
- `/healthz` returns a plain 200 liveness response without touching the database or exposing details;
  `robots.txt` disallows the private areas. There is no deeper readiness probe or migration report.

### 2.6 Architecture, tests, docs

- `main.rs` is only process setup and the listeners; the application is `src/lib.rs`, which is what the
  tests build against. `apply_layers`, `connect`, `migrator` and `init_session_store` are library
  functions.
- **Suite**: 57 unit, 16 database (`#[sqlx::test]`, a freshly migrated database per test), 8 end-to-end
  (`tower::oneshot` through the real layered router). Currently all passing; the only build warning is the
  pre-existing `sqlx-postgres v0.7.4` future-incompatibility notice.
- README documents the two listeners, roles, backups/restore, security headers, test commands, the layout
  and the migrations; `fixes.md` §13–§15 records each pass with its live verification.

---

## 3. What is left

### A. Not started — no implementation exists

1. **Principal / Academic Admin role** (brief §5). There is no `Principal` in `Role`, no route, and no
   approver for HOD appointments: the IT admin appoints directly, and there is no self-approval rule to
   enforce because there is no separate approver. `role_assignments.role` accepts `'principal'` in its
   CHECK, so the schema half is in place.
2. **Super Admin as a distinct authority from IT Admin.** The listener boundary exists, but there is no
   `super_admin` handling in code — no role-management screen, no emergency-recovery or
   emergency-intervention flow, and no "last recovery account cannot be deactivated" guard.
3. **Result correction workflow** (brief §7, P1.1). Finalized marks are *blocked* with "use the correction
   workflow" and `mark_history` exists, but there is no correction route or UI: nothing can write a
   correct-with-reason, nothing writes `mark_history`, and there is no approval step or supersession view.
   This is the largest single functional gap, because the brief calls it out explicitly.
4. **Notifications** (brief §7, §8). No table and no feed. `/admin/work-queue` is a task list, but there are
   no actionable notifications for approvals, deadlines, failed imports or handovers.
5. **Scheduled CMS publishing** (brief §7, §8). `clean_status` has only `draft`/`published`/`archived`; there
   is no scheduled state, no start/end times, and no single timezone-aware visibility predicate.
6. **Excel result import.** `calamine` is used only for the student import; results accept CSV only.
7. **Re-authentication for critical operations** (brief §6): role/appointment changes, result
   finalization/correction, backup/restore and emergency actions do not ask for the password again.
8. **Reserved route paths for CMS pages.** A page path is normalized and must start with `/`, but nothing
   stops an admin creating a page that shadows `/login`, `/admin`, `/hub`, `/static` or `/uploads`.

### B. Partial — the data or half the flow exists, the feature does not

9. **Single applicability rule for exams/timetable** (P0.12). Exam creation checks `may_manage_programme`
   (department scope) but does not verify that the submitted course belongs to the selected
   programme/semester, and there is no shared `applicable_course` helper across exam, marks, import and
   timetable writes.
10. **Timetable/course relationship enforcement** (P0.10). Forms no longer treat a bad id as "unassigned",
    but a published period still does not require a validated teacher, and there is no explicit
    `teacher_tbd` state with a work-queue item.
11. **Capacity policy** (P0.11). Student confirmation is now race-safe (`SELECT … FOR UPDATE` with a
    fresh enrolment count). HOD assignment and cohort finalization still do not enforce capacity.
12. **Offerings in the student results view** (P1.3). Results are filed per course, and the results query
    filters on `courses.semester`, which is `NULL` for catalogue courses — so a confirmed offering appears
    in "Enrolled courses" but not in `/hub/results`. The separate internal/university storage is correct;
    the read path is not.
13. **Offerings in substitution candidates** (P1.4). `attendance::teacher_day` joins `programmes`, so an
    offering period never appears in the substitute-teacher candidate list.
14. **Clash detection between two offerings** (P1.4). Programme-vs-offering is covered; two different
    offerings targeting the same programme can still be scheduled on top of each other.
15. **Attendance report filters** (P1.4). `by_session` filters on the course's programme while
    `totals`/`by_student` filter on the student's, so the session table can be empty while the totals are
    not; the course dropdown hides catalogue courses under a programme filter.
16. **Audit detail** (P0.7). Rows carry actor/action/entity/entity_id, and the log is append-only by
    trigger, but most rows have no before/after summary, reason or correlation id, and some audit calls
    still discard their error with `let _ =`.
17. **Reusable UI components** (brief §8). Tables, empty states and form errors are consistent by
    convention, not by shared components; there is no pending-approval badge in the navigation.
18. **Authentication test depth** (brief §9). There is no test that signs in as each role and attempts
    cross-department or cross-user requests through the router; authorization is covered by unit tests on
    the decision functions plus the service-level checks.
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

1. **Result correction workflow** (schema is already there). Unblocks the "finalized records cannot be
   silently overwritten *or* corrected" half of the brief and gives `mark_history` a writer.
2. **Offerings in the student results view** (§3.B.12) — one read path fix that makes the whole
   offering/results feature coherent.
3. **Principal role and the HOD appointment approval** — the last missing authority in the brief.
4. **Scheduled CMS publishing** plus reserved-path validation — self-contained, touches content only.
5. **Notifications/task list** — builds on the work queue that already exists.
6. **Exam/timetable applicability helper** (§3.B.9) and **capacity everywhere** (§3.B.11).
7. **Remaining P1.4 defects** (substitutions, offering-vs-offering clashes, report filters).
8. **Super Admin distinct powers, re-authentication, audit detail, `.xlsx` results, authz test matrix.**
