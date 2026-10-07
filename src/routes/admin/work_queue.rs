//! The head of department's landing page: everything awaiting a decision, in
//! one place.
//!
//! The catalogue, the offering detail page and the selections page stay exactly
//! as they are — this page only gathers what is *waiting* (approvals, change
//! requests, cohort decisions, published offerings with no timetable, and full
//! seats) and links each row to the page that already knows how to act on it.

use askama::Template;
use axum::extract::State;
use tower_sessions::Session;

use crate::{
    auth::Manager,
    error::AppError,
    services::courses,
    shell::Shell,
    state::AppState,
};

#[derive(Template)]
#[template(path = "admin/work_queue.html")]
pub struct WorkQueueTemplate {
    shell: Shell,
    department_name: String,
    /// External offerings still waiting for this department's decision.
    approvals: Vec<courses::Approval>,
    /// Students asking to move between confirmed courses.
    requests: Vec<courses::ChangeRequest>,
    /// Cohorts whose choice group has no finalisation yet.
    pending_cohorts: Vec<courses::CohortChoicePending>,
    /// Published offerings with no timetable periods at all.
    unscheduled: Vec<courses::UnscheduledOffering>,
    /// Published offerings with no seats left.
    capacity: Vec<courses::CapacityWarning>,
}

async fn department_name(s: &AppState, manager: &Manager) -> Result<String, AppError> {
    Ok(match manager.department() {
        Some(id) => sqlx::query_scalar::<_, String>("SELECT name FROM departments WHERE id = $1")
            .bind(id)
            .fetch_optional(&s.db)
            .await?
            .unwrap_or_else(|| "your department".into()),
        None => "the college".into(),
    })
}

pub async fn page(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
) -> Result<WorkQueueTemplate, AppError> {
    let department = manager.department();

    // Approvals arrive with every status; only the undecided ones are work.
    let approvals: Vec<courses::Approval> = courses::external_offerings_for(&s.db, department)
        .await?
        .into_iter()
        .filter(|a| a.status == "pending")
        .collect();

    Ok(WorkQueueTemplate {
        shell: Shell::build(&manager.user, &session).await?,
        department_name: department_name(&s, &manager).await?,
        approvals,
        requests: courses::pending_change_requests(&s.db, department).await?,
        pending_cohorts: courses::pending_cohort_choices(&s.db, department).await?,
        unscheduled: courses::unscheduled_offerings(&s.db, department).await?,
        capacity: courses::capacity_warnings(&s.db, department).await?,
    })
}
