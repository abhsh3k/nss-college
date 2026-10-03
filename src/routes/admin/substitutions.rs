use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::Redirect,
    Form,
};
use serde::Deserialize;
use tower_sessions::Session;

use super::parse_i64;
use crate::{
    auth::{csrf, AdminOnly},
    error::{is_foreign_key_violation, AppError},
    services::{
        academics::{self, TeacherOption},
        attendance::{self, CoverCandidate},
        users,
    },
    shell::{self, Shell},
    state::AppState,
};

#[derive(Deserialize)]
pub struct PageQuery {
    date: Option<String>,
    teacher: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/substitutions.html")]
pub struct SubstitutionsTemplate {
    shell: Shell,
    date: String,
    teacher_id: i64,
    teachers: Vec<TeacherOption>,
    candidates: Vec<CoverCandidate>,
    window_days: i32,
}

pub async fn page(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Query(q): Query<PageQuery>,
) -> Result<SubstitutionsTemplate, AppError> {
    let rules = attendance::rules(&s.db).await?;
    let date = match q.date.filter(|d| attendance::valid_date(d)) {
        Some(d) => d,
        None => attendance::today(&s.db).await?,
    };
    let teacher_id = q.teacher.as_deref().and_then(parse_i64).unwrap_or(0);
    let candidates = if teacher_id > 0 {
        attendance::teacher_day(&s.db, teacher_id, &date).await?
    } else {
        Vec::new()
    };
    Ok(SubstitutionsTemplate {
        shell: Shell::build(&user, &session).await?,
        date,
        teacher_id,
        teachers: academics::teacher_options(&s.db).await?,
        candidates,
        window_days: rules.edit_window_days,
    })
}

#[derive(Deserialize)]
pub struct SetForm {
    csrf_token: String,
    date: String,
    teacher: String,
    entry_id: String,
    substitute_id: String,
    #[serde(default)]
    reason: String,
}

fn back(date: &str, teacher: i64) -> String {
    format!("/admin/substitutions?date={date}&teacher={teacher}")
}

pub async fn set(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Form(f): Form<SetForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    if !attendance::valid_date(&f.date) {
        return Err(AppError::NotFound);
    }
    let teacher = parse_i64(&f.teacher).ok_or(AppError::NotFound)?;
    let target = back(&f.date, teacher);

    let (Some(entry_id), Some(substitute)) = (parse_i64(&f.entry_id), parse_i64(&f.substitute_id).filter(|v| *v > 0)) else {
        shell::flash(&session, "Choose a substitute teacher for the period.").await?;
        return Ok(Redirect::to(&target));
    };
    if attendance::entry_teacher(&s.db, entry_id).await? != Some(teacher) {
        shell::flash(&session, "That period does not belong to the selected teacher.").await?;
        return Ok(Redirect::to(&target));
    }
    if substitute == teacher {
        shell::flash(&session, "The substitute must be a different teacher.").await?;
        return Ok(Redirect::to(&target));
    }

    let rules = attendance::rules(&s.db).await?;
    let earliest = attendance::window_start(&s.db, rules.edit_window_days).await?;
    if f.date < earliest {
        shell::flash(&session, format!("Substitutions can't be set for dates older than {} days.", rules.edit_window_days)).await?;
        return Ok(Redirect::to(&target));
    }
    if attendance::faculty_busy(&s.db, substitute, entry_id, &f.date).await? {
        shell::flash(&session, "That teacher already has a class (or another cover) at an overlapping time.").await?;
        return Ok(Redirect::to(&target));
    }

    match attendance::set_substitution(&s.db, entry_id, &f.date, substitute, f.reason.trim(), user.id).await {
        Ok(()) => {
            users::audit(&s.db, Some(user.id), "substitution_set", "timetable_entry", Some(entry_id)).await?;
            shell::flash(&session, "Substitute assigned. They can now take attendance for that period.").await?;
        }
        Err(e) if is_foreign_key_violation(&e) => {
            shell::flash(&session, "That teacher could not be found.").await?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(Redirect::to(&target))
}

#[derive(Deserialize)]
pub struct RemoveForm {
    csrf_token: String,
    date: String,
    teacher: String,
}

pub async fn remove(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
    Form(f): Form<RemoveForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    if !attendance::valid_date(&f.date) {
        return Err(AppError::NotFound);
    }
    let teacher = parse_i64(&f.teacher).ok_or(AppError::NotFound)?;
    attendance::remove_substitution(&s.db, id).await?;
    users::audit(&s.db, Some(user.id), "substitution_removed", "substitution", Some(id)).await?;
    shell::flash(&session, "Substitution removed. The original teacher is responsible for the period again.").await?;
    Ok(Redirect::to(&back(&f.date, teacher)))
}
