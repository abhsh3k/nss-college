use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Form,
};
use serde::Deserialize;
use sqlx::FromRow;
use tower_sessions::Session;

use crate::{
    auth::{csrf, AuthUser, TeacherOnly},
    error::AppError,
    services::{
        attendance::{self, CoverRow, PeriodRow, RosterRow, Rules, TeacherCourse, TeacherSlot},
        users,
    },
    shell::Shell,
    state::AppState,
};

pub struct StudentAttendanceItem {
    pub student_id: i64,
    pub full_name: String,
    pub roll_number: String,
    pub status: String,
}

#[derive(Template)]
#[template(path = "partials/attendance_sheet.html")]
pub struct AttendanceSheetTemplate {
    pub entry_id: i64,
    pub session_id: i64,
    pub on_date_str: String,
    pub csrf_token: String,
    pub students: Vec<StudentAttendanceItem>,
}

/// One roster row: the student's current status, defaulting to present.
pub async fn get_attendance_sheet(
    State(s): State<AppState>,session: Session,
    TeacherOnly(user): TeacherOnly,
    Path(entry_id): Path<i64>,
) -> Result<AttendanceSheetTemplate, AppError> {
    let faculty_id: i64 = sqlx::query_scalar("SELECT id FROM faculty WHERE user_id = $1")
        .bind(user.id)
        .fetch_one(&s.db)
        .await?;

    // Opening a sheet is a read. It must not create an attendance session and
    // it must resolve the entry through the same ownership/substitution query
    // as the normal attendance flow.
    let rules = attendance::rules(&s.db).await?;
    let (today_str, _, _) = attendance::now_parts(&s.db).await?;
    let Some(period) = attendance::period(
        &s.db,
        faculty_id,
        entry_id,
        &today_str,
        rules.edit_window_days,
    )
    .await?
    else {
        return Err(AppError::Forbidden);
    };
    let session_id: i64 = sqlx::query_scalar(
        "SELECT id FROM attendance_sessions WHERE timetable_entry_id = $1 AND on_date = $2::date",
    )
    .bind(entry_id)
    .bind(&today_str)
    .fetch_optional(&s.db)
    .await?
    .unwrap_or(0);
    let students = attendance::roster(&s.db, period.course_id, entry_id, &today_str)
        .await?
        .into_iter()
        .map(|r| StudentAttendanceItem {
            student_id: r.student_id,
            full_name: r.name,
            roll_number: r.admission_no,
            status: r.status,
        })
        .collect();

    Ok(AttendanceSheetTemplate {
        entry_id,
        session_id,
        on_date_str: today_str,
        csrf_token: csrf::token(&session).await?,
        students,
    })
}

#[derive(Deserialize)]
pub struct AttendanceToggleInput {
    pub student_id: i64,
    pub status: String,
    pub csrf_token: String,
    #[serde(default)]
    pub entry_id: i64,
}

#[derive(Template)]
#[template(path = "partials/attendance_badge.html")]
pub struct AttendanceBadgeTemplate {
    pub session_id: i64,
    pub student_id: i64,
    pub status: String,
    pub csrf_token: String,
}

pub async fn toggle_attendance_status(
    State(s): State<AppState>,
    session: Session,
    TeacherOnly(user): TeacherOnly,
    Path(session_id): Path<i64>,
    Form(payload): Form<AttendanceToggleInput>,
) -> Result<AttendanceBadgeTemplate, AppError> {
    csrf::verify(&session, &payload.csrf_token).await?;

    if !matches!(payload.status.as_str(), "present" | "absent" | "leave") {
        return Err(AppError::Forbidden);
    }

    let faculty_id: i64 = sqlx::query_scalar("SELECT id FROM faculty WHERE user_id = $1")
        .bind(user.id)
        .fetch_one(&s.db)
        .await?;
    let rules = attendance::rules(&s.db).await?;
    let (today_str, _, _) = attendance::now_parts(&s.db).await?;
    let entry_id = if session_id > 0 {
        sqlx::query_scalar::<_, i64>(
            "SELECT timetable_entry_id FROM attendance_sessions WHERE id = $1 AND on_date = $2::date",
        )
        .bind(session_id)
        .bind(&today_str)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::Forbidden)?
    } else if payload.entry_id > 0 {
        payload.entry_id
    } else {
        return Err(AppError::Forbidden);
    };
    let period = attendance::period(
        &s.db,
        faculty_id,
        entry_id,
        &today_str,
        rules.edit_window_days,
    )
    .await?
    .ok_or(AppError::Forbidden)?;
    let session_id = if session_id > 0 {
        session_id
    } else {
        sqlx::query_scalar(
            r#"INSERT INTO attendance_sessions
                 (timetable_entry_id, course_id, on_date, taught_by, marked_by)
               VALUES ($1, $2, $3::date, $4, $5)
               ON CONFLICT (timetable_entry_id, on_date)
               DO UPDATE SET marked_by = EXCLUDED.marked_by, updated_at = now()
               RETURNING id"#,
        )
        .bind(entry_id)
        .bind(period.course_id)
        .bind(&today_str)
        .bind(faculty_id)
        .bind(user.id)
        .fetch_one(&s.db)
        .await?
    };
    let on_roster = attendance::roster(&s.db, period.course_id, entry_id, &today_str)
        .await?
        .into_iter()
        .any(|r| r.student_id == payload.student_id);
    if !on_roster {
        return Err(AppError::Forbidden);
    }

    sqlx::query(
        r#"
        INSERT INTO attendance_records (session_id, student_id, status, updated_by)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (session_id, student_id)
        DO UPDATE SET status = EXCLUDED.status, updated_by = EXCLUDED.updated_by, updated_at = now()
        "#,
    )
    .bind(session_id)
    .bind(payload.student_id)
    .bind(&payload.status)
    .bind(user.id)
    .execute(&s.db)
    .await?;

    Ok(AttendanceBadgeTemplate {
        session_id,
        student_id: payload.student_id,
        status: payload.status,
        csrf_token: payload.csrf_token,
    })
}

// ---------- Teacher workspace ----------
//
// Restored from the pre-merge implementation. These pages are backed by
// `services::attendance`; the HTMX sheet above is the newer, self-contained one.
// Both are kept: the sheet is reachable from the dashboard, these from the sidebar.

#[derive(Template)]
#[template(path = "teacher/no_profile.html")]
pub struct NoProfileTemplate {
    shell: Shell,
}

/// Teachers need a teacher profile (created by the admin under People). Without one, say so.
async fn teacher_profile(s: &AppState, user: &AuthUser) -> Result<Option<i64>, AppError> {
    Ok(attendance::faculty_id_for_user(&s.db, user.id).await?)
}

pub struct DayGroup {
    pub label: String,
    pub periods: Vec<PeriodRow>,
}

fn group(periods: Vec<PeriodRow>) -> Vec<DayGroup> {
    let mut groups: Vec<DayGroup> = Vec::new();
    for p in periods {
        match groups.last_mut() {
            Some(g) if g.label == p.day_label => g.periods.push(p),
            _ => groups.push(DayGroup { label: p.day_label.clone(), periods: vec![p] }),
        }
    }
    groups
}

// ---------- Today ----------

#[derive(Template)]
#[template(path = "teacher/today.html")]
pub struct TodayTemplate {
    shell: Shell,
    groups: Vec<DayGroup>,
    window_days: i32,
}

pub async fn today(
    State(s): State<AppState>,
    session: Session,
    TeacherOnly(user): TeacherOnly,
) -> Result<Response, AppError> {
    let shell = Shell::build(&user, &session).await?;
    let Some(fid) = teacher_profile(&s, &user).await? else {
        return Ok(NoProfileTemplate { shell }.into_response());
    };
    let rules = attendance::rules(&s.db).await?;
    Ok(TodayTemplate {
        shell,
        groups: group(attendance::periods(&s.db, fid, 0).await?),
        window_days: rules.edit_window_days,
    }
    .into_response())
}

// ---------- Recent periods (catch-up and edits) ----------

#[derive(Template)]
#[template(path = "teacher/attendance_list.html")]
pub struct AttendanceListTemplate {
    shell: Shell,
    groups: Vec<DayGroup>,
    window_days: i32,
}

pub async fn attendance_list(
    State(s): State<AppState>,
    session: Session,
    TeacherOnly(user): TeacherOnly,
) -> Result<Response, AppError> {
    let shell = Shell::build(&user, &session).await?;
    let Some(fid) = teacher_profile(&s, &user).await? else {
        return Ok(NoProfileTemplate { shell }.into_response());
    };
    let rules = attendance::rules(&s.db).await?;
    Ok(AttendanceListTemplate {
        shell,
        groups: group(attendance::periods(&s.db, fid, rules.edit_window_days).await?),
        window_days: rules.edit_window_days,
    }
    .into_response())
}

// ---------- Take or edit attendance for one period ----------

#[derive(Template)]
#[template(path = "teacher/mark.html")]
pub struct MarkTemplate {
    shell: Shell,
    period: PeriodRow,
    roster: Vec<RosterRow>,
    window_days: i32,
}

pub async fn mark_form(
    State(s): State<AppState>,
    session: Session,
    TeacherOnly(user): TeacherOnly,
    Path((entry_id, date)): Path<(i64, String)>,
) -> Result<Response, AppError> {
    if !attendance::valid_date(&date) {
        return Err(AppError::NotFound);
    }
    let shell = Shell::build(&user, &session).await?;
    let Some(fid) = teacher_profile(&s, &user).await? else {
        return Ok(NoProfileTemplate { shell }.into_response());
    };
    let rules = attendance::rules(&s.db).await?;
    let period = attendance::period(&s.db, fid, entry_id, &date, rules.edit_window_days)
        .await?
        .ok_or(AppError::NotFound)?;
    let roster = attendance::roster(&s.db, period.course_id, entry_id, &date).await?;
    Ok(MarkTemplate { shell, period, roster, window_days: rules.edit_window_days }.into_response())
}

#[derive(Template)]
#[template(path = "partials/attendance_saved.html")]
pub struct SavedTemplate {
    error: Option<String>,
    present: usize,
    absent: usize,
    on_leave: usize,
    window_days: i32,
}

impl SavedTemplate {
    fn error(msg: &str, window_days: i32) -> Self {
        SavedTemplate { error: Some(msg.to_string()), present: 0, absent: 0, on_leave: 0, window_days }
    }
}

/// HTMX posts the whole register; the reply replaces only the small "saved" message.
pub async fn mark_save(
    State(s): State<AppState>,
    session: Session,
    TeacherOnly(user): TeacherOnly,
    Path((entry_id, date)): Path<(i64, String)>,
    Form(fields): Form<Vec<(String, String)>>,
) -> Result<SavedTemplate, AppError> {
    let token = fields
        .iter()
        .find(|(k, _)| k == "csrf_token")
        .map(|(_, v)| v.as_str())
        .unwrap_or("");
    csrf::verify(&session, token).await?;
    if !attendance::valid_date(&date) {
        return Err(AppError::NotFound);
    }

    let rules: Rules = attendance::rules(&s.db).await?;
    let Some(fid) = teacher_profile(&s, &user).await? else {
        return Ok(SavedTemplate::error("Your account is not linked to a teacher profile.", rules.edit_window_days));
    };
    let Some(period) = attendance::period(&s.db, fid, entry_id, &date, rules.edit_window_days).await? else {
        return Ok(SavedTemplate::error(
            &format!("This period can't be changed. It is not your class, or it is older than {} days.", rules.edit_window_days),
            rules.edit_window_days,
        ));
    };

    let roster = attendance::roster(&s.db, period.course_id, entry_id, &date).await?;
    if roster.is_empty() {
        return Ok(SavedTemplate::error("No students are enrolled in this course yet.", rules.edit_window_days));
    }

    let mut ids: Vec<i64> = Vec::with_capacity(roster.len());
    let mut statuses: Vec<String> = Vec::with_capacity(roster.len());
    for st in &roster {
        let key = format!("status_{}", st.student_id);
        let value = fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str());
        match value {
            Some(v @ ("present" | "absent" | "leave")) => {
                ids.push(st.student_id);
                statuses.push(v.to_string());
            }
            _ => {
                return Ok(SavedTemplate::error(
                    "Choose present, absent or leave for every student, then save again.",
                    rules.edit_window_days,
                ))
            }
        }
    }

    let session_id = attendance::save(&s.db, &period, fid, user.id, &ids, &statuses).await?;
    users::audit(&s.db, Some(user.id), "attendance_saved", "attendance_session", Some(session_id)).await?;

    let count = |wanted: &str| statuses.iter().filter(|x| x.as_str() == wanted).count();
    Ok(SavedTemplate {
        error: None,
        present: count("present"),
        absent: count("absent"),
        on_leave: count("leave"),
        window_days: rules.edit_window_days,
    })
}

// ---------- Weekly timetable ----------

pub struct WeekDay {
    pub name: &'static str,
    pub slots: Vec<TeacherSlot>,
}

#[derive(Template)]
#[template(path = "teacher/timetable.html")]
pub struct TimetableTemplate {
    shell: Shell,
    days: Vec<WeekDay>,
    covers: Vec<CoverRow>,
}

pub async fn timetable(
    State(s): State<AppState>,
    session: Session,
    TeacherOnly(user): TeacherOnly,
) -> Result<Response, AppError> {
    let shell = Shell::build(&user, &session).await?;
    let Some(fid) = teacher_profile(&s, &user).await? else {
        return Ok(NoProfileTemplate { shell }.into_response());
    };
    let names = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    let mut days: Vec<WeekDay> = names.iter().map(|n| WeekDay { name: n, slots: Vec::new() }).collect();
    for slot in attendance::teacher_slots(&s.db, fid).await? {
        if let Some(d) = days.get_mut((slot.weekday - 1) as usize) {
            d.slots.push(slot);
        }
    }
    days.retain(|d| !d.slots.is_empty());
    Ok(TimetableTemplate {
        shell,
        days,
        covers: attendance::upcoming_covers(&s.db, fid).await?,
    }
    .into_response())
}

// ---------- Reports ----------

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

#[derive(Deserialize)]
pub struct ReportQuery {
    course: Option<i64>,
}

#[derive(Template)]
#[template(path = "teacher/reports.html")]
pub struct ReportsTemplate {
    shell: Shell,
    courses: Vec<TeacherCourse>,
    selected: i64,
    course_title: String,
    lines: Vec<ReportLine>,
    rules: Rules,
}

pub async fn reports(
    State(s): State<AppState>,
    session: Session,
    TeacherOnly(user): TeacherOnly,
    Query(q): Query<ReportQuery>,
) -> Result<Response, AppError> {
    let shell = Shell::build(&user, &session).await?;
    let Some(fid) = teacher_profile(&s, &user).await? else {
        return Ok(NoProfileTemplate { shell }.into_response());
    };
    let rules = attendance::rules(&s.db).await?;
    let courses = attendance::teacher_courses(&s.db, fid).await?;

    let selected = q.course.unwrap_or(0);
    let chosen = courses.iter().find(|c| c.id == selected);
    let (course_title, lines) = match chosen {
        None => (String::new(), Vec::new()),
        Some(c) => {
            let lines = attendance::course_report(&s.db, c.id)
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
                        percent_label: pct.map(|p| format!("{p:.1}%")).unwrap_or_else(|| "No data".into()),
                        low: pct.map(|p| p < rules.min_percent as f64).unwrap_or(false),
                    }
                })
                .collect();
            (format!("{} {}", c.code, c.title), lines)
        }
    };

    Ok(ReportsTemplate { shell, courses, selected, course_title, lines, rules }.into_response())
}
