//! IT-administrator tools. Every handler here takes `AdminOnly`.

mod academics;
mod attendance;
mod events;
mod news;
mod notices;
mod people;
mod timetable;

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
        .route("/admin/people/import", get(people::import_form).post(people::import_submit))
        .route("/admin/people/:id", get(people::edit_form).post(people::update))
        .route("/admin/people/:id/reset-password", post(people::reset_password))
        .route("/admin/people/:id/active", post(people::set_active));

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
        // Attendance reporting
        .route("/admin/attendance", get(attendance::report))
        .route("/admin/attendance.csv", get(attendance::csv));

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

    Router::new()
        .merge(people)
        .merge(academics_and_timetable)
        .merge(publishing)
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