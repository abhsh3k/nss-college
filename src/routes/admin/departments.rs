//! Head-of-department tools: placing students in a department, and the
//! whole-department attendance report that an HOD may correct.
//!
//! Every handler here takes `Manager`, so the IT admin can use them too. An HOD
//! is confined to their own department: each write re-checks that the row being
//! touched really belongs to the department the guard reported, rather than
//! trusting the id that arrived in the form.

use askama::Template;
use axum::{
    extract::{Path, State},
    response::Redirect,
    Form,
};
use serde::Deserialize;
use tower_sessions::Session;

use super::parse_i64;
use crate::{
    auth::{csrf, Manager},
    error::AppError,
    services::{
        attendance::{self, DeptCourse, PeriodRow, RosterRow, Rules},
        users,
    },
    shell::{self, Shell},
    state::AppState,
};

/// One row of the department's own attendance report.
pub struct ReportLine {
    pub name: String,
    pub admission_no: String,
    pub marked: i64,
    pub present: i64,
    pub absent: i64,
    pub on_leave: i64,
    pub percent_label: String,
    pub low: bool,
}

#[derive(sqlx::FromRow)]
pub struct DepartmentOption {
    pub id: i64,
    pub name: String,
}

/// The department this manager is called, for page headings.
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

// ---------- Students in this department ----------

#[derive(sqlx::FromRow)]
pub struct StudentRow {
    pub id: i64,
    pub name: String,
    pub admission_no: String,
    pub programme: String,
    pub semester: i32,
    pub department: String,
}

#[derive(Template)]
#[template(path = "admin/departments.html")]
pub struct DepartmentsTemplate {
    shell: Shell,
    students: Vec<StudentRow>,
    departments: Vec<DepartmentOption>,
    department_name: String,
    can_place_anywhere: bool,
}

pub async fn page(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
) -> Result<DepartmentsTemplate, AppError> {
    let department = manager.department();

    // An HOD sees the students of their own department, whether they have been
    // placed explicitly or arrive through their programme.
    let students = sqlx::query_as::<_, StudentRow>(
        r#"SELECT s.id, s.name, s.admission_no, p.name AS programme, s.semester,
                  COALESCE(d.name, 'Unassigned') AS department
           FROM students s
           JOIN programmes p ON p.id = s.programme_id
           LEFT JOIN departments d ON d.id = s.department_id
           WHERE s.is_active
             AND ($1::bigint IS NULL OR s.department_id = $1 OR p.department_id = $1)
           ORDER BY s.name"#,
    )
    .bind(department)
    .fetch_all(&s.db)
    .await?;

    let departments = sqlx::query_as::<_, DepartmentOption>(
        "SELECT id, name FROM departments ORDER BY sort_order, name",
    )
    .fetch_all(&s.db)
    .await?;

    Ok(DepartmentsTemplate {
        shell: Shell::build(&manager.user, &session).await?,
        students,
        departments,
        department_name: department_name(&s, &manager).await?,
        can_place_anywhere: manager.is_admin(),
    })
}

#[derive(Deserialize)]
pub struct PlaceForm {
    csrf_token: String,
    department_id: String,
}

pub async fn place(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(student_id): Path<i64>,
    Form(f): Form<PlaceForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let target = parse_i64(&f.department_id).filter(|v| *v > 0).ok_or(AppError::NotFound)?;

    // The IT admin may place a student anywhere; an HOD only in their own
    // department, whatever the form claims.
    if let Some(own) = manager.department() {
        if target != own {
            return Err(AppError::Forbidden);
        }
    }

    let updated = sqlx::query("UPDATE students SET department_id = $2 WHERE id = $1 AND is_active")
        .bind(student_id)
        .bind(target)
        .execute(&s.db)
        .await?;
    if updated.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    users::audit(
        &s.db,
        Some(manager.user.id),
        "student_department_set",
        "student",
        Some(student_id),
    )
    .await?;
    shell::flash(&session, "Student placed in that department.").await?;
    Ok(Redirect::to("/admin/departments"))
}

// ---------- Whole-department attendance ----------

pub struct CourseReport {
    pub course: DeptCourse,
    pub lines: Vec<ReportLine>,
}

#[derive(Template)]
#[template(path = "admin/department_attendance.html")]
pub struct DepartmentAttendanceTemplate {
    shell: Shell,
    courses: Vec<CourseReport>,
    department_name: String,
    rules: Rules,
}

pub async fn attendance_page(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
) -> Result<DepartmentAttendanceTemplate, AppError> {
    let rules = attendance::rules(&s.db).await?;
    let mut courses = Vec::new();

    for course in attendance::department_courses(&s.db, manager.department()).await? {
        let lines = attendance::course_report(&s.db, course.id)
            .await?
            .into_iter()
            .map(|r| {
                let pct = attendance::percent(r.present, r.on_leave, r.marked, rules.leave_counts);
                ReportLine {
                    name: r.name,
                    admission_no: r.admission_no,
                    marked: r.marked,
                    present: r.present,
                    absent: r.absent,
                    on_leave: r.on_leave,
                    percent_label: pct
                        .map(|p| format!("{p:.1}%"))
                        .unwrap_or_else(|| "No data".into()),
                    low: pct.map(|p| p < rules.min_percent as f64).unwrap_or(false),
                }
            })
            .collect();
        courses.push(CourseReport { course, lines });
    }

    Ok(DepartmentAttendanceTemplate {
        shell: Shell::build(&manager.user, &session).await?,
        courses,
        department_name: department_name(&s, &manager).await?,
        rules,
    })
}

/// Refuse anything that is not a period in a department this manager owns.
async fn ensure_our_department(
    s: &AppState,
    manager: &Manager,
    entry_id: i64,
) -> Result<(), AppError> {
    if let Some(own) = manager.department() {
        match attendance::entry_department(&s.db, entry_id).await? {
            Some(actual) if actual == own => Ok(()),
            _ => Err(AppError::Forbidden),
        }
    } else {
        Ok(())
    }
}

// One period's register, editable by the HOD.
#[derive(Template)]
#[template(path = "admin/department_mark.html")]
pub struct DepartmentMarkTemplate {
    shell: Shell,
    period: PeriodRow,
    roster: Vec<RosterRow>,
    window_days: i32,
    csrf_token: String,
}

pub async fn mark_form(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path((entry_id, date)): Path<(i64, String)>,
) -> Result<DepartmentMarkTemplate, AppError> {
    if !attendance::valid_date(&date) {
        return Err(AppError::NotFound);
    }
    ensure_our_department(&s, &manager, entry_id).await?;

    let rules = attendance::rules(&s.db).await?;
    let period =
        attendance::period_for_manager(&s.db, entry_id, &date, rules.edit_window_days)
            .await?
            .ok_or(AppError::NotFound)?;
    let roster = attendance::roster(&s.db, period.course_id, entry_id, &date).await?;
    let csrf_token = csrf::token(&session).await?;

    Ok(DepartmentMarkTemplate {
        shell: Shell::build(&manager.user, &session).await?,
        period,
        roster,
        window_days: rules.edit_window_days,
        csrf_token,
    })
}

pub async fn mark_save(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path((entry_id, date)): Path<(i64, String)>,
    Form(fields): Form<Vec<(String, String)>>,
) -> Result<Redirect, AppError> {
    let token = fields
        .iter()
        .find(|(k, _)| k == "csrf_token")
        .map(|(_, v)| v.as_str())
        .unwrap_or("");
    csrf::verify(&session, token).await?;
    if !attendance::valid_date(&date) {
        return Err(AppError::NotFound);
    }
    ensure_our_department(&s, &manager, entry_id).await?;

    let rules = attendance::rules(&s.db).await?;
    let Some(period) =
        attendance::period_for_manager(&s.db, entry_id, &date, rules.edit_window_days).await?
    else {
        shell::flash(&session, "That period can't be changed.").await?;
        return Ok(Redirect::to("/admin/departments/attendance"));
    };
    let roster = attendance::roster(&s.db, period.course_id, entry_id, &date).await?;

    let mut ids: Vec<i64> = Vec::with_capacity(roster.len());
    let mut statuses: Vec<String> = Vec::with_capacity(roster.len());
    for st in &roster {
        let key = format!("status_{}", st.student_id);
        match fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str()) {
            Some(v @ ("present" | "absent" | "leave")) => {
                ids.push(st.student_id);
                statuses.push(v.to_string());
            }
            _ => {
                shell::flash(&session, "Choose present, absent or leave for every student.").await?;
                return Ok(Redirect::to(&format!(
                    "/admin/departments/attendance/{entry_id}?date={date}"
                )));
            }
        }
    }

    // The register belongs to the period's own teacher; the HOD is the one
    // correcting it, which the audit log records.
    let taught_by: i64 =
        sqlx::query_scalar("SELECT faculty_id FROM timetable_entries WHERE id = $1")
            .bind(entry_id)
            .fetch_one(&s.db)
            .await?;
    attendance::save(&s.db, &period, taught_by, manager.user.id, &ids, &statuses).await?;
    users::audit(
        &s.db,
        Some(manager.user.id),
        "attendance_corrected_by_hod",
        "attendance_session",
        Some(entry_id),
    )
    .await?;
    shell::flash(&session, "Attendance corrected.").await?;
    Ok(Redirect::to("/admin/departments/attendance"))
}