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
    sections::{self, Area},
    services::courses,
    shell::Shell,
    state::AppState,
};

#[derive(Template)]
#[template(path = "admin/work_queue.html")]
pub struct WorkQueueTemplate {
    shell: Shell,
    department_name: String,
    /// Heading for the card grid below the queue.
    areas_title: &'static str,
    /// The management areas this caller may open. Links only — the page behind
    /// each one keeps its own guard.
    areas: &'static [Area],
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

    // The IT admin reaches the college-wide areas; a head of department gets
    // the department-scoped ones. Same rule as the links they can open.
    let (areas_title, areas) = if manager.is_admin() {
        ("Management areas", sections::ADMIN)
    } else {
        ("Department management", sections::HOD)
    };

    // Approvals arrive with every status; only the undecided ones are work.
    let approvals: Vec<courses::Approval> = courses::external_offerings_for(&s.db, department)
        .await?
        .into_iter()
        .filter(|a| a.status == "pending")
        .collect();

    Ok(WorkQueueTemplate {
        shell: Shell::build(&manager.user, &session).await?,
        department_name: department_name(&s, &manager).await?,
        areas_title,
        areas,
        approvals,
        requests: courses::pending_change_requests(&s.db, department).await?,
        pending_cohorts: courses::pending_cohort_choices(&s.db, department).await?,
        unscheduled: courses::unscheduled_offerings(&s.db, department).await?,
        capacity: courses::capacity_warnings(&s.db, department).await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(areas_title: &'static str, areas: &'static [crate::sections::Area]) -> String {
        WorkQueueTemplate {
            shell: crate::sections::test_shell("Head of department"),
            department_name: "Computer Science".into(),
            areas_title,
            areas,
            approvals: Vec::new(),
            requests: Vec::new(),
            pending_cohorts: Vec::new(),
            unscheduled: Vec::new(),
            capacity: Vec::new(),
        }
        .render()
        .expect("the work queue renders")
    }

    #[test]
    fn the_queue_keeps_its_pending_lists_and_gains_the_department_areas() {
        let html = render("Department management", crate::sections::HOD);

        // The pending work stays on the page; the cards are added, not swapped in.
        for heading in [
            "External offerings to decide",
            "Change requests",
            "Cohort decisions not finalised",
            "Published offerings with no timetable",
            "Capacity warnings",
        ] {
            assert!(html.contains(heading), "lost the {heading} section");
        }

        assert!(html.contains("Department management"));
        for a in crate::sections::HOD {
            assert!(html.contains(&format!("/admin/manage/{}", a.key)), "no card for {}", a.key);
            let title = a.title.replace('&', "&amp;");
            assert!(html.contains(&title), "no title for {}", a.key);
        }
    }

    #[test]
    fn the_it_admin_sees_the_college_wide_areas_on_the_same_page() {
        let html = render("Management areas", crate::sections::ADMIN);
        assert!(html.contains("Management areas"));
        assert!(html.contains("/admin/manage/users"));
        assert!(!html.contains("/admin/manage/students"));
    }
}
