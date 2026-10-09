# Fixed Issues — nss-college

Date: 2026-10-09
Commit: local only, not pushed

## What I fixed

1. **CSV credentials endpoint guard clarity**
   - File: `src/routes/admin/people.rs`
   - Before: `import_credentials_csv` took `_user: AdminOnly`, which looked like it was trying to ignore the guard.
   - After: the guard binding is now `user: AdminOnly`, so the route’s auth requirement is explicit. The route still requires an admin session.

2. **`discard_upload` error handling**
   - File: `src/services/content_admin.rs`
   - Before: DB cleanup failures during file discard were silently swallowed inside `if let Ok(...)`.
   - After: all three outcomes (upload row found, upload row missing, DB error) are logged with `tracing::warn!`. The function still does not return errors to the caller, because it is used during deletes/updates where cleanup failure should not roll back the main action.

3. **Student hub “today” timezone**
   - File: `src/routes/hub_pages.rs`
   - Before: the hub used `OffsetDateTime::now_utc()` for the college “today” string and for weekday/is-now/is-past comparisons.
   - After: the hub now derives “today” and weekday from the same college-local definition used by attendance (`Asia/Kolkata`). Time display still comes from the server clock, but the day/weeking logic now matches the attendance world.

4. **Dead code cleanup**
   - Files: `src/routes/admin/people.rs`, `src/services/content_admin.rs`, `src/services/reports.rs`, `src/services/courses.rs`
   - Removed `ReviewForm`, `ContentCounts::counts`, and `sessions_csv`.
   - Silenced remaining obvious dead-code warnings on shared reporting structs with targeted `#[allow(dead_code)]` where the field is plausibly still wanted for UI/debug.

## What I did not change

- I did not alter the promote/enrollment-semester behavior or `auto_apply_fixed`. That is a product-behavior question, not a clear bug from code alone.
- I did not add a DB-backed readiness check or tighten `safe_next`. Those are follow-ups, not emergency fixes.
- I did not push the commit.

## Verification

- Ran `cargo check` after the edits; it compiles with warnings only.
- Committed the changed files plus `audit.md` in a local commit.
