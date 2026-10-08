//! IT-administrator tools. Every handler here takes `AdminOnly`.

mod academics;
mod attendance;
mod course_offerings;
mod departments;
mod documents;
mod exams;
mod events;
mod news;
mod notices;
mod pages;
mod people;
mod rank_holders;
mod settings;
mod substitutions;
mod timetable;
mod work_queue;

use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Router,
};

use crate::state::AppState;

/// Ceiling on a content form's whole body, a little above the per-file limit in
/// `crate::uploads` so a big attachment still has room for the text fields.
const CONTENT_BODY_LIMIT: usize = 10 * 1024 * 1024;

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

    // Course offerings and student selection (HOD tools).
    let course_offerings = Router::new()
        .route("/admin/courses", get(course_offerings::catalogue))
        .route("/admin/courses/add", post(course_offerings::add_course))
        .route("/admin/courses/:id/edit", post(course_offerings::edit_course))
        .route("/admin/courses/:id/toggle", post(course_offerings::toggle_course))
        .route("/admin/courses/offerings", get(course_offerings::offerings).post(course_offerings::create_offering))
        .route("/admin/courses/offerings/:id", get(course_offerings::offering_page).post(course_offerings::edit_offering))
        .route("/admin/courses/offerings/:id/delete", post(course_offerings::delete_offering))
        .route("/admin/courses/offerings/:id/targets", post(course_offerings::set_targets))
        .route("/admin/courses/offerings/:id/status", post(course_offerings::publish))
        .route("/admin/courses/offerings/:id/periods", post(course_offerings::add_period))
        .route("/admin/courses/offerings/:id/periods/:entry_id/delete", post(course_offerings::delete_period))
        .route("/admin/courses/external", get(course_offerings::external))
        .route("/admin/courses/external/:id/decide", post(course_offerings::decide))
        .route("/admin/courses/selections", get(course_offerings::selections))
        .route("/admin/courses/selections/assign", post(course_offerings::assign))
        .route("/admin/courses/selections/:selection_id/state", post(course_offerings::set_selection_state))
        .route("/admin/courses/selections/:selection_id/unassign", post(course_offerings::unassign))
        .route("/admin/courses/selections/changes/:id/decide", post(course_offerings::decide_change))
        .route("/admin/courses/selections/cohort", post(course_offerings::finalize_cohort));

    // The HOD landing page: every queue above in one list, with links back to
    // the pages that already handle each kind of decision.
    let work_queue = Router::new().route("/admin/work-queue", get(work_queue::page));

    let academics_and_timetable = Router::new()
        // Programmes and courses
        .route("/admin/academics", get(academics::index))
        .route("/admin/academics/programmes/:id", get(academics::programme))
        .route("/admin/academics/programmes/:id/courses", post(academics::add_course))
        .route("/admin/academics/programmes/:id/enroll", post(academics::enroll))
        .route("/admin/academics/courses/:id", get(academics::course_form).post(academics::course_update))
        .route("/admin/academics/courses/:id/delete", post(academics::course_delete))
        // Timetable
        .route("/admin/timetable", get(timetable::page))
        .route("/admin/timetable/slots", post(timetable::add_slot))
        .route("/admin/timetable/slots/:id/delete", post(timetable::delete_slot))
        // Periods of a course offering, scheduled from the same page.
        .route("/admin/timetable/offering-periods", post(timetable::add_offering_slot))
        .route(
            "/admin/timetable/offering-periods/:id/delete",
            post(timetable::delete_offering_slot),
        )
        // Substitutes (head of department)
        .route("/admin/substitutions", get(substitutions::page).post(substitutions::set))
        .route("/admin/substitutions/:id/remove", post(substitutions::remove))
        // Students in the department, and correcting its attendance
        .route("/admin/departments", get(departments::page))
        .route("/admin/departments/:id", post(departments::place))
        .route("/admin/departments/attendance", get(departments::attendance_page))
        .route("/admin/departments/attendance/:entry_id", get(departments::mark_form))
        .route(
            "/admin/departments/attendance/:entry_id/save",
            post(departments::mark_save),
        )
        // Attendance reporting
        .route("/admin/attendance", get(attendance::report))
        .route("/admin/attendance.csv", get(attendance::csv))
        // Exam timetable and semester results, pushed to students by PRN
        .route("/admin/exams", get(exams::page))
        .route("/admin/exams/add", post(exams::add))
        .route("/admin/exams/:id/delete", post(exams::delete))
        .route("/admin/exams/push", post(exams::push))
        .route("/admin/exams/unpush", post(exams::unpush))
        .route("/admin/exams/results", post(exams::push_results))
        .route("/admin/exams/results/csv", post(exams::push_results_csv));

    // Notices, news and events take multipart bodies, so they need a body limit
    // well above axum's 2 MB default.
    let publishing = Router::new()
        .route("/admin/notices", get(notices::index).post(notices::create))
        .route("/admin/notices/new", get(notices::new_form).post(notices::create))
        .route("/admin/notices/:id", get(notices::edit_form).post(notices::update))
        .route("/admin/notices/:id/status", post(notices::set_status))
        .route("/admin/notices/:id/delete", post(notices::destroy))
        .route("/admin/news", get(news::index).post(news::create))
        .route("/admin/news/new", get(news::new_form).post(news::create))
        .route("/admin/news/:id", get(news::edit_form).post(news::update))
        .route("/admin/news/:id/status", post(news::set_status))
        .route("/admin/news/:id/delete", post(news::destroy))
        .route("/admin/events", get(events::index).post(events::create))
        .route("/admin/events/new", get(events::new_form).post(events::create))
        .route("/admin/events/:id", get(events::edit_form).post(events::update))
        .route("/admin/events/:id/status", post(events::set_status))
        .route("/admin/events/:id/delete", post(events::destroy))
        .layer(DefaultBodyLimit::max(CONTENT_BODY_LIMIT));

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

    Router::new()
        .merge(people)
        .merge(imports)
        .merge(academics_and_timetable)
        .merge(course_offerings)
        .merge(work_queue)
        .merge(publishing)
        .merge(documents)
        .merge(rank_holders)
        .merge(pages)
        .merge(settings)
}

// ---------- small form helpers shared by the admin handlers ----------

pub(super) fn parse_i64(s: &str) -> Option<i64> {
    s.trim().parse().ok()
}

pub(super) fn parse_i32(s: &str) -> Option<i32> {
    s.trim().parse().ok()
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
/// The `time` crate's own parser is behind a feature this project does not enable,
/// so the check is done here. Validating before we reach Postgres keeps a
/// nonsensical day as a form error rather than a 500.
pub(super) fn is_valid_date(s: &str) -> bool {
    let mut parts = s.trim().split('-');
    let (Some(y), Some(m), Some(d), None) = (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return false;
    }
    if !y.bytes().chain(m.bytes()).chain(d.bytes()).all(|b| b.is_ascii_digit()) {
        return false;
    }
    let (Ok(y), Ok(m), Ok(d)) = (y.parse::<i32>(), m.parse::<u32>(), d.parse::<u32>()) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let max = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    d >= 1 && d <= max
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

/// `back` comes from the form, so only ever follow it back into the admin area.
pub(super) fn safe_back(value: &str, fallback: &str) -> String {
    if value.starts_with("/admin/") && !value.starts_with("//") {
        value.to_string()
    } else {
        fallback.to_string()
    }
}