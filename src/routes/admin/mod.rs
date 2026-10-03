//! IT-administrator tools. Every handler here takes `AdminOnly`.

mod academics;
mod people;
mod timetable;

use axum::{
    routing::{get, post},
    Router,
};

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        // People
        .route("/admin/people", get(people::list).post(people::create))
        .route("/admin/people/new", get(people::new_form))
        .route("/admin/people/import", get(people::import_form).post(people::import_submit))
        .route("/admin/people/:id", get(people::edit_form).post(people::update))
        .route("/admin/people/:id/reset-password", post(people::reset_password))
        .route("/admin/people/:id/active", post(people::set_active))
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
