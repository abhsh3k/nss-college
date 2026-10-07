//! Head-of-department tools for course offerings and selection.
//!
//! An HOD manages their own department's catalogue courses and offerings,
//! approves other departments' offerings aimed at their students, reviews
//! student selections, decides change requests and finalises cohort
//! specializations. The IT admin (`Manager` with `Scope::All`) sees everything.
//! Every write re-checks the department server-side; the id in a form proves
//! nothing.

use askama::Template;
use axum::{
    extract::{Path, State},
    response::Redirect,
    Form,
};
use serde::Deserialize;
use tower_sessions::Session;

use super::parse_i32;
use crate::{
    auth::{csrf, Manager},
    error::{is_unique_violation, AppError},
    services::{
        courses::{self, CatalogueCourse, CourseInput},
        users,
    },
    shell::{self, Shell},
    state::AppState,
};

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

// ---------- Course catalogue ----------

#[derive(Template)]
#[template(path = "admin/course_catalogue.html")]
pub struct CatalogueTemplate {
    shell: Shell,
    courses: Vec<CatalogueCourse>,
    department_name: String,
    can_edit: bool,
}

pub async fn catalogue(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
) -> Result<CatalogueTemplate, AppError> {
    let department = manager.department();
    Ok(CatalogueTemplate {
        shell: Shell::build(&manager.user, &session).await?,
        courses: match department {
            Some(id) => courses::department_courses(&s.db, id).await?,
            None => courses::catalogue_courses(&s.db, None).await?,
        },
        department_name: department_name(&s, &manager).await?,
        // Authoring courses is a department act: the IT admin may read every
        // department's catalogue but does not add or retire courses in it.
        can_edit: department.is_some(),
    })
}
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct CourseForm {
    csrf_token: String,
    code: String,
    title: String,
    credits: String,
    category: String,
    semester: String,
}

const CATEGORIES: [&str; 10] = [
    "", "MAJOR", "MINOR", "SPECIALIZATION", "DSC", "DSE", "MDC", "SEC", "VAC", "AEC",
];

fn check_course(f: &CourseForm) -> Result<CourseInput, &'static str> {
    let code = f.code.trim().to_string();
    let title = f.title.trim().to_string();
    if code.is_empty() || title.is_empty() {
        return Err("Enter both the course code and the title.");
    }
    let credits = parse_i32(&f.credits)
        .filter(|v| (0..=40).contains(v))
        .ok_or("Credits must be a number from 0 to 40.")?;
    let category = f.category.trim().to_uppercase();
    if !CATEGORIES.contains(&category.as_str()) {
        return Err("Choose a valid course category.");
    }
    let semester = parse_i32(&f.semester)
        .filter(|v| (0..=8).contains(v))
        .ok_or("Choose the suggested semester, or \"no fixed semester\".")?;
    Ok(CourseInput { code, title, credits, category, semester })
}

pub async fn add_course(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Form(f): Form<CourseForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let department_id = manager.department().ok_or(AppError::Forbidden)?;
    let input = match check_course(&f) {
        Ok(c) => c,
        Err(msg) => {
            shell::flash(&session, msg).await?;
            return Ok(Redirect::to("/admin/courses"));
        }
    };
    match courses::create_catalogue_course(&s.db, department_id, &input).await {
        Ok(id) => {
            users::audit(&s.db, Some(manager.user.id), "catalogue_course_created", "course", Some(id)).await?;
            shell::flash(&session, format!("Added {} {}.", input.code, input.title)).await?;
        }
        Err(e) if is_unique_violation(&e) => {
            shell::flash(&session, "That course code already exists in your department.").await?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(Redirect::to("/admin/courses"))
}

pub async fn edit_course(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(f): Form<CourseForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let department_id = manager.department().ok_or(AppError::Forbidden)?;
    // The course being edited must really belong to this department.
    let owner = courses::course_department(&s.db, id).await?;
    if owner != Some(department_id) {
        return Err(AppError::Forbidden);
    }
    let input = match check_course(&f) {
        Ok(c) => c,
        Err(msg) => {
            shell::flash(&session, msg).await?;
            return Ok(Redirect::to("/admin/courses"));
        }
    };
    match courses::update_catalogue_course(&s.db, id, &input).await {
        Ok(()) => {
            users::audit(&s.db, Some(manager.user.id), "catalogue_course_updated", "course", Some(id)).await?;
            shell::flash(&session, "Course saved.").await?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(Redirect::to("/admin/courses"))
}

#[derive(Deserialize)]
pub struct ToggleForm {
    csrf_token: String,
}

pub async fn toggle_course(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(f): Form<ToggleForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let department_id = manager.department().ok_or(AppError::Forbidden)?;
    if courses::course_department(&s.db, id).await? != Some(department_id) {
        return Err(AppError::Forbidden);
    }
    let active: bool = sqlx::query_scalar("SELECT is_active FROM courses WHERE id = $1")
        .bind(id)
        .fetch_one(&s.db)
        .await?;
    courses::set_course_active(&s.db, id, !active).await?;
    users::audit(&s.db, Some(manager.user.id), "catalogue_course_toggled", "course", Some(id)).await?;
    shell::flash(&session, if active { "Course retired." } else { "Course reactivated." }).await?;
    Ok(Redirect::to("/admin/courses"))
}

// ---------- Offerings ----------

#[derive(Template)]
#[template(path = "admin/offerings.html")]
pub struct OfferingsTemplate {
    shell: Shell,
    offerings: Vec<courses::OfferingRow>,
    catalogue: Vec<CatalogueCourse>,
    years: Vec<courses::AcademicYear>,
    department_name: String,
    /// Only a department head creates offerings; the IT admin reads them all.
    can_create: bool,
}

pub async fn offerings(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
) -> Result<OfferingsTemplate, AppError> {
    let department = manager.department();
    Ok(OfferingsTemplate {
        shell: Shell::build(&manager.user, &session).await?,
        offerings: courses::offerings_for_department(&s.db, department).await?,
        // An HOD offers courses from their own catalogue; the admin sees every department's.
        catalogue: match department {
            Some(id) => courses::department_courses(&s.db, id).await?,
            None => courses::catalogue_courses(&s.db, None).await?,
        },
        years: courses::academic_years(&s.db).await?,
        department_name: department_name(&s, &manager).await?,
        can_create: department.is_some(),
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct OfferingForm {
    csrf_token: String,
    course_id: String,
    academic_year_id: String,
    semester: String,
    course_type: String,
    selection_mode: String,
    choice_group: String,
    capacity: String,
}

const MODES: [&str; 4] = ["FIXED", "HOD_ASSIGNED", "INDIVIDUAL_CHOICE", "COHORT_CHOICE"];

/// The part of an offering that can be reconfigured after creation.
struct OfferingConfig {
    semester: i32,
    course_type: String,
    selection_mode: String,
    choice_group: String,
    capacity: Option<i32>,
}

/// Validate the configurable half of an offering: semester, free-text course
/// type, selection mode, choice group (mandatory for cohort choice) and seats.
fn check_config(
    semester: &str,
    course_type: &str,
    selection_mode: &str,
    choice_group: &str,
    capacity: &str,
) -> Result<OfferingConfig, String> {
    let semester = parse_i32(semester)
        .filter(|v| (1..=6).contains(v))
        .ok_or("Choose a semester from 1 to 6.")?;
    let mode = selection_mode.trim().to_uppercase();
    if !MODES.contains(&mode.as_str()) {
        return Err("Choose a selection mode.".into());
    }
    let mut group = choice_group.trim().to_string();
    if mode == "COHORT_CHOICE" && group.is_empty() {
        return Err("A cohort-choice offering needs a choice group name, e.g. \"BCA Specialization\".".into());
    }
    if mode != "COHORT_CHOICE" {
        group.clear();
    }
    let capacity = match capacity.trim() {
        "" => None,
        v => match v.parse::<i32>() {
            Ok(c) if c > 0 => Some(c),
            _ => return Err("Capacity must be a positive number.".into()),
        },
    };
    let course_type = course_type.trim().to_string();
    if course_type.is_empty() || course_type.chars().count() > 60 {
        return Err("Give the course type a short name, e.g. \"Regular\".".into());
    }
    Ok(OfferingConfig { semester, course_type, selection_mode: mode, choice_group: group, capacity })
}

fn check_offering(f: &OfferingForm) -> Result<courses::NewOffering, String> {
    let course_id: i64 = f.course_id.trim().parse().map_err(|_| "Choose a course.")?;
    let year_id: i64 = f.academic_year_id.trim().parse().map_err(|_| "Choose the academic year.")?;
    let c = check_config(
        &f.semester,
        &f.course_type,
        &f.selection_mode,
        &f.choice_group,
        &f.capacity,
    )?;
    Ok(courses::NewOffering {
        course_id,
        offering_department_id: 0, // filled in by the caller
        academic_year_id: year_id,
        semester: c.semester,
        course_type: c.course_type,
        selection_mode: c.selection_mode,
        choice_group: c.choice_group,
        capacity: c.capacity,
        faculty_id: None,
    })
}

pub async fn create_offering(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Form(f): Form<OfferingForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let department_id = manager.department().ok_or(AppError::Forbidden)?;
    let mut n = match check_offering(&f) {
        Ok(n) => n,
        Err(msg) => {
            shell::flash(&session, msg).await?;
            return Ok(Redirect::to("/admin/courses/offerings"));
        }
    };
    // The course must belong to the offering department, whatever the form says.
    if courses::course_department(&s.db, n.course_id).await? != Some(department_id) {
        return Err(AppError::Forbidden);
    }
    n.offering_department_id = department_id;
    match courses::create_offering(&s.db, &n).await {
        Ok(id) => {
            users::audit(&s.db, Some(manager.user.id), "offering_created", "course_offering", Some(id)).await?;
            shell::flash(&session, "Offering created as a draft. Add targets, then publish.").await?;
        }
        Err(e) if is_unique_violation(&e) => {
            shell::flash(&session, "That course already has an offering for this year and semester.").await?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(Redirect::to("/admin/courses/offerings"))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct EditOfferingForm {
    csrf_token: String,
    semester: String,
    course_type: String,
    selection_mode: String,
    choice_group: String,
    capacity: String,
}

/// Reconfigure an offering: semester, course type, selection mode, choice
/// group and capacity. Course, year and owning department are fixed at
/// creation, and an offering that already has student selections cannot be
/// re-shaped — the states students confirmed would stop meaning what the mode
/// says.
pub async fn edit_offering(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(f): Form<EditOfferingForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let Some(o) = courses::offering(&s.db, id).await? else {
        return Err(AppError::NotFound);
    };
    if manager.department() != Some(o.offering_department_id) && !manager.is_admin() {
        return Err(AppError::Forbidden);
    }
    let back = format!("/admin/courses/offerings/{id}");
    let cfg = match check_config(
        &f.semester,
        &f.course_type,
        &f.selection_mode,
        &f.choice_group,
        &f.capacity,
    ) {
        Ok(c) => c,
        Err(msg) => {
            shell::flash(&session, msg).await?;
            return Ok(Redirect::to(&back));
        }
    };
    let taken: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM student_course_selections WHERE offering_id = $1",
    )
    .bind(id)
    .fetch_one(&s.db)
    .await?;
    if taken > 0 {
        shell::flash(
            &session,
            "Students have already selected this offering, so its semester and selection mode are fixed.",
        )
        .await?;
        return Ok(Redirect::to(&back));
    }
    courses::update_offering(
        &s.db,
        id,
        cfg.semester,
        &cfg.course_type,
        &cfg.selection_mode,
        &cfg.choice_group,
        cfg.capacity,
    )
    .await?;
    if o.status == "published" {
        courses::auto_apply_fixed(&s.db, Some(id), None).await?;
    }
    users::audit(&s.db, Some(manager.user.id), "offering_updated", "course_offering", Some(id)).await?;
    shell::flash(&session, "Offering saved.").await?;
    Ok(Redirect::to(&back))
}

#[derive(Deserialize)]
pub struct DeleteOfferingForm {
    csrf_token: String,
}

/// A draft can be removed outright. Anything that was ever published is
/// archived instead, so student records keep their history.
pub async fn delete_offering(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(f): Form<DeleteOfferingForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let Some(o) = courses::offering(&s.db, id).await? else {
        return Err(AppError::NotFound);
    };
    if manager.department() != Some(o.offering_department_id) && !manager.is_admin() {
        return Err(AppError::Forbidden);
    }
    if courses::delete_offering(&s.db, id).await? {
        users::audit(&s.db, Some(manager.user.id), "offering_deleted", "course_offering", Some(id)).await?;
        shell::flash(&session, "Draft offering deleted.").await?;
        Ok(Redirect::to("/admin/courses/offerings"))
    } else {
        shell::flash(
            &session,
            "Only a draft can be deleted; unpublish or archive this offering instead.",
        )
        .await?;
        Ok(Redirect::to(&format!("/admin/courses/offerings/{id}")))
    }
}

// ---------- One offering: targets, approvals, timetable ----------

pub struct TargetView {
    pub programme: String,
    pub department: String,
    pub batch_year: Option<i32>,
}

pub struct ApprovalView {
    pub department: String,
    pub status: String,
    pub note: String,
}

pub struct OfferingTimetableRow {
    pub id: i64,
    pub weekday: i16,
    pub start_at: String,
    pub end_at: String,
    pub room: String,
    pub faculty: String,
}

#[derive(Template)]
#[template(path = "admin/offering.html")]
pub struct OfferingTemplate {
    shell: Shell,
    offering: courses::Offering,
    targets: Vec<TargetView>,
    approvals: Vec<ApprovalView>,
    programmes: Vec<(i64, String, String)>, // id, name, department name
    periods: Vec<OfferingTimetableRow>,
    teachers: Vec<crate::services::academics::TeacherOption>,
    can_edit: bool,
}

pub async fn offering_page(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
) -> Result<OfferingTemplate, AppError> {
    let Some(o) = courses::offering(&s.db, id).await? else {
        return Err(AppError::NotFound);
    };
    // Receiving HODs may read the page too; only the offering department may edit.
    let can_edit = manager.is_admin() || manager.department() == Some(o.offering_department_id);
    if !can_edit && manager.department().is_none() {
        return Err(AppError::Forbidden);
    }

    let targets = courses::offering_targets(&s.db, id).await?;
    let approvals = sqlx::query_as::<_, (String, String, String)>(
        r#"SELECT d.name, a.status, a.note
             FROM course_offering_approvals a
             JOIN departments d ON d.id = a.department_id
            WHERE a.offering_id = $1
            ORDER BY d.name"#,
    )
    .bind(id)
    .fetch_all(&s.db)
    .await?;

    let programmes = sqlx::query_as::<_, (i64, String, String)>(
        r#"SELECT p.id, p.name, d.name
             FROM programmes p JOIN departments d ON d.id = p.department_id
            WHERE p.status = 'published' ORDER BY d.name, p.name"#,
    )
    .fetch_all(&s.db)
    .await?;

    let periods = sqlx::query_as::<_, (i64, i16, String, String, String, String)>(
        r#"SELECT t.id, t.weekday,
                  to_char(t.start_time, 'HH24:MI'), to_char(t.end_time, 'HH24:MI'),
                  COALESCE(t.room, ''), COALESCE(f.name, '')
             FROM timetable_entries t LEFT JOIN faculty f ON f.id = t.faculty_id
            WHERE t.course_offering_id = $1
            ORDER BY t.weekday, t.start_time"#,
    )
    .bind(id)
    .fetch_all(&s.db)
    .await?;

    Ok(OfferingTemplate {
        shell: Shell::build(&manager.user, &session).await?,
        targets: targets
            .into_iter()
            .map(|t| TargetView {
                programme: t.programme,
                department: t.department,
                batch_year: t.batch_year,
            })
            .collect(),
        approvals: approvals
            .into_iter()
            .map(|(department, status, note)| ApprovalView { department, status, note })
            .collect(),
        programmes,
        periods: periods
            .into_iter()
            .map(|(id, weekday, start_at, end_at, room, faculty)| OfferingTimetableRow {
                id,
                weekday,
                start_at,
                end_at,
                room,
                faculty,
            })
            .collect(),
        teachers: crate::services::academics::teacher_options_for(
            &s.db,
            if can_edit { manager.department() } else { None },
        )
        .await?,
        offering: o,
        can_edit,
    })
}

/// Targets are read as raw form pairs: a checkbox group posts one `programmes`
/// pair per tick, which a derived `Form<T>` struct cannot deserialize. The
/// same `Form<Vec<(String, String)>>` shape the attendance sheets use.
pub async fn set_targets(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(fields): Form<Vec<(String, String)>>,
) -> Result<Redirect, AppError> {
    let field = |name: &str| -> String {
        fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    csrf::verify(&session, &field("csrf_token")).await?;
    let Some(o) = courses::offering(&s.db, id).await? else {
        return Err(AppError::NotFound);
    };
    if manager.department() != Some(o.offering_department_id) && !manager.is_admin() {
        return Err(AppError::Forbidden);
    }

    let batch_year = field("batch_year")
        .trim()
        .parse::<i32>()
        .ok()
        .filter(|v| (2000..=2100).contains(v));
    let mut programme_ids: Vec<i64> = Vec::new();
    for (k, p) in &fields {
        if k == "programmes" {
            if let Ok(pid) = p.trim().parse::<i64>() {
                programme_ids.push(pid);
            }
        }
    }
    if programme_ids.is_empty() {
        shell::flash(&session, "Choose at least one programme (or cohort).").await?;
        return Ok(Redirect::to(&format!("/admin/courses/offerings/{id}")));
    }

    // A batch year narrows the target to a cohort; create or find it on demand.
    let cohort_id = match batch_year {
        Some(year) => {
            let first: i64 = *programme_ids.first().ok_or(AppError::NotFound)?;
            if programme_ids.len() > 1 {
                shell::flash(&session, "A batch year targets one programme's cohort; pick one programme or clear the year.").await?;
                return Ok(Redirect::to(&format!("/admin/courses/offerings/{id}")));
            }
            Some(
                sqlx::query_scalar(
                    r#"INSERT INTO cohorts (programme_id, batch_year)
                       VALUES ($1, $2)
                       ON CONFLICT (programme_id, batch_year) DO UPDATE SET batch_year = EXCLUDED.batch_year
                       RETURNING id"#,
                )
                .bind(first)
                .bind(year)
                .fetch_one(&s.db)
                .await?,
            )
        }
        None => None,
    };

    courses::replace_targets(&s.db, id, &programme_ids, cohort_id).await?;
    courses::sync_approvals(&s.db, id).await?;
    // Re-apply FIXED offerings against the new audience: newly targeted
    // students are enrolled, students who dropped out of the audience have
    // the automatic application withdrawn (never a choice they made themselves).
    courses::retract_fixed(&s.db, id).await?;
    courses::auto_apply_fixed(&s.db, Some(id), None).await?;
    users::audit(&s.db, Some(manager.user.id), "offering_targets_set", "course_offering", Some(id)).await?;
    shell::flash(&session, "Target programmes saved.").await?;
    Ok(Redirect::to(&format!("/admin/courses/offerings/{id}")))
}

#[derive(Deserialize)]
pub struct PublishForm {
    csrf_token: String,
    status: String,
}

pub async fn publish(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(f): Form<PublishForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let Some(o) = courses::offering(&s.db, id).await? else {
        return Err(AppError::NotFound);
    };
    if manager.department() != Some(o.offering_department_id) && !manager.is_admin() {
        return Err(AppError::Forbidden);
    }
    let status = match f.status.as_str() {
        "published" => "published",
        "draft" => "draft",
        "archived" => "archived",
        _ => return Err(AppError::BadRequest("Unknown status.".into())),
    };
    if status == "published" && !o.has_target {
        shell::flash(&session, "Choose the target programmes before publishing.").await?;
        return Ok(Redirect::to(&format!("/admin/courses/offerings/{id}")));
    }
    courses::set_offering_status(&s.db, id, status).await?;
    let message = if status == "published" {
        courses::sync_approvals(&s.db, id).await?;
        // A FIXED offering applies itself to everyone already eligible.
        let applied = courses::auto_apply_fixed(&s.db, Some(id), None).await?;
        if applied > 0 {
            format!(
                "Offering published. It was applied automatically to {applied} fixed student(s); the receiving departments can now approve it for theirs."
            )
        } else {
            "Offering published. The receiving departments can now approve it for their students."
                .to_string()
        }
    } else {
        // FIXED applications go away with the publication; choices a student
        // or a HOD made are kept for the record.
        courses::retract_fixed(&s.db, id).await?;
        "Offering unpublished.".to_string()
    };
    users::audit(&s.db, Some(manager.user.id), "offering_status_set", "course_offering", Some(id)).await?;
    shell::flash(&session, message).await?;
    Ok(Redirect::to(&format!("/admin/courses/offerings/{id}")))
}

#[derive(Deserialize)]
pub struct ApprovalDecisionForm {
    csrf_token: String,
    decision: String,
    #[serde(default)]
    note: String,
}

/// A receiving department's HOD approves or rejects an external offering.
pub async fn decide(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(f): Form<ApprovalDecisionForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let status = match f.decision.as_str() {
        "approve" => "approved",
        "reject" => "rejected",
        _ => return Err(AppError::BadRequest("Unknown decision.".into())),
    };
    // Which offering this decision belongs to, already scoped to this HOD's
    // department: an approval aimed at someone else never resolves.
    let offering_id: Option<i64> = sqlx::query_scalar(
        "SELECT offering_id FROM course_offering_approvals WHERE id = $1 AND ($2::bigint IS NULL OR department_id = $2)",
    )
    .bind(id)
    .bind(manager.department())
    .fetch_optional(&s.db)
    .await?;
    // `decide_approval` re-checks the department server-side; the HOD can only
    // decide rows aimed at their own department.
    let ok = courses::decide_approval(
        &s.db,
        id,
        manager.department(),
        status,
        f.note.trim(),
        manager.user.id,
    )
    .await?;
    if ok {
        // Approval puts a FIXED offering in front of the students at once;
        // rejection withdraws any automatic application.
        if let Some(offering_id) = offering_id {
            if status == "approved" {
                courses::auto_apply_fixed(&s.db, Some(offering_id), None).await?;
            } else {
                courses::retract_fixed(&s.db, offering_id).await?;
            }
        }
        users::audit(&s.db, Some(manager.user.id), "offering_approval_decided", "course_offering_approval", Some(id)).await?;
        shell::flash(&session, if status == "approved" {
            "Offering approved. Your eligible students can now select it."
        } else {
            "Offering rejected; it will not reach your students."
        }).await?;
    } else {
        shell::flash(&session, "That approval is not yours to decide.").await?;
    }
    Ok(Redirect::to("/admin/courses/external"))
}

#[derive(Template)]
#[template(path = "admin/external_offerings.html")]
pub struct ExternalTemplate {
    shell: Shell,
    approvals: Vec<courses::Approval>,
    department_name: String,
}

pub async fn external(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
) -> Result<ExternalTemplate, AppError> {
    // A receiving HOD sees their own worklist; the IT admin sees them all.
    let department = manager.department();
    Ok(ExternalTemplate {
        shell: Shell::build(&manager.user, &session).await?,
        approvals: courses::external_offerings_for(&s.db, department).await?,
        department_name: department_name(&s, &manager).await?,
    })
}

// ---------- Offering timetable ----------

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct PeriodForm {
    csrf_token: String,
    weekday: String,
    start_at: String,
    end_at: String,
    room: String,
    faculty_id: String,
}

pub async fn add_period(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(f): Form<PeriodForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let Some(o) = courses::offering(&s.db, id).await? else {
        return Err(AppError::NotFound);
    };
    if manager.department() != Some(o.offering_department_id) && !manager.is_admin() {
        return Err(AppError::Forbidden);
    }

    let weekday = parse_i32(&f.weekday).filter(|v| (1..=6).contains(v)).map(|v| v as i16);
    let start = super::normalise_time(&f.start_at);
    let end = super::normalise_time(&f.end_at);
    let faculty_id = f.faculty_id.trim().parse::<i64>().ok().filter(|v| *v > 0).unwrap_or(0);
    let (Some(weekday), Some(start), Some(end)) = (weekday, start, end) else {
        shell::flash(&session, "Choose the day and both times.").await?;
        return Ok(Redirect::to(&format!("/admin/courses/offerings/{id}")));
    };
    if end <= start {
        shell::flash(&session, "The period must end after it starts.").await?;
        return Ok(Redirect::to(&format!("/admin/courses/offerings/{id}")));
    }

    // Same clash rules as the timetable page: a teacher, room or class is
    // never double-booked, whichever of the two paths added the period.
    let clashes = crate::services::academics::offering_clashes(
        &s.db,
        &crate::services::academics::NewOfferingSlot {
            offering_id: id,
            faculty_id,
            weekday,
            start_at: start.clone(),
            end_at: end.clone(),
            room: f.room.trim().to_string(),
        },
    )
    .await?;
    if let Some(c) = clashes.first() {
        let who = if c.same_class {
            "that class already has"
        } else if c.same_teacher {
            "the teacher is already teaching"
        } else {
            "the room is already used for"
        };
        shell::flash(
            &session,
            format!("Clash: {who} {} ({}–{}).", c.course, c.start_at, c.end_at),
        )
        .await?;
        return Ok(Redirect::to(&format!("/admin/courses/offerings/{id}")));
    }

    match sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO timetable_entries
               (course_offering_id, programme_id, semester, course_id, faculty_id,
                weekday, start_time, end_time, room)
           SELECT $1, NULL, o.semester, o.course_id, NULLIF($2, 0), $3, $4::time, $5::time, NULLIF($6, '')
           FROM course_offerings o WHERE o.id = $1
           RETURNING id"#,
    )
    .bind(id)
    .bind(faculty_id)
    .bind(weekday)
    .bind(&start)
    .bind(&end)
    .bind(f.room.trim())
    .fetch_one(&s.db)
    .await
    {
        Ok(entry_id) => {
            users::audit(&s.db, Some(manager.user.id), "offering_period_added", "timetable_entry", Some(entry_id)).await?;
            shell::flash(&session, format!("Added {start}–{end}.")).await?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(Redirect::to(&format!("/admin/courses/offerings/{id}")))
}

pub async fn delete_period(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path((id, entry_id)): Path<(i64, i64)>,
    Form(f): Form<ToggleForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let Some(o) = courses::offering(&s.db, id).await? else {
        return Err(AppError::NotFound);
    };
    if manager.department() != Some(o.offering_department_id) && !manager.is_admin() {
        return Err(AppError::Forbidden);
    }
    let deleted = sqlx::query(
        "DELETE FROM timetable_entries WHERE id = $1 AND course_offering_id = $2",
    )
    .bind(entry_id)
    .bind(id)
    .execute(&s.db)
    .await;
    let deleted = match deleted {
        Ok(d) => d,
        // Attendance already taken for this period: keep the row and say why.
        Err(e) if crate::error::is_foreign_key_violation(&e) => {
            shell::flash(
                &session,
                "Attendance has already been taken for this period, so it can't be removed.",
            )
            .await?;
            return Ok(Redirect::to(&format!("/admin/courses/offerings/{id}")));
        }
        Err(e) => return Err(e.into()),
    };
    if deleted.rows_affected() > 0 {
        users::audit(&s.db, Some(manager.user.id), "offering_period_removed", "timetable_entry", Some(entry_id)).await?;
        shell::flash(&session, "Period removed.").await?;
    }
    Ok(Redirect::to(&format!("/admin/courses/offerings/{id}")))
}

// ---------- Student selections (HOD view) ----------

#[derive(Template)]
#[template(path = "admin/selections.html")]
pub struct SelectionsTemplate {
    shell: Shell,
    selections: Vec<courses::DeptSelection>,
    requests: Vec<courses::ChangeRequest>,
    cohort_decisions: Vec<courses::CohortDecision>,
    pending_cohorts: Vec<courses::CohortChoicePending>,
    /// HOD-assigned offerings this department may place on its students.
    assignable: Vec<courses::AssignableOffering>,
    students: Vec<courses::AssignableStudent>,
    department_name: String,
}

pub async fn selections(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
) -> Result<SelectionsTemplate, AppError> {
    let department = manager.department();
    Ok(SelectionsTemplate {
        shell: Shell::build(&manager.user, &session).await?,
        selections: courses::department_selections(&s.db, department).await?,
        requests: courses::pending_change_requests(&s.db, department).await?,
        cohort_decisions: courses::cohort_decisions(&s.db, department).await?,
        pending_cohorts: courses::pending_cohort_choices(&s.db, department).await?,
        assignable: courses::assignable_offerings(&s.db, department).await?,
        students: courses::assignable_students(&s.db, department).await?,
        department_name: department_name(&s, &manager).await?,
    })
}

#[derive(Deserialize)]
pub struct SelectionStateForm {
    csrf_token: String,
    state: String,
}

/// Lock or unlock one confirmed selection. A locked selection cannot be
/// changed, reassigned or moved by an approval until it is unlocked.
pub async fn set_selection_state(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(selection_id): Path<i64>,
    Form(f): Form<SelectionStateForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let department = manager.department();
    let (state, locked_by) = match f.state.as_str() {
        "lock" => ("locked", Some(manager.user.id)),
        "unlock" => ("confirmed", None),
        _ => return Err(AppError::BadRequest("Unknown state.".into())),
    };
    // The selection must be confirmed or locked, and belong to this
    // department's student (the IT admin reaches all of them).
    let owned = sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS (
               SELECT 1
                 FROM student_course_selections sel
                 JOIN students st ON st.id = sel.student_id
                 JOIN programmes p ON p.id = st.programme_id
                WHERE sel.id = $1
                  AND sel.state IN ('confirmed', 'locked')
                  AND ($2::bigint IS NULL OR st.department_id = $2 OR p.department_id = $2))"#,
    )
    .bind(selection_id)
    .bind(department)
    .fetch_one(&s.db)
    .await?;
    if !owned {
        return Err(AppError::Forbidden);
    }
    sqlx::query(
        r#"UPDATE student_course_selections
              SET state = $2, locked_by = $3,
                  locked_at = CASE WHEN $2 = 'locked' THEN now() ELSE NULL END
            WHERE id = $1"#,
    )
    .bind(selection_id)
    .bind(state)
    .bind(locked_by)
    .execute(&s.db)
    .await?;
    users::audit(&s.db, Some(manager.user.id), "selection_state_set", "student_course_selection", Some(selection_id)).await?;
    shell::flash(&session, if state == "locked" { "Selection locked." } else { "Selection unlocked." }).await?;
    Ok(Redirect::to("/admin/courses/selections"))
}

#[derive(Deserialize)]
pub struct ChangeDecisionForm {
    csrf_token: String,
    decision: String,
    #[serde(default)]
    note: String,
}

pub async fn decide_change(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(f): Form<ChangeDecisionForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let approve = match f.decision.as_str() {
        "approve" => true,
        "reject" => false,
        _ => return Err(AppError::BadRequest("Unknown decision.".into())),
    };
    let ok = courses::decide_change_request(
        &s.db,
        id,
        manager.department(),
        approve,
        f.note.trim(),
        manager.user.id,
    )
    .await?;
    if ok {
        users::audit(&s.db, Some(manager.user.id), "course_change_decided", "course_change_request", Some(id)).await?;
        shell::flash(&session, if approve {
            "Change approved; the student's enrollment has been moved."
        } else {
            "Change rejected; the student keeps their confirmed course."
        }).await?;
    } else {
        shell::flash(&session, "That request is not yours to decide.").await?;
    }
    Ok(Redirect::to("/admin/courses/selections"))
}

#[derive(Deserialize)]
pub struct CohortForm {
    csrf_token: String,
    cohort_id: String,
    choice_group: String,
    offering_id: String,
    #[serde(default)]
    note: String,
}

pub async fn finalize_cohort(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Form(f): Form<CohortForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let department = manager.department();
    let cohort_id: i64 = f.cohort_id.trim().parse().map_err(|_| AppError::NotFound)?;
    let offering_id: i64 = f.offering_id.trim().parse().map_err(|_| AppError::NotFound)?;
    // The cohort must belong to a programme of this department (the IT admin
    // may finalize for any cohort).
    let owner: Option<i64> = sqlx::query_scalar(
        "SELECT p.department_id FROM cohorts cb JOIN programmes p ON p.id = cb.programme_id WHERE cb.id = $1",
    )
    .bind(cohort_id)
    .fetch_optional(&s.db)
    .await?;
    if department.is_some() && owner != department {
        return Err(AppError::Forbidden);
    }
    let applied = courses::finalize_cohort_specialization(
        &s.db,
        cohort_id,
        &f.choice_group,
        offering_id,
        f.note.trim(),
        manager.user.id,
    )
    .await?;
    users::audit(&s.db, Some(manager.user.id), "cohort_specialization_finalized", "cohort", Some(cohort_id)).await?;
    shell::flash(
        &session,
        if applied > 0 {
            format!("Specialization finalised for the cohort: {applied} student(s) inherited it.")
        } else {
            "Specialization finalised for the cohort; it has no active students yet.".into()
        },
    )
    .await?;
    Ok(Redirect::to("/admin/courses/selections"))
}

// ---------- Assigning HOD-assigned courses ----------

#[derive(Deserialize)]
pub struct AssignForm {
    csrf_token: String,
    offering_id: i64,
    student_id: i64,
}

/// Place a HOD_ASSIGNED offering on one student of this manager's department.
/// The offering's audience, semester and approval are re-checked server-side.
pub async fn assign(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Form(f): Form<AssignForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    if let Some(department) = manager.department() {
        let owned: bool = sqlx::query_scalar(
            r#"SELECT EXISTS (
                   SELECT 1 FROM students st JOIN programmes p ON p.id = st.programme_id
                    WHERE st.id = $1 AND st.is_active
                      AND COALESCE(st.department_id, p.department_id) = $2)"#,
        )
        .bind(f.student_id)
        .bind(department)
        .fetch_one(&s.db)
        .await?;
        if !owned {
            return Err(AppError::Forbidden);
        }
    }
    match courses::assign_selection(&s.db, f.offering_id, f.student_id, manager.user.id).await {
        Ok(()) => {
            users::audit(&s.db, Some(manager.user.id), "course_assigned", "course_offering", Some(f.offering_id)).await?;
            shell::flash(&session, "Course assigned; the student is enrolled.").await?;
        }
        Err(AppError::BadRequest(m)) => shell::flash(&session, m).await?,
        Err(AppError::Forbidden) => {
            shell::flash(
                &session,
                "That course is not open to that student (wrong semester or batch, or not approved yet).",
            )
            .await?
        }
        Err(e) => return Err(e),
    }
    Ok(Redirect::to("/admin/courses/selections"))
}

#[derive(Deserialize)]
pub struct UnassignForm {
    csrf_token: String,
}

/// Withdraw an assignment the department made: the selection goes and the
/// enrollment with it, unless the selection is locked.
pub async fn unassign(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(selection_id): Path<i64>,
    Form(f): Form<UnassignForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let row: Option<(i64, i64)> = sqlx::query_as(
        r#"SELECT sel.student_id, sel.offering_id
             FROM student_course_selections sel
             JOIN students st ON st.id = sel.student_id
             JOIN programmes p ON p.id = st.programme_id
            WHERE sel.id = $1
              AND ($2::bigint IS NULL OR st.department_id = $2 OR p.department_id = $2)"#,
    )
    .bind(selection_id)
    .bind(manager.department())
    .fetch_optional(&s.db)
    .await?;
    let Some((student_id, offering_id)) = row else {
        return Err(AppError::Forbidden);
    };
    match courses::unassign_selection(&s.db, offering_id, student_id).await {
        Ok(true) => {
            users::audit(&s.db, Some(manager.user.id), "course_unassigned", "student_course_selection", Some(selection_id)).await?;
            shell::flash(&session, "Assignment withdrawn; the student is no longer enrolled in that course.").await?;
        }
        Ok(false) => shell::flash(&session, "That selection is locked; unlock it first.").await?,
        Err(AppError::BadRequest(m)) => shell::flash(&session, m).await?,
        Err(e) => return Err(e),
    }
    Ok(Redirect::to("/admin/courses/selections"))
}
