use askama::Template;
use axum::{
    extract::{Path, Query, State},
    Form,
};
use serde::Deserialize;
use tower_sessions::Session;

use super::{normalise_time, parse_i32, parse_i64};
use crate::{
    auth::{csrf, Manager},
    error::AppError,
    services::{
        academics::{self, CourseRow, NewSlot, ProgrammeOption, SlotRow, TeacherOption},
        users,
    },
    shell::Shell,
    state::AppState,
};

const DAYS: [(i16, &str); 6] = [
    (1, "Monday"),
    (2, "Tuesday"),
    (3, "Wednesday"),
    (4, "Thursday"),
    (5, "Friday"),
    (6, "Saturday"),
];

pub struct Day {
    pub number: i16,
    pub name: &'static str,
    pub slots: Vec<SlotRow>,
}

/// Everything the timetable panel shows. `programme_id == 0` means nothing is selected yet.
pub struct Panel {
    pub csrf_token: String,
    pub programme_id: i64,
    pub semester: i32,
    pub programme_name: String,
    pub days: Vec<Day>,
    pub courses: Vec<CourseRow>,
    pub teachers: Vec<TeacherOption>,
    pub notice: Option<String>,
    pub error: Option<String>,
}

impl Panel {
    fn empty() -> Panel {
        Panel {
            csrf_token: String::new(),
            programme_id: 0,
            semester: 1,
            programme_name: String::new(),
            days: Vec::new(),
            courses: Vec::new(),
            teachers: Vec::new(),
            notice: None,
            error: None,
        }
    }
}

async fn build_panel(
    s: &AppState,
    csrf_token: String,
    programme_id: i64,
    semester: i32,
    notice: Option<String>,
    error: Option<String>,
) -> Result<Panel, AppError> {
    let programme = academics::programme(&s.db, programme_id).await?.ok_or(AppError::NotFound)?;
    let slots = academics::slots(&s.db, programme_id, semester).await?;
    let days = DAYS
        .iter()
        .map(|(number, name)| Day {
            number: *number,
            name: *name,
            slots: Vec::new(),
        })
        .collect::<Vec<_>>();
    let mut days = days;
    for slot in slots {
        if let Some(day) = days.iter_mut().find(|d| d.number == slot.weekday) {
            day.slots.push(slot);
        }
    }
    let courses = academics::courses(&s.db, programme_id)
        .await?
        .into_iter()
        .filter(|c| c.semester == semester)
        .collect();
    Ok(Panel {
        csrf_token,
        programme_id,
        semester,
        programme_name: programme.name,
        days,
        courses,
        teachers: academics::teacher_options(&s.db).await?,
        notice,
        error,
    })
}

// ---------- Page ----------

#[derive(Deserialize)]
pub struct PageQuery {
    programme: Option<String>,
    semester: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/timetable.html")]
pub struct TimetablePage {
    shell: Shell,
    programmes: Vec<ProgrammeOption>,
    sel_programme: i64,
    sel_semester: i32,
    panel: Panel,
}

pub async fn page(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Query(q): Query<PageQuery>,
) -> Result<TimetablePage, AppError> {
    let shell = Shell::build(&manager.user, &session).await?;
    let department = manager.department();
    let sel_programme = q.programme.as_deref().and_then(parse_i64).unwrap_or(0);
    let sel_semester = q
        .semester
        .as_deref()
        .and_then(parse_i32)
        .filter(|v| (1..=8).contains(v))
        .unwrap_or(1);
    // A programme from another department is simply not selectable here.
    let sel_programme = match sel_programme {
        0 => 0,
        id if academics::may_manage_programme(&s.db, id, department).await? => id,
        _ => 0,
    };
    let panel = if sel_programme > 0 {
        build_panel(&s, shell.csrf_token.clone(), sel_programme, sel_semester, None, None).await?
    } else {
        Panel::empty()
    };
    Ok(TimetablePage {
        shell,
        programmes: academics::programme_options_for(&s.db, department).await?,
        sel_programme,
        sel_semester,
        panel,
    })
}

// ---------- Add / remove a period (HTMX: both return just the panel) ----------

#[derive(Template)]
#[template(path = "partials/timetable_panel.html")]
pub struct PanelTemplate {
    panel: Panel,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct SlotForm {
    csrf_token: String,
    programme_id: String,
    semester: String,
    weekday: String,
    start_at: String,
    end_at: String,
    course_id: String,
    faculty_id: String,
    room: String,
}

pub async fn add_slot(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Form(f): Form<SlotForm>,
) -> Result<PanelTemplate, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let programme_id = parse_i64(&f.programme_id).ok_or(AppError::NotFound)?;
    // HODs may only build a timetable for their own department.
    if !academics::may_manage_programme(&s.db, programme_id, manager.department()).await? {
        return Err(AppError::Forbidden);
    }
    let semester = parse_i32(&f.semester).filter(|v| (1..=8).contains(v)).ok_or(AppError::NotFound)?;
    let token = f.csrf_token.clone();

    let fail = |msg: &str| build_panel(&s, token.clone(), programme_id, semester, None, Some(msg.to_string()));

    let weekday = parse_i32(&f.weekday).filter(|d| (1..=6).contains(d)).map(|d| d as i16);
    let start_at = normalise_time(&f.start_at);
    let end_at = normalise_time(&f.end_at);
    let course_id = parse_i64(&f.course_id);
    let (Some(weekday), Some(start_at), Some(end_at), Some(course_id)) = (weekday, start_at, end_at, course_id) else {
        return Ok(PanelTemplate { panel: fail("Choose the day, start and end time, and the course.").await? });
    };
    if end_at <= start_at {
        return Ok(PanelTemplate { panel: fail("The period must end after it starts.").await? });
    }
    if !academics::course_in_class(&s.db, course_id, programme_id, semester).await? {
        return Ok(PanelTemplate { panel: fail("That course does not belong to this programme and semester.").await? });
    }

    // Use the course's teacher unless a different one was chosen for this period.
    let faculty_id = match parse_i64(&f.faculty_id).filter(|v| *v > 0) {
        Some(id) => id,
        None => academics::course_teacher(&s.db, course_id).await?,
    };

    let slot = NewSlot {
        programme_id,
        semester,
        course_id,
        faculty_id,
        weekday,
        start_at: start_at.clone(),
        end_at: end_at.clone(),
        room: f.room.trim().to_string(),
    };

    let clashes = academics::clashes(&s.db, &slot).await?;
    if !clashes.is_empty() {
        let reasons: Vec<String> = clashes
            .iter()
            .map(|c| {
                let who = if c.same_class {
                    "this class already has"
                } else if c.same_teacher {
                    "the teacher is already teaching"
                } else {
                    "the room is already used for"
                };
                format!("{who} {} ({}–{})", c.course, c.start_at, c.end_at)
            })
            .collect();
        return Ok(PanelTemplate {
            panel: fail(&format!("Clash: {}.", reasons.join("; "))).await?,
        });
    }

    let id = academics::add_slot(&s.db, &slot).await?;
    users::audit(&s.db, Some(manager.user.id), "timetable_slot_added", "timetable_entry", Some(id)).await?;
    let notice = format!("Added {start_at}–{end_at}.");
    Ok(PanelTemplate {
        panel: build_panel(&s, token, programme_id, semester, Some(notice), None).await?,
    })
}

#[derive(Deserialize)]
pub struct DeleteForm {
    csrf_token: String,
    programme_id: String,
    semester: String,
}

pub async fn delete_slot(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(f): Form<DeleteForm>,
) -> Result<PanelTemplate, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let programme_id = parse_i64(&f.programme_id).ok_or(AppError::NotFound)?;
    // The period being removed must belong to a programme this manager owns.
    if !academics::may_manage_programme(&s.db, programme_id, manager.department()).await? {
        return Err(AppError::Forbidden);
    }
    let slot_programme: Option<i64> =
        sqlx::query_scalar("SELECT programme_id FROM timetable_entries WHERE id = $1")
            .bind(id)
            .fetch_optional(&s.db)
            .await?;
    match slot_programme {
        Some(p) if p == programme_id => {}
        _ => return Err(AppError::Forbidden),
    }
    let semester = parse_i32(&f.semester).filter(|v| (1..=8).contains(v)).ok_or(AppError::NotFound)?;
    match academics::delete_slot(&s.db, id).await {
        Ok(()) => {
            users::audit(&s.db, Some(manager.user.id), "timetable_slot_removed", "timetable_entry", Some(id)).await?;
            Ok(PanelTemplate {
                panel: build_panel(&s, f.csrf_token, programme_id, semester, Some("Period removed.".into()), None).await?,
            })
        }
        Err(e) if crate::error::is_foreign_key_violation(&e) => Ok(PanelTemplate {
            panel: build_panel(
                &s,
                f.csrf_token,
                programme_id,
                semester,
                None,
                Some("Attendance has already been taken for this period, so it can't be removed.".into()),
            )
            .await?,
        }),
        Err(e) => Err(e.into()),
    }
}
