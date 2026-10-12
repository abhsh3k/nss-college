//! The management area: IT-administrator tools, and the department tools that
//! a head of department also uses.
//!
//! The routes come in two sets:
//!
//! * [`routes`] — the technical administration pages (people, publishing,
//!   settings, appointments, root authority). They are absolute `/admin/...`
//!   paths, and their handlers take `AdminOnly`/`OfficeOrAdmin`/
//!   `SuperAdminOnly`. They are served on both listeners for the IT
//!   administrator, the Principal and office staff; the public mount is wrapped
//!   by [`crate::auth::management_area_on_public`], which keeps root authority
//!   loopback-only.
//! * [`manager_routes`] — the department pages, whose handlers take `Manager`
//!   (plus the per-user notification feed and the section index). They are
//!   written with *relative* paths and mounted twice: at `/admin` (where the IT
//!   administrator and the Principal work) and at `/hod`. The `/hod` mount is
//!   what makes the head of department's tools work: an HOD is a teacher, they
//!   sign in on the *public* listener, and the `/admin` form is refused there.
//! * [`content_routes`] — notices, news and events, also relative, mounted at
//!   `/admin` for the IT administrator and at `/staff` (see [`staff_routes`])
//!   for office staff.
//!
//! Mounting the same handlers under a second prefix grants nothing: the guards
//! run per request and are identical, so a caller who is not a manager is
//! refused at `/hod` exactly as they are at `/admin`.

mod academics;
mod appointments;
mod attendance;
mod course_offerings;
mod departments;
mod documents;
mod exams;
mod events;
mod manage;
mod marks;
mod materials;
mod news;
mod notices;
mod notifications;
mod pages;
mod people;
mod rank_holders;
mod results_import;
mod settings;
mod substitutions;
mod super_admin;
mod timetable;
mod work_queue;

use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Router,
};

use crate::{sections, state::AppState};

/// Ceiling on a content form's whole body, a little above the per-file limit in
/// `crate::uploads` so a big attachment still has room for the text fields.
const CONTENT_BODY_LIMIT: usize = 10 * 1024 * 1024;

/// The department (manager-scoped) pages, with relative paths so the set can be
/// mounted in more than one workspace. See the module docs.
pub fn manager_routes() -> Router<AppState> {
    // The head of department's landing page: every queue in one list, with
    // links back to the pages that already handle each kind of decision.
    let work_queue = Router::new().route("/work-queue", get(work_queue::page));

    // The management-area index a dashboard card opens. It only lists pages
    // that already exist behind their own guards, so it is `Ready` + a
    // catalogue lookup — nothing is granted here.
    let areas = Router::new().route("/manage/:key", get(manage::page));

    // The in-app notification feed, readable by any signed-in user for their
    // own rows. An HOD needs it on the listener they sign in to.
    let notifications = Router::new()
        .route("/notifications", get(notifications::page))
        .route("/notifications/:id/read", post(notifications::mark_read))
        .route("/notifications/read-all", post(notifications::mark_all));

    // Course offerings and student selection (HOD tools).
    let course_offerings = Router::new()
        .route("/courses", get(course_offerings::catalogue))
        .route("/courses/add", post(course_offerings::add_course))
        .route(
            "/courses/:id/edit",
            get(course_offerings::edit_form).post(course_offerings::edit_course),
        )
        .route("/courses/:id/toggle", post(course_offerings::toggle_course))
        .route("/courses/offerings", get(course_offerings::offerings).post(course_offerings::create_offering))
        .route("/courses/offerings/:id", get(course_offerings::offering_page).post(course_offerings::edit_offering))
        .route("/courses/offerings/:id/delete", post(course_offerings::delete_offering))
        .route("/courses/offerings/:id/targets", post(course_offerings::set_targets))
        .route("/courses/offerings/:id/status", post(course_offerings::publish))
        .route("/courses/offerings/:id/periods", post(course_offerings::add_period))
        .route("/courses/offerings/:id/periods/:entry_id/delete", post(course_offerings::delete_period))
        .route("/courses/external", get(course_offerings::external))
        .route("/courses/external/:id/decide", post(course_offerings::decide))
        .route("/courses/selections", get(course_offerings::selections))
        .route("/courses/selections/assign", post(course_offerings::assign))
        .route("/courses/selections/:selection_id/state", post(course_offerings::set_selection_state))
        .route("/courses/selections/:selection_id/unassign", post(course_offerings::unassign))
        .route("/courses/selections/changes/:id/decide", post(course_offerings::decide_change))
        .route("/courses/selections/cohort", post(course_offerings::finalize_cohort));

    let academics_and_timetable = Router::new()
        // Programmes and courses. Adding a course to a programme, or editing
        // the course itself, is `AdminOnly` and stays on the technical router.
        .route("/academics", get(academics::index))
        .route("/academics/programmes/:id", get(academics::programme))
        .route("/academics/programmes/:id/enroll", post(academics::enroll))
        // Timetable
        .route("/timetable", get(timetable::page))
        .route("/timetable/slots", post(timetable::add_slot))
        .route("/timetable/slots/:id/delete", post(timetable::delete_slot))
        // Periods of a course offering, scheduled from the same page.
        .route("/timetable/offering-periods", post(timetable::add_offering_slot))
        .route(
            "/timetable/offering-periods/:id/delete",
            post(timetable::delete_offering_slot),
        )
        // Substitutes (head of department)
        .route("/substitutions", get(substitutions::page).post(substitutions::set))
        .route("/substitutions/:id/remove", post(substitutions::remove))
        // Students in the department, and correcting its attendance
        .route("/departments", get(departments::page))
        .route("/departments/:id", post(departments::place))
        .route("/departments/attendance", get(departments::attendance_page))
        .route("/departments/attendance/:entry_id/:date", get(departments::mark_form))
        .route(
            "/departments/attendance/:entry_id/:date/save",
            post(departments::mark_save),
        )
        // Exam timetable (results are filed from Marks entry)
        .route("/exams", get(exams::page))
        .route("/exams/add", post(exams::add))
        .route("/exams/:id/delete", post(exams::delete))
        .route("/exams/push", post(exams::push))
        .route("/exams/unpush", post(exams::unpush))
        .route("/exams/results", post(exams::push_results))
        .route("/exams/results/csv", post(exams::push_results_csv))
        // Marks entry grid and the management of filed results.
        .route("/marks", get(marks::page))
        .route("/marks/save", post(marks::save))
        // The course-wise blank form a head downloads for the semester (or one
        // course of it) and fills in before uploading it again.
        .route("/marks/template.csv", get(marks::template_csv))
        // Finalize a course's published marks, then correct them through a
        // reasoned, separately-approved correction.
        .route("/marks/finalize", post(marks::finalize))
        .route("/marks/:id/toggle", post(marks::toggle))
        .route("/marks/:id/delete", post(marks::delete))
        .route("/marks/:id/correct", post(marks::correct))
        .route("/marks/:id/correction/approve", post(marks::approve))
        .route("/marks/:id/correction/reject", post(marks::reject));

    // The marks CSV import uploads a file, so it needs the same body limit as
    // the other multipart forms.
    let marks_import = Router::new()
        .route("/marks/import", post(marks::import_csv))
        .route("/marks/import/preview", post(marks::import_preview))
        // One line per student, one column per course, expanded into the same
        // staged batch as the one-row-per-result import.
        .route(
            "/marks/import/courses/preview",
            post(marks::import_courses_preview),
        )
        .layer(DefaultBodyLimit::max(CONTENT_BODY_LIMIT));

    // Staged result import: paste or CSV upload stages a preview, then a
    // separate confirmation commits it. The CSV upload needs the raised limit.
    let results_import = Router::new()
        .route(
            "/exams/results/preview",
            get(results_import::preview_page).post(results_import::preview_paste),
        )
        .route("/exams/results/upload", post(results_import::upload))
        .route("/exams/results/commit", post(results_import::commit))
        .route("/exams/results/discard", post(results_import::discard))
        .layer(DefaultBodyLimit::max(CONTENT_BODY_LIMIT));

    // Study materials: the file upload makes the whole form multipart, so it
    // shares the raised body limit.
    let materials = Router::new()
        .route("/materials", get(materials::index).post(materials::create))
        .route("/materials/:id/status", post(materials::set_status))
        .route("/materials/:id/delete", post(materials::destroy))
        .layer(DefaultBodyLimit::max(CONTENT_BODY_LIMIT));

    Router::new()
        .merge(work_queue)
        .merge(areas)
        .merge(notifications)
        .merge(course_offerings)
        .merge(academics_and_timetable)
        .merge(marks_import)
        .merge(results_import)
        .merge(materials)
}

/// Everything the IT administrator's own workspace serves: the technical tools
/// at their `/admin/...` paths, and the department set mounted underneath them.
pub fn routes() -> Router<AppState> {
    let people = Router::new()
        // People
        .route("/admin/people", get(people::list).post(people::create))
        .route("/admin/people/new", get(people::new_form))
        .route("/admin/people/:id", get(people::edit_form).post(people::update))
        .route("/admin/people/:id/reset-password", post(people::reset_password))
        .route("/admin/people/:id/active", post(people::set_active))
        // End-of-term promotion: one student, or a whole programme's semester.
        .route("/admin/people/:id/promote", post(people::promote_one))
        .route("/admin/people/promote", post(people::promote_cohort))
        .route("/admin/people/delete", post(people::bulk_delete));

    // The import takes a whole class list, so it needs the same body limit as
    // the publishing forms.
    let imports = Router::new()
        .route("/admin/people/import", get(people::import_form))
        .route("/admin/people/import/upload", post(people::import_upload))
        .route("/admin/people/import/review", get(people::import_review).post(people::import_review_submit))
        .route("/admin/people/import/credentials.csv", get(people::import_credentials_csv))
        .route("/admin/people/import/finished", post(people::import_finished))
        .layer(DefaultBodyLimit::max(CONTENT_BODY_LIMIT));

    // The college-wide attendance report is `AdminOnly`, so it stays in this
    // workspace; a head of department corrects their own department's register
    // at `/departments/attendance` instead.
    let attendance_report = Router::new()
        .route("/admin/attendance", get(attendance::report))
        .route("/admin/attendance.csv", get(attendance::csv));

    // Adding courses to a programme, and editing or deleting a course, is
    // `AdminOnly`: a head of department enrols students into a semester's
    // courses, but does not change the programme's structure.
    let academics_admin = Router::new()
        .route("/admin/academics/programmes/:id/courses", post(academics::add_course))
        .route("/admin/academics/courses/:id", get(academics::course_form).post(academics::course_update))
        .route("/admin/academics/courses/:id/delete", post(academics::course_delete));

    // Documents take an uploaded file, so they need the same body limit.
    let documents = Router::new()
        .route("/admin/documents", get(documents::index).post(documents::create))
        .route("/admin/documents/new", get(documents::new_form).post(documents::create))
        .route("/admin/documents/:id", get(documents::edit_form).post(documents::update))
        .route("/admin/documents/:id/status", post(documents::set_status))
        .route("/admin/documents/:id/delete", post(documents::destroy))
        .layer(DefaultBodyLimit::max(CONTENT_BODY_LIMIT));

    let pages = Router::new()
        .route("/admin/pages", get(pages::index).post(pages::create))
        .route("/admin/pages/new", get(pages::new_form).post(pages::create))
        .route("/admin/pages/:id", get(pages::edit_form).post(pages::update))
        .route("/admin/pages/:id/status", post(pages::set_status))
        .route("/admin/pages/:id/delete", post(pages::destroy))
        .route("/admin/pages/:id/sections", post(pages::add_section))
        .route(
            "/admin/pages/:id/sections/:section_id",
            post(pages::update_section),
        )
        .route(
            "/admin/pages/:id/sections/:section_id/delete",
            post(pages::delete_section),
        )
        .route(
            "/admin/pages/:id/sections/:section_id/move",
            post(pages::move_section),
        )
        // A section form can upload a photo, so these posts are multipart and
        // need the same raised body limit as the publishing forms.
        .layer(DefaultBodyLimit::max(CONTENT_BODY_LIMIT));

    // The settings screen posts one multipart body covering every field, so it
    // shares the content body limit.
    let settings = Router::new()
        .route("/admin/settings", get(settings::index).post(settings::save))
        .route("/admin/settings/home/:key", post(settings::save_home_section))
        .route(
            "/admin/settings/display-defaults",
            post(settings::save_display_defaults),
        )
        .layer(DefaultBodyLimit::max(CONTENT_BODY_LIMIT));

    // Rank holders: the list of university rank holders and how it is presented.
    // The forms upload a photo, so they need the same body limit as documents.
    let rank_holders = Router::new()
        .route("/admin/rank-holders", get(rank_holders::index))
        .route("/admin/rank-holders/new", get(rank_holders::new_form).post(rank_holders::create))
        .route(
            "/admin/rank-holders/display",
            post(rank_holders::save_display),
        )
        .route(
            "/admin/rank-holders/:id",
            get(rank_holders::edit_form).post(rank_holders::update),
        )
        .route("/admin/rank-holders/:id/status", post(rank_holders::set_status))
        .route("/admin/rank-holders/:id/delete", post(rank_holders::destroy))
        .route("/admin/rank-holders/:id/move", post(rank_holders::reorder))
        .layer(DefaultBodyLimit::max(CONTENT_BODY_LIMIT));

    // HOD appointments and delegations: plain urlencoded forms, no uploads.
    let appointments = Router::new()
        .route("/admin/appointments", get(appointments::index))
        .route("/admin/appointments/grant", post(appointments::grant))
        // The Principal signs off the academic appointment the IT admin asks for.
        .route("/admin/appointments/:id/approve", post(appointments::approve))
        .route("/admin/appointments/:id/reject", post(appointments::reject))
        .route("/admin/appointments/:id/revoke", post(appointments::revoke))
        .route("/admin/appointments/delegate", post(appointments::delegate))
        .route(
            "/admin/appointments/delegations/:id/revoke",
            post(appointments::revoke_delegation),
        );

    // Root authority: account roles, emergency recovery, the Principal
    // appointment, and the audited reopening of a finalized academic record.
    // Every handler takes `SuperAdminOnly`, and the management listener already
    // keeps this area off the network.
    let root = Router::new()
        .route("/admin/roles", get(super_admin::page))
        .route("/admin/roles/:id/role", post(super_admin::set_role))
        .route("/admin/roles/:id/unlock", post(super_admin::unlock))
        .route("/admin/root/principal", post(super_admin::appoint_principal))
        .route(
            "/admin/root/principal/:id/revoke",
            post(super_admin::revoke_principal),
        )
        .route("/admin/root/reopen", post(super_admin::reopen_finalized));

    Router::new()
        .merge(root)
        .merge(people)
        .merge(imports)
        .merge(academics_admin)
        .merge(attendance_report)
        .merge(appointments)
        .merge(documents)
        .merge(rank_holders)
        .merge(pages)
        .merge(settings)
        // The department tools and the content tools in their `/admin` form,
        // for the roles that work in this workspace. A head of department
        // reaches the department set under `/hod` (see `manager_routes`) and
        // office staff reach the content set under `/staff` (see
        // [`staff_routes`]).
        .nest("/admin", manager_routes().merge(content_routes()))
}

/// The public-facing content tools: notices, news and events.
///
/// They are written with *relative* paths and mounted inside the caller's own
/// workspace — `/admin` for the IT administrator, `/staff` for office staff —
/// so every list, form action and redirect is built from
/// [`sections::base_for`]. Both roles take `OfficeOrAdmin` on every handler, so
/// office staff may create, edit, publish, archive and delete.
pub fn content_routes() -> Router<AppState> {
    Router::new()
        .route("/notices", get(notices::index).post(notices::create))
        .route("/notices/new", get(notices::new_form).post(notices::create))
        .route("/notices/:id", get(notices::edit_form).post(notices::update))
        .route("/notices/:id/status", post(notices::set_status))
        .route("/notices/:id/delete", post(notices::destroy))
        .route("/news", get(news::index).post(news::create))
        .route("/news/new", get(news::new_form).post(news::create))
        .route("/news/:id", get(news::edit_form).post(news::update))
        .route("/news/:id/status", post(news::set_status))
        .route("/news/:id/delete", post(news::destroy))
        .route("/events", get(events::index).post(events::create))
        .route("/events/new", get(events::new_form).post(events::create))
        .route("/events/:id", get(events::edit_form).post(events::update))
        .route("/events/:id/status", post(events::set_status))
        .route("/events/:id/delete", post(events::destroy))
        // These forms take multipart bodies, so they need a body limit well
        // above axum's 2 MB default.
        .layer(DefaultBodyLimit::max(CONTENT_BODY_LIMIT))
}

/// The office-staff workspace, mounted at `/staff` on both listeners.
///
/// It is a dashboard plus the content tools and the per-user notification
/// feed. Mounting it grants nothing: every handler keeps its own guard, so a
/// caller who is not office staff (or the IT admin) is refused here exactly as
/// they are at `/admin`.
pub fn staff_routes() -> Router<AppState> {
    Router::new()
        .route("/", get(crate::routes::dashboards::staff))
        .route("/manage/:key", get(manage::page))
        .route("/notifications", get(notifications::page))
        .route("/notifications/:id/read", post(notifications::mark_read))
        .route("/notifications/read-all", post(notifications::mark_all))
        .merge(content_routes())
}

// ---------- small form helpers shared by the admin handlers ----------

pub(super) fn parse_i64(s: &str) -> Option<i64> {
    s.trim().parse().ok()
}

pub(super) fn parse_i32(s: &str) -> Option<i32> {
    s.trim().parse().ok()
}

/// Read a checkbox field. An unchecked box is simply absent from a urlencoded
/// body (and arrives empty here), so absence must mean `false` rather than an
/// error, and every spelling a browser or test might send is accepted.
pub(super) fn checkbox(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "on" | "true" | "yes"
    )
}

/// Accepts "9:30", "09:30" or "09:30:00" and returns "09:30".
pub(super) fn normalise_time(s: &str) -> Option<String> {
    let mut parts = s.trim().split(':');
    let h: u32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    if h < 24 && m < 60 {
        Some(format!("{h:02}:{m:02}"))
    } else {
        None
    }
}

/// The status and audience options offered by the content forms.
pub(super) fn statuses() -> Vec<String> {
    crate::services::content_admin::STATUSES
        .iter()
        .map(|s| s.to_string())
        .collect()
}

pub(super) fn audiences() -> Vec<String> {
    crate::services::content_admin::AUDIENCES
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// True when `s` is a real calendar date written as `YYYY-MM-DD`.
///
/// The one implementation lives in `crate::dates`; this is the name the admin
/// handlers have always used. Kept as a thin alias so the many call sites did
/// not all have to change, while the rule itself is shared with the exam and
/// attendance forms (which each carried a subtly different copy).
pub(super) fn is_valid_date(s: &str) -> bool {
    crate::dates::valid_date(s)
}

/// Turn a `<input type="datetime-local">` value into something Postgres will read as
/// a `timestamptz` in the server's timezone ("2026-05-04 09:30").
pub(super) fn normalise_datetime(s: &str) -> Option<String> {
    let raw = s.trim().replace('T', " ");
    let (date, time) = raw.split_once(' ')?;
    if !is_valid_date(date) {
        return None;
    }
    let (h, m) = match time.split_once(':') {
        Some((h, m)) => (h, m),
        None => (time, "00"),
    };
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    Some(format!("{date} {h:02}:{m:02}"))
}

/// Normalise an optional `datetime-local` field (publish/expire times). An
/// empty value stays empty; a valid one becomes `YYYY-MM-DD HH:MM`; anything
/// else is an error the form can show.
pub(super) fn clean_when(value: &str) -> Result<String, String> {
    let v = value.trim();
    if v.is_empty() {
        return Ok(String::new());
    }
    normalise_datetime(v).ok_or_else(|| "Use a valid publish or expire time, or leave it blank.".to_string())
}

/// The CSRF token on its own, for the small per-row buttons that carry no other fields.
#[derive(serde::Deserialize)]
pub(super) struct TokenForm {
    pub csrf_token: String,
    /// The status a "publish"/"archive" button wants, empty for a delete.
    #[serde(default)]
    pub status: String,
    /// Where to send the browser afterwards, so a status change can return to
    /// whatever list page and filter it came from.
    #[serde(default)]
    pub back: String,
}

/// `back` comes from the form, so only ever follow it back into one of the two
/// management workspaces: `/admin` (the IT administrator) or `/hod` (a head of
/// department, whose pages live on the public listener).
pub(super) fn safe_back(value: &str, fallback: &str) -> String {
    if (value.starts_with(sections::ADMIN_BASE)
        || value.starts_with(sections::HOD_BASE)
        || value.starts_with(sections::STAFF_BASE))
        && !value.starts_with("//")
    {
        value.to_string()
    } else {
        fallback.to_string()
    }
}

/// The list page of a content tool (`/notices`, `/news`, `/events`) in the
/// caller's own workspace, so a redirect lands under `/staff` for office staff
/// and `/admin` for the IT administrator.
pub(super) fn content_list(user: &crate::auth::AuthUser, path: &str) -> String {
    format!("{}{}", sections::base_for(user), path)
}

#[cfg(test)]
mod tests {
    use super::checkbox;

    #[test]
    fn a_checkbox_understands_every_spelling_a_browser_sends() {
        // A checked box; every form of "on", plus the value the template sets.
        for on in ["1", "on", "true", "TRUE", "yes", " Yes "] {
            assert!(checkbox(on), "{on:?} should read as checked");
        }
        // An unchecked box is simply absent from the body, arriving empty.
        for off in ["", " ", "0", "off", "false", "no"] {
            assert!(!checkbox(off), "{off:?} should read as unchecked");
        }
    }
}
