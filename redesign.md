# NSS College System Redesign — Implementation Brief

## 1. Mission

Perform a thorough, codebase-wide redesign of the NSS College management system. This is **not just a dashboard/UI refresh**. Audit the existing application, fix architectural and data-integrity problems, strengthen authorization, improve academic workflows, and modernize the user experience while preserving existing functionality.

Work directly in the repository. Do not stop after producing a plan, implementing a dashboard, or getting a successful build. Continue through implementation and verification until the agreed scope is complete or a genuine blocker prevents progress.

## 2. Existing Codebase: Starting Context

The project is the NSS College website and management system, available at:

- Repository: https://github.com/abhsh3k/nss-college
- Current technology stack: **Rust, Axum, Askama, HTMX, Tailwind CSS, SQLx, PostgreSQL**
- The database currently contains mock/development data; the system is not yet in real production use.
- Existing functionality includes a public college website and role-based management/portal features. The repository may contain functionality for departments, programmes, courses, course offerings and student enrolment/selection, approval requests, timetable management, faculty substitutions, attendance and reports/CSV exports, exam timetables, marks and imports, semester results, notices/news/events/documents, pages and page sections, rank holders, homepage settings, and system configuration.
- This list is a **starting inventory, not a guarantee that every feature is complete or implemented exactly as described**. Inspect the actual repository, routes, templates, database schema, migrations, and tests before making assumptions.
- A previously observed issue: newly created timetable/course records did not appear correctly because faculty/course foreign-key fields were `NULL`, while older records worked. Investigate this and similar integrity bugs rather than applying a UI-only workaround.
- Prior builds have emitted Rust/dependency warnings, including a warning involving `sqlx-postgres v0.7.4`. Audit relevant warnings and dependency health, but do not upgrade dependencies indiscriminately.
- Existing authentication/session/CSRF protections may already exist. Verify the actual implementation; do not assume a security feature is correct merely because a helper or middleware exists.

## 3. Non-Negotiable Technology Constraints

Keep the existing stack:

- Rust
- Axum
- Askama
- HTMX
- Tailwind CSS
- SQLx
- PostgreSQL

Improve architecture within this stack. Add dependencies only when there is a clear, documented technical reason. Do not migrate to a different web framework or undertake a major stack redesign.

Focus on application and database code, plus basic run/setup instructions. Broad deployment engineering, a mandatory Docker/Compose setup, and hosting infrastructure redesign are out of scope. This does **not** relax any required security boundary.

## 4. Required Working Method

1. **Inspect first.** Read the repository structure, `Cargo.toml`, routes, handlers, middleware, templates, SQL queries, migrations, seed data, and tests. Map the existing features and role/permission checks.
2. **Produce a concise audit and implementation plan.** Identify architectural weaknesses, authorization gaps, data-integrity risks, workflow problems, duplicated logic, broken features, and migration needs. State dependencies between implementation phases.
3. **Implement in dependency-aware phases.** Do not stop after the audit or plan. Update application code, database migrations, seed data, templates, and tests as required.
4. **Test continuously.** Add and run unit, database integration, authorization/isolation, workflow, migration, and critical end-to-end tests. Run the relevant suite during implementation and again at the end.
5. **Preserve functionality.** Inventory existing features and keep them by default. Fix bugs and improve implementation/UX. Do not silently delete or disable a feature. If a feature truly needs removal or replacement, document why and provide an agreed alternative before removing it.
6. **Keep scope disciplined.** Fix bugs affecting the redesign, existing features, permissions, workflows, and data integrity. Record unrelated bugs separately in a prioritized TODO list.
7. **Report truthfully.** Never claim a command, test, migration, or workflow passed unless it was actually run and its result inspected. Clearly report untested areas, environment limitations, blockers, and any unfinished work.

## 5. Roles, Authority, and Account Lifecycle

Design an explicit, maintainable authorization model after auditing the current implementation. Do not blindly impose RBAC or ABAC without assessing the actual requirements. Every protected operation must be authorized server-side, with scope derived from trusted database relationships rather than browser-supplied IDs.

### Super Admin

- Has root-level system authority, including managing roles/permissions, emergency recovery, system-wide configuration, and emergency intervention in academic records when necessary.
- Must use a **dedicated management listener bound exclusively to `127.0.0.1`**. The interface must not be directly reachable from another computer, including on the college LAN.
- Enforce this boundary at the server/network/listener layer—not through hidden navigation, frontend checks, a client-side localhost check, or an untrusted forwarded header.
- Keep the public site on its separate public listener so it can continue operating independently.
- Retain authentication, secure sessions, and audit logging for this interface. Verify the actual deployment/proxy arrangement does not expose the private listener.

### IT Admin

- Handles technical administration, account creation/access operations, credential resets, account activation/deactivation, security/technical configuration, backups/operational administration, and CMS management.
- Controls CMS publishing: may create, edit, publish, schedule, unpublish, and archive website content without in-system academic approval.
- Cannot independently grant themselves Super Admin authority or activate privileged academic appointments. Account creation and login access are distinct from academic appointment approval.
- Cannot make academic decisions, alter marks/results outside an explicitly authorized correction process, or approve academic workflows merely because they are an IT Admin.

### Principal / Academic Admin

- Has institution-wide academic authority: academic structure/calendar, cross-department oversight and conflicts, defined approvals, and authorized HOD overrides.
- Does not automatically receive technical administration privileges.
- The Super Admin appoints and activates the Principal appointment, with effective dates and a mandatory audit trail. A Principal cannot approve their own appointment.

### HOD

- One active HOD appointment per department at a time.
- Manages only their own department, within explicit authority and the defined academic workflows.
- May delegate specific responsibilities to faculty for a limited period, but cannot delegate restricted HOD-only powers or bypass academic finalization rules.
- When an HOD appointment ends, delegations they granted must stop granting access. The incoming permanent/acting HOD reviews each delegation and renews, changes, or revokes it; no automatic carryover.

### Acting HOD and HOD appointments

- IT Admin creates the account and initiates the HOD appointment request; the Principal approves the academic appointment. Only then may the scoped appointment become active.
- A vacant department may have a formally appointed acting HOD, appointed by the Principal.
- Acting appointments must be effective-dated, clearly identified, and audited. An acting HOD has the same normal authorized departmental HOD permissions for the appointment's duration.
- Enforce at most one active HOD/acting-HOD appointment per department, including concurrent requests/race conditions.
- When an appointment ends, revoke its permissions and review pending work, approvals, timetable changes, results, and assignments for handover. Preserve the former HOD's identity, authorship, and history.

### Faculty, students, and delegation

- Teachers can access only assigned courses/classes and the related teaching tasks and student information they are authorized to handle. They cannot access unrelated courses/departments or publish finalized results.
- Students can access their own profile, enrolments, courses, timetable, attendance, exam information, published results, notices, and documents. They may submit permitted requests but cannot directly edit official records.
- Student account creation/import is department-led; staff/teacher account creation is IT-led.
- Account existence, login access, active academic appointment, effective permissions, and department/course assignment must be represented and validated distinctly where appropriate.
- Use effective dates and lifecycle states for appointments/transfers. Deactivation or transfer must revoke the old scope without deleting academic history.

## 6. Authorization and Security Requirements

- Audit every route, handler, mutation, and sensitive query. Identify privilege escalation, inconsistent checks, insecure direct object references, cross-department data leaks, and trust in user-supplied identifiers.
- Centralize/reuse authorization logic where appropriate, but ensure each mutation enforces the correct action and scope.
- Test authorization at the backend, including attempts to access or mutate another department's, another teacher's, or another student's data.
- Protect sessions and authentication appropriately. Audit CSRF protection, login throttling/lockout, session revocation, cookie settings, and reauthentication for critical operations. Keep existing sound protections and correct gaps.
- Audit and log sensitive operations, including role/appointment changes, account deactivation, academic finalization, result corrections, CMS publishing, configuration changes, backup/restore operations, and emergency interventions.
- Audit records should be append-only for ordinary administrators. Never log passwords, session cookies, CSRF tokens, or other authentication secrets.
- Ensure unpublished CMS content cannot leak through direct URLs, stale caches, or alternate routes.
- Backups must be protected from public access and ordinary-admin compromise. Include relevant uploaded files/recovery assets, retention, restore instructions, and restore verification where supported by the existing application scope.
- Do not mandate MFA as a new requirement unless the audit establishes a need and it is agreed separately.

## 7. Academic Data and Workflow Requirements

### Workflow states and approvals

- Build reusable approval/task infrastructure where it reduces duplication, with workflow-specific rules and permitted transitions. Avoid building an unnecessarily generic configurable workflow engine.
- Important academic records must have explicit states and authorized transitions, for example: draft → reviewed → finalized → published, with correction/supersession states where appropriate.
- Finalized records must not be silently overwritten. Corrections require the correct permission, a reason, an audit trail, and preservation of history.
- In-app notifications and actionable task lists should support approvals, pending requests, deadlines, failed imports, and handovers. Notifications never grant access; opening an item must re-check authorization.

### Courses, enrolment, and timetable

- Give courses stable internal identities. Allow course codes to vary by batch/curriculum with explicit mapping/equivalence where confirmed.
- Offerings, enrolments, and results must reference the correct applicable course definition/version. If two course codes represent genuinely different courses, use distinct identities; do not infer equivalence from names alone.
- Syllabus redesign is out of scope; the requirement is to handle code/course identity differences correctly.
- Validate required faculty/course/department/semester relationships before writing records. Add suitable foreign keys, uniqueness constraints, indexes, and transaction boundaries. Never silently create incomplete timetable/course records.
- Audit and fix the observed timetable/course issue involving newly created records with `NULL` faculty/course IDs, and search for related failures in forms, handlers, database writes, and selection queries.

### Results and exams

- Preserve internal assessment results and university results separately. University results take precedence in the relevant official student-facing views, while internal history remains available to authorized staff.
- Never automatically delete prior-semester results or internal assessments when a student advances or university results arrive.
- Support one to three internal exams (or a configurable number of assessment records) without requiring a schema change for every exam count.
- Model whether internal assessments contribute to an official university result explicitly.
- Support authorized manual university-result entry and validated CSV/Excel bulk import. Import must provide a preview, relationship validation, duplicate detection, explicit course-code mapping, clear row-level errors, and confirmation before changes are committed.
- Do not match students by name alone or silently overwrite finalized results. Corrections must be authorized and audited.
- Enforce the intended lifecycle and visibility rules so students see results only when they are published/visible to them.

### Data and migrations

- The existing database contains mock data, so the schema can be redesigned properly where necessary.
- Use **versioned SQLx migrations** for schema changes. Update seed/mock data and all dependent application code and tests.
- Do not treat the fact that current data is mock data as permission to use unsafe, undocumented destructive migration practices. Never assume future/production data can be discarded.
- Preserve historical academic records and audit history. Do not cascade-delete attendance, marks, results, appointments, or audit records when deactivating accounts or changing assignments.

## 8. User Interface and Existing Features

- Create separate role-specific workspaces/dashboards and navigation for the roles that need them. Prioritize pending tasks, alerts, useful statistics, and relevant actions instead of overcrowded CRUD-button dashboards.
- Build reusable components/patterns for tables, forms, search/filter controls, approval panels, status indicators, and validation/error feedback.
- Use a clean, modern, responsive design with restrained colors, consistent spacing, accessible contrast, and useful loading, empty, and error states.
- Use HTMX for fast interactions and avoid unnecessary full-page reloads, without compromising authorization or data integrity.
- The public college website may have a distinct editorial design from the administrative workspaces.
- IT Admin controls all CMS publishing. Audit content types and publishing behavior in the actual codebase; preserve existing content-management capabilities.
- Preserve every existing feature by default, including features discovered during the repository inventory even if they are not listed in this brief. Fix bugs and improve architecture rather than removing features.

## 9. Testing and Acceptance Criteria

Comprehensive automated testing is required. Add or improve tests at the appropriate levels:

- Unit tests for domain logic, validation, state transitions, and permission decisions.
- Database integration tests for SQL queries, constraints, transactions, and migrations.
- Authorization tests for each role and protected action, including cross-department/cross-user isolation and privilege-escalation attempts.
- Workflow tests for appointments, acting HOD succession, delegation handover, academic approvals, timetable operations, attendance, result imports/corrections/finalization/publication, and CMS publishing.
- Regression tests for existing features and discovered bugs.
- Critical end-to-end tests for the most important role journeys, using the project's practical test infrastructure.
- Migration tests from the available development/mock schema and seed data. Document assumptions where a realistic previous-version database is unavailable.

Before declaring completion:

1. The application compiles; relevant warnings are reviewed and explained or fixed when appropriate.
2. All migrations and seed/mock data changes are applied and verified in the test environment.
3. The full relevant automated test suite has actually run; failures are fixed or explicitly documented with evidence and reasons.
4. Critical authorization boundaries and workflows are verified, not merely inferred from the UI.
5. Existing features have been regression-tested.
6. Basic run/setup instructions and concise architecture/permission/workflow notes are updated.
7. A final report lists changes made, migrations, commands and tests actually run with results, any limitations, and unrelated bugs recorded for later.

A build alone is not sufficient evidence of correctness. Never claim success for tests or manual verification that did not happen.

## 10. Suggested Implementation Phases

Adapt the order after the initial audit, but preserve the dependency-aware approach:

1. **Repository and feature inventory:** map modules, routes, templates, schema, migrations, seed data, authorization, and current tests.
2. **Audit and design:** document key defects; propose role/permission model, appointment/delegation model, academic state transitions, and database redesign.
3. **Database foundations:** implement versioned migrations, relationships, constraints, indexes, and updated seed data.
4. **Authorization foundations:** establish reliable server-side authorization and scope checks; test isolation and escalation cases.
5. **Appointments and lifecycle:** implement role/account separation, HOD/acting-HOD appointments, effective dates, delegation, revocation, and handover.
6. **Academic workflows:** fix and verify courses, enrolments, timetable, attendance, exams, results, imports, approvals, and historical record handling.
7. **CMS and existing features:** preserve and improve website content management and all other inventoried functionality.
8. **Role-based UI:** improve dashboards, navigation, forms, tables, notifications, responsive behavior, and error states.
9. **Verification and documentation:** run migrations and full relevant tests, regression-test features, review security boundaries, and report evidence.

Do not force this exact order if repository dependencies justify a safer one; document significant sequencing changes.

## 11. Final Instructions to the Coding Agent

- Treat this document as the agreed scope and constraints; inspect the actual repository before implementing.
- Do not ask the user to restate decisions already specified here. Ask concise questions only when a genuinely blocking ambiguity cannot be safely resolved from the codebase.
- Make real code changes and test them. Do not return only advice, pseudocode, or a plan.
- Keep a short, current TODO/checklist and update it as work progresses.
- Preserve all existing features by default. Do not perform silent destructive operations or silently weaken security.
- If blocked, finish all independent safe work, explain the exact blocker and evidence, and state the next action needed.
- At the end, report **what changed, why, migrations, tests/commands actually run and their outcomes, remaining limitations, and unrelated TODOs**.
