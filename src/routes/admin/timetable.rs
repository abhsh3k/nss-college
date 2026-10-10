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
        academics::{self, CourseRow, NewOfferingSlot, NewSlot, ProgrammeOption, SlotRow, TeacherOption},
        courses,
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
    /// Whether any row in `days` is a course-offering period, so the grid can
    /// explain why those rows have no Remove button.
    pub has_offering_periods: bool,
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
            has_offering_periods: false,
            notice: None,
            error: None,
        }
    }
}

async fn build_panel(
    s: &AppState,
    department: Option<i64>,
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
    let has_offering_periods = days.iter().any(|d| d.slots.iter().any(|s| s.is_offering));
    Ok(Panel {
        csrf_token,
        programme_id,
        semester,
        programme_name: programme.name,
        days,
        courses,
        teachers: academics::teacher_options_for(&s.db, department).await?,
        has_offering_periods,
        notice,
        error,
    })
}

// ---------- Offering panel: the same builder for a course offering ----------

/// Everything the offering timetable panel shows. `offering_id == 0` means
/// nothing is selected yet.
pub struct OfferingPanel {
    pub csrf_token: String,
    pub offering_id: i64,
    pub code: String,
    pub title: String,
    pub year_label: String,
    pub semester: i32,
    pub department: String,
    pub days: Vec<Day>,
    pub teachers: Vec<TeacherOption>,
    pub notice: Option<String>,
    pub error: Option<String>,
}

impl OfferingPanel {
    fn empty() -> OfferingPanel {
        OfferingPanel {
            csrf_token: String::new(),
            offering_id: 0,
            code: String::new(),
            title: String::new(),
            year_label: String::new(),
            semester: 0,
            department: String::new(),
            days: Vec::new(),
            teachers: Vec::new(),
            notice: None,
            error: None,
        }
    }
}

/// Whether this manager may schedule periods for this offering: it must be
/// published, and belong to their department (the IT admin reaches all).
async fn may_manage_offering(s: &AppState, manager: &Manager, offering_id: i64) -> Result<bool, AppError> {
    let Some(o) = courses::offering(&s.db, offering_id).await? else {
        return Ok(false);
    };
    Ok(o.status == "published"
        && (manager.is_admin() || manager.department() == Some(o.offering_department_id)))
}

async fn build_offering_panel(
    s: &AppState,
    manager: &Manager,
    csrf_token: String,
    offering_id: i64,
    notice: Option<String>,
    error: Option<String>,
) -> Result<OfferingPanel, AppError> {
    if !may_manage_offering(s, manager, offering_id).await? {
        return Err(AppError::Forbidden);
    }
    let o = courses::offering(&s.db, offering_id).await?.ok_or(AppError::NotFound)?;
    let slots = academics::offering_slots(&s.db, offering_id).await?;
    let mut days: Vec<Day> = DAYS
        .iter()
        .map(|(number, name)| Day { number: *number, name: *name, slots: Vec::new() })
        .collect();
    for slot in slots {
        if let Some(day) = days.iter_mut().find(|d| d.number == slot.weekday) {
            day.slots.push(slot);
        }
    }
    Ok(OfferingPanel {
        csrf_token,
        offering_id,
        code: o.code,
        title: o.title,
        year_label: o.year_label,
        semester: o.semester,
        department: o.department,
        days,
        teachers: academics::teacher_options_for(&s.db, manager.department()).await?,
        notice,
        error,
    })
}

// ---------- Page ----------

#[derive(Deserialize)]
pub struct PageQuery {
    programme: Option<String>,
    semester: Option<String>,
    offering: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/timetable.html")]
pub struct TimetablePage {
    shell: Shell,
    programmes: Vec<ProgrammeOption>,
    /// Published offerings this manager may schedule, for the offering picker.
    offerings: Vec<courses::SchedulableOffering>,
    sel_programme: i64,
    sel_semester: i32,
    panel: Panel,
    offering_panel: OfferingPanel,
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

    // `?offering=` selects the offering panel; a stale or forbidden id falls
    // back to the programme picker rather than failing the whole page.
    let sel_offering = q.offering.as_deref().and_then(parse_i64).unwrap_or(0);
    let offering_panel = match sel_offering {
        0 => OfferingPanel::empty(),
        id => {
            match build_offering_panel(&s, &manager, shell.csrf_token.clone(), id, None, None).await {
                Ok(p) => p,
                Err(AppError::NotFound) | Err(AppError::Forbidden) => OfferingPanel::empty(),
                Err(e) => return Err(e),
            }
        }
    };

    let panel = if offering_panel.offering_id != 0 {
        // One panel at a time: an offering's periods are not a programme's.
        Panel::empty()
    } else if sel_programme > 0 {
        build_panel(&s, department, shell.csrf_token.clone(), sel_programme, sel_semester, None, None).await?
    } else {
        Panel::empty()
    };

    Ok(TimetablePage {
        shell,
        offerings: courses::schedulable_offerings(&s.db, department).await?,
        programmes: academics::programme_options_for(&s.db, department).await?,
        sel_programme,
        sel_semester,
        panel,
        offering_panel,
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

    let fail = |msg: &str| build_panel(&s, manager.department(), token.clone(), programme_id, semester, None, Some(msg.to_string()));

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
    if faculty_id <= 0
        || !academics::may_manage_teacher(&s.db, faculty_id, manager.department()).await?
        || !sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM faculty WHERE id = $1 AND status = 'published')",
        )
        .bind(faculty_id)
        .fetch_one(&s.db)
        .await?
    {
        return Ok(PanelTemplate {
            panel: fail("Choose an active teacher in the permitted department.").await?,
        });
    }

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
        panel: build_panel(&s, manager.department(), token, programme_id, semester, Some(notice), None).await?,
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
                panel: build_panel(&s, manager.department(), f.csrf_token, programme_id, semester, Some("Period removed.".into()), None).await?,
            })
        }
        Err(e) if crate::error::is_foreign_key_violation(&e) => Ok(PanelTemplate {
            panel: build_panel(
                &s,
                manager.department(),
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

// ---------- Add / remove a period on a course offering ----------

#[derive(Template)]
#[template(path = "partials/timetable_offering_panel.html")]
pub struct OfferingPanelTemplate {
    offering_panel: OfferingPanel,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct OfferingSlotForm {
    csrf_token: String,
    offering_id: String,
    weekday: String,
    start_at: String,
    end_at: String,
    faculty_id: String,
    room: String,
}

pub async fn add_offering_slot(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Form(f): Form<OfferingSlotForm>,
) -> Result<OfferingPanelTemplate, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let offering_id = parse_i64(&f.offering_id).ok_or(AppError::NotFound)?;
    if !may_manage_offering(&s, &manager, offering_id).await? {
        return Err(AppError::Forbidden);
    }
    let token = f.csrf_token.clone();
    let fail = |msg: &str| {
        build_offering_panel(&s, &manager, token.clone(), offering_id, None, Some(msg.to_string()))
    };

    let weekday = parse_i32(&f.weekday).filter(|d| (1..=6).contains(d)).map(|d| d as i16);
    let start_at = normalise_time(&f.start_at);
    let end_at = normalise_time(&f.end_at);
    let (Some(weekday), Some(start_at), Some(end_at)) = (weekday, start_at, end_at) else {
        return Ok(OfferingPanelTemplate {
            offering_panel: fail("Choose the day, start and end time.").await?,
        });
    };
    if end_at <= start_at {
        return Ok(OfferingPanelTemplate {
            offering_panel: fail("The period must end after it starts.").await?,
        });
    }

    // The course's own teacher unless a different one was chosen for this period.
    let o = courses::offering(&s.db, offering_id).await?.ok_or(AppError::NotFound)?;
    let faculty_id = match parse_i64(&f.faculty_id).filter(|v| *v > 0) {
        Some(id) => id,
        None => academics::course_teacher(&s.db, o.course_id).await?,
    };
    if faculty_id <= 0
        || !academics::may_manage_teacher(&s.db, faculty_id, manager.department()).await?
        || !sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM faculty WHERE id = $1 AND status = 'published')",
        )
        .bind(faculty_id)
        .fetch_one(&s.db)
        .await?
    {
        return Ok(OfferingPanelTemplate {
            offering_panel: fail("Choose an active teacher in the permitted department.").await?,
        });
    }

    let slot = NewOfferingSlot {
        offering_id,
        faculty_id,
        weekday,
        start_at: start_at.clone(),
        end_at: end_at.clone(),
        room: f.room.trim().to_string(),
    };

    let clashes = academics::offering_clashes(&s.db, &slot).await?;
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
        return Ok(OfferingPanelTemplate {
            offering_panel: fail(&format!("Clash: {}.", reasons.join("; "))).await?,
        });
    }

    let id = academics::add_offering_slot(&s.db, &slot).await?;
    users::audit(
        &s.db,
        Some(manager.user.id),
        "offering_period_added",
        "timetable_entry",
        Some(id),
    )
    .await?;
    let notice = format!("Added {start_at}–{end_at}.");
    Ok(OfferingPanelTemplate {
        offering_panel: build_offering_panel(&s, &manager, token, offering_id, Some(notice), None).await?,
    })
}

pub async fn delete_offering_slot(
    State(s): State<AppState>,
    session: Session,
    manager: Manager,
    Path(id): Path<i64>,
    Form(f): Form<OfferingSlotForm>,
) -> Result<OfferingPanelTemplate, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    // The offering comes from the period itself, never from the form.
    let offering_id: Option<i64> =
        sqlx::query_scalar("SELECT course_offering_id FROM timetable_entries WHERE id = $1")
            .bind(id)
            .fetch_optional(&s.db)
            .await?;
    let Some(offering_id) = offering_id else {
        return Err(AppError::NotFound);
    };
    if !may_manage_offering(&s, &manager, offering_id).await? {
        return Err(AppError::Forbidden);
    }
    let deleted = match sqlx::query(
        "DELETE FROM timetable_entries WHERE id = $1 AND course_offering_id = $2",
    )
    .bind(id)
    .bind(offering_id)
    .execute(&s.db)
    .await
    {
        Ok(d) => d,
        // Attendance already taken for this period: the row must stay, and the
        // teacher is told why instead of being shown a 500.
        Err(e) if crate::error::is_foreign_key_violation(&e) => {
            return Ok(OfferingPanelTemplate {
                offering_panel: build_offering_panel(
                    &s,
                    &manager,
                    f.csrf_token,
                    offering_id,
                    None,
                    Some(
                        "Attendance has already been taken for this period, so it can't be removed."
                            .into(),
                    ),
                )
                .await?,
            });
        }
        Err(e) => return Err(e.into()),
    };
    if deleted.rows_affected() > 0 {
        users::audit(
            &s.db,
            Some(manager.user.id),
            "offering_period_removed",
            "timetable_entry",
            Some(id),
        )
        .await?;
    }
    Ok(OfferingPanelTemplate {
        offering_panel: build_offering_panel(
            &s,
            &manager,
            f.csrf_token,
            offering_id,
            Some("Period removed.".into()),
            None,
        )
        .await?,
    })
}
