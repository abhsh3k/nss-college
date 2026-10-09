//! The student hub's own pages: the timetable, results for any semester, and
//! the exam timetable while it has been pushed. The overview keeps notices,
//! today's classes and attendance; every other section lives here.

use std::collections::{BTreeMap, HashMap};

use askama::Template;
use axum::extract::{Query, State};
use serde::Deserialize;
use sqlx::types::time::OffsetDateTime;
use crate::services::attendance;
use tower_sessions::Session;

use super::dashboards::{period_detail, HubCell, HubDay, HubPeriod, HubRow};
use crate::services::exams::{self, ResultCourse, StudentExam};
use crate::{
    auth::StudentOnly,
    error::AppError,
    services::hub,
    shell::Shell,
    state::AppState,
};

// ---------- Timetable ----------

#[derive(Template)]
#[template(path = "hub/timetable.html")]
pub struct TimetableTemplate {
    shell: Shell,
    today_str: String,
    has_profile: bool,
    today_periods: Vec<HubPeriod>,
    has_week: bool,
    days: Vec<HubDay>,
    rows: Vec<HubRow>,
}

async fn now_parts(db: &sqlx::PgPool) -> Result<(String, String, i16), AppError> {
    let today_str = attendance::today(db).await?;
    let now = OffsetDateTime::now_utc();
    let now_hm = format!("{:02}:{:02}", now.hour(), now.minute());
    let weekday_today: i16 = sqlx::query_scalar("SELECT EXTRACT(ISODOW FROM (now() AT TIME ZONE 'Asia/Kolkata'))::int")
        .fetch_one(db)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok((today_str, now_hm, weekday_today))
}

pub async fn timetable(
    State(s): State<AppState>,
    session: Session,
    StudentOnly(user): StudentOnly,
) -> Result<TimetableTemplate, AppError> {
    let shell = Shell::build(&user, &session).await?;
    let (today_str, now_hm, weekday_today) = now_parts(&s.db).await?;

    let Some(p) = hub::student_profile(&s.db, user.id).await? else {
        return Ok(TimetableTemplate {
            shell,
            today_str,
            has_profile: false,
            today_periods: Vec::new(),
            has_week: false,
            days: Vec::new(),
            rows: Vec::new(),
        });
    };

    // Offering periods the student is enrolled in come along with their
    // programme's class periods.
    let week = hub::week_schedule(&s.db, p.id, p.programme_id, p.semester).await?;
    let today_periods = super::dashboards::today_periods(&week, weekday_today, &now_hm);

    // Weekly matrix: rows = distinct start times, columns = days (Sunday only
    // if used).
    let sunday_used = week.iter().any(|x| x.weekday == 7);
    let day_specs: [(i16, &str); 7] = [
        (1, "Mon"),
        (2, "Tue"),
        (3, "Wed"),
        (4, "Thu"),
        (5, "Fri"),
        (6, "Sat"),
        (7, "Sun"),
    ];
    let days: Vec<HubDay> = day_specs
        .iter()
        .filter(|(num, _)| *num < 7 || sunday_used)
        .map(|(num, short)| HubDay {
            num: *num,
            short,
            is_today: *num == weekday_today,
        })
        .collect();

    let mut grid: BTreeMap<String, HashMap<i16, HubCell>> = BTreeMap::new();
    for x in &week {
        grid.entry(x.start_time.clone()).or_default().insert(
            x.weekday,
            HubCell {
                code: x.code.clone(),
                title: x.title.clone(),
                detail: period_detail(&x.room, &x.faculty),
                is_today: false,
            },
        );
    }
    let rows: Vec<HubRow> = grid
        .into_iter()
        .map(|(time, cells)| HubRow {
            time,
            cells: days
                .iter()
                .map(|d| {
                    cells.get(&d.num).cloned().map(|mut c| {
                        c.is_today = d.is_today;
                        c
                    })
                })
                .collect(),
        })
        .collect();
    let has_week = !rows.is_empty();

    Ok(TimetableTemplate {
        shell,
        today_str,
        has_profile: true,
        today_periods,
        has_week,
        days,
        rows,
    })
}

// ---------- Results ----------

#[derive(Deserialize, Default)]
pub struct ResultsQuery {
    sem: Option<String>,
}

#[derive(Template)]
#[template(path = "hub/results.html")]
pub struct ResultsTemplate {
    shell: Shell,
    has_profile: bool,
    /// (value, is the selected one, is the current semester) per option.
    semester_opts: Vec<(i32, bool, bool)>,
    semester_label: String,
    courses: Vec<ResultCourse>,
    any_marks: bool,
    total_label: String,
}

pub async fn results(
    State(s): State<AppState>,
    session: Session,
    StudentOnly(user): StudentOnly,
    Query(q): Query<ResultsQuery>,
) -> Result<ResultsTemplate, AppError> {
    let shell = Shell::build(&user, &session).await?;
    let Some(p) = hub::student_profile(&s.db, user.id).await? else {
        return Ok(ResultsTemplate {
            shell,
            has_profile: false,
            semester_opts: Vec::new(),
            semester_label: "Semester 1".into(),
            courses: Vec::new(),
            any_marks: false,
            total_label: "—".into(),
        });
    };

    let mut semesters = exams::result_semesters(&s.db, p.id).await?;
    if !semesters.contains(&p.semester) {
        semesters.push(p.semester);
        semesters.sort_unstable();
    }
    // Defaults to the current semester; any past semester stays selectable.
    let selected = q
        .sem
        .as_deref()
        .and_then(|v| v.parse::<i32>().ok())
        .filter(|v| semesters.contains(v))
        .unwrap_or(p.semester);

    let courses = exams::results_for(&s.db, p.id, selected).await?;
    let any_marks = courses.iter().any(|c| c.has_marks);
    // The coursework total: internal, assignment and practical marks, without
    // the internal exam or the university exam.
    let obtained: f64 = courses
        .iter()
        .flat_map(|c| c.marks.iter())
        .filter(|m| m.is_coursework)
        .map(|m| m.obtained_label.parse::<f64>().unwrap_or(0.0))
        .sum();
    let maximum: f64 = courses
        .iter()
        .flat_map(|c| c.marks.iter())
        .filter(|m| m.is_coursework)
        .map(|m| m.max_label.parse::<f64>().unwrap_or(0.0))
        .sum();
    let total_label = if maximum > 0.0 {
        format!("{obtained:.0} / {maximum:.0} coursework")
    } else {
        "—".into()
    };

    let semester_opts = semesters.iter().map(|n| (*n, *n == selected, *n == p.semester)).collect();

    Ok(ResultsTemplate {
        shell,
        has_profile: true,
        semester_opts,
        semester_label: format!("Semester {selected}"),
        courses,
        any_marks,
        total_label,
    })
}

// ---------- Exam timetable ----------

#[derive(Template)]
#[template(path = "hub/exams.html")]
pub struct ExamTimetableTemplate {
    shell: Shell,
    has_profile: bool,
    entries: Vec<StudentExam>,
}

pub async fn exam_timetable(
    State(s): State<AppState>,
    session: Session,
    StudentOnly(user): StudentOnly,
) -> Result<ExamTimetableTemplate, AppError> {
    let shell = Shell::build(&user, &session).await?;
    let Some(p) = hub::student_profile(&s.db, user.id).await? else {
        return Ok(ExamTimetableTemplate {
            shell,
            has_profile: false,
            entries: Vec::new(),
        });
    };
    Ok(ExamTimetableTemplate {
        shell,
        has_profile: true,
        entries: exams::student_exams(&s.db, p.id).await?,
    })
}
