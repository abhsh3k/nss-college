//! The student's course-selection page: fixed courses, selectable offerings,
//! confirm, and change requests.

use askama::Template;
use axum::{
    extract::State,
    response::Redirect,
    Form,
};
use serde::Deserialize;
use tower_sessions::Session;

use crate::{
    auth::{csrf, StudentOnly},
    error::AppError,
    services::{courses, hub, users},
    shell::{self, Shell},
    state::AppState,
};

/// A selectable offering with the student's current state and what they can do.
pub struct ChoiceCourse {
    pub offering_id: i64,
    pub code: String,
    pub title: String,
    pub credits: i32,
    pub department: String,
    pub course_type: String,
    pub selection_mode: String,
    pub choice_group: String,
    pub state: String, // "", draft, submitted, confirmed, locked, change_requested
    pub is_confirmed: bool,
    pub is_locked: bool,
    pub is_change_requested: bool,
}

/// One pending change request of this student.
pub struct MyChangeRequest {
    pub current_code: String,
    pub new_code: String,
    pub reason: String,
    pub status: String,
}

#[derive(Template)]
#[template(path = "hub/courses.html")]
pub struct MyCoursesTemplate {
    shell: Shell,
    /// Programme, department, batch and semester — the structure behind it all.
    structure: Option<courses::StudentStructure>,
    fixed: Vec<courses::FixedCourse>,
    choices: Vec<ChoiceCourse>,
    requests: Vec<MyChangeRequest>,
    has_choices: bool,
}

pub async fn my_courses(
    State(s): State<AppState>,
    session: Session,
    StudentOnly(user): StudentOnly,
) -> Result<MyCoursesTemplate, AppError> {
    let shell = Shell::build(&user, &session).await?;
    let Some(p) = hub::student_profile(&s.db, user.id).await? else {
        return Ok(MyCoursesTemplate {
            shell,
            structure: None,
            fixed: Vec::new(),
            choices: Vec::new(),
            requests: Vec::new(),
            has_choices: false,
        });
    };

    // FIXED offerings apply themselves — including to a student who was
    // imported after the offering was published. Idempotent and scoped to
    // this one student, so opening the page can never touch anyone else.
    courses::auto_apply_fixed(&s.db, None, Some(p.id)).await?;

    let structure = courses::student_structure(&s.db, p.id).await?;

    // Fixed courses: enrolled, and not managed by a selection anywhere.
    let fixed = courses::fixed_courses(&s.db, p.id).await?;

    // Everything else that is open to them. FIXED offerings are left out on
    // purpose: they belong to the fixed list above and need no decision, so
    // no course is ever listed twice.
    let choices: Vec<ChoiceCourse> = courses::student_eligible_offerings(&s.db, p.id)
        .await?
        .into_iter()
        .filter(|o| o.selection_mode != "FIXED")
        .map(|o| ChoiceCourse {
            offering_id: o.id,
            code: o.code,
            title: o.title,
            credits: o.credits,
            department: o.department,
            course_type: o.course_type,
            selection_mode: o.selection_mode.clone(),
            choice_group: o.choice_group,
            is_confirmed: o.state == "confirmed" || o.state == "locked",
            is_locked: o.state == "locked",
            is_change_requested: o.state == "change_requested",
            state: o.state,
        })
        .collect();

    let requests = sqlx::query_as::<_, (i64, String, String, String, String)>(
        r#"SELECT r.id, cc.code, cn.code, r.reason, r.status
             FROM course_change_requests r
             JOIN course_offerings co ON co.id = r.offering_id
             JOIN courses cc ON cc.id = co.course_id
             JOIN course_offerings no_ ON no_.id = r.new_offering_id
             JOIN courses cn ON cn.id = no_.course_id
            WHERE r.student_id = $1
            ORDER BY r.created_at DESC
            LIMIT 20"#,
    )
    .bind(p.id)
    .fetch_all(&s.db)
    .await?
    .into_iter()
    .map(|(_, current_code, new_code, reason, status)| MyChangeRequest {
        current_code,
        new_code,
        reason,
        status,
    })
    .collect();

    let has_choices = !choices.is_empty();
    Ok(MyCoursesTemplate {
        shell,
        structure,
        fixed,
        choices,
        requests,
        has_choices,
    })
}

#[derive(Deserialize)]
pub struct SelectForm {
    csrf_token: String,
    offering_id: i64,
}

/// Add an offering to the student's sheet as a draft. Eligibility is
/// re-checked server-side in the service.
pub async fn select(
    State(s): State<AppState>,
    session: Session,
    StudentOnly(user): StudentOnly,
    Form(f): Form<SelectForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let Some(p) = hub::student_profile(&s.db, user.id).await? else {
        return Err(AppError::NotFound);
    };
    match courses::select_course(&s.db, p.id, f.offering_id).await {
        Ok(()) => {
            users::audit(&s.db, Some(user.id), "student_course_selected", "course_offering", Some(f.offering_id)).await?;
            shell::flash(&session, "Course added to your selection sheet.").await?
        }
        Err(AppError::Forbidden) => {
            shell::flash(&session, "That course is not open to you right now.").await?
        }
        Err(AppError::BadRequest(m)) => shell::flash(&session, m).await?,
        Err(e) => return Err(e),
    }
    Ok(Redirect::to("/hub/courses"))
}

pub async fn withdraw(
    State(s): State<AppState>,
    session: Session,
    StudentOnly(user): StudentOnly,
    Form(f): Form<SelectForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let Some(p) = hub::student_profile(&s.db, user.id).await? else {
        return Err(AppError::NotFound);
    };
    courses::withdraw_selection(&s.db, p.id, f.offering_id).await?;
    users::audit(&s.db, Some(user.id), "student_course_withdrawn", "course_offering", Some(f.offering_id)).await?;
    shell::flash(&session, "Removed from your selection sheet.").await?;
    Ok(Redirect::to("/hub/courses"))
}

/// Confirm the whole sheet: every draft becomes confirmed, the enrollments are
/// created, and timetable/attendance follow.
pub async fn confirm(
    State(s): State<AppState>,
    session: Session,
    StudentOnly(user): StudentOnly,
    Form(f): Form<SelectForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let Some(p) = hub::student_profile(&s.db, user.id).await? else {
        return Err(AppError::NotFound);
    };
    // The form's offering_id may be 0 meaning "confirm everything drafted".
    let targets: Vec<i64> = if f.offering_id > 0 {
        vec![f.offering_id]
    } else {
        courses::student_selections(&s.db, p.id)
            .await?
            .into_iter()
            .filter(|x| x.state == "draft" || x.state == "submitted")
            .map(|x| x.offering_id)
            .collect()
    };
    if targets.is_empty() {
        shell::flash(&session, "Nothing to confirm yet; select a course first.").await?;
        return Ok(Redirect::to("/hub/courses"));
    }
    let mut problems = 0;
    let mut reason: Option<String> = None;
    for offering_id in targets {
        if let Err(e) = courses::confirm_selection(&s.db, p.id, offering_id).await {
            problems += 1;
            if reason.is_none() {
                reason = match &e {
                    AppError::BadRequest(m) => Some(m.clone()),
                    _ => None,
                };
            }
            tracing::warn!(student = p.id, offering = offering_id, error = ?e, "confirm failed");
        }
    }
    if problems == 0 {
        users::audit(&s.db, Some(user.id), "student_course_selection_confirmed", "student", Some(p.id)).await?;
    }
    let hint = reason.map(|r| format!(" {r}")).unwrap_or_default();
    shell::flash(
        &session,
        if problems == 0 {
            "Your course selection is confirmed. Timetable and attendance now follow these courses."
        } else {
            "Some courses could not be confirmed. The rest are saved."
        }
        .to_string()
        + &hint,
    )
    .await?;
    Ok(Redirect::to("/hub/courses"))
}

#[derive(Deserialize)]
pub struct ChangeForm {
    csrf_token: String,
    offering_id: i64,
    /// 0 when the dropdown had no other course to move to (the form can be
    /// rendered with an empty list), handled as a friendly message rather
    /// than a deserialization error.
    #[serde(default)]
    new_offering_id: i64,
    reason: String,
}

/// Ask the HOD to move a confirmed course to another one.
pub async fn request_change(
    State(s): State<AppState>,
    session: Session,
    StudentOnly(user): StudentOnly,
    Form(f): Form<ChangeForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let Some(p) = hub::student_profile(&s.db, user.id).await? else {
        return Err(AppError::NotFound);
    };
    if f.new_offering_id <= 0 || f.new_offering_id == f.offering_id {
        shell::flash(&session, "Pick the course you would rather take.").await?;
        return Ok(Redirect::to("/hub/courses"));
    }
    let reason = f.reason.trim().to_string();
    if reason.is_empty() {
        shell::flash(&session, "Tell your HOD briefly why you want the change.").await?;
        return Ok(Redirect::to("/hub/courses"));
    }
    match courses::create_change_request(
        &s.db,
        &courses::NewChangeRequest {
            student_id: p.id,
            offering_id: f.offering_id,
            new_offering_id: f.new_offering_id,
            reason,
        },
    )
    .await
    {
        Ok(()) => {
            users::audit(&s.db, Some(user.id), "student_course_change_requested", "course_offering", Some(f.offering_id)).await?;
            shell::flash(&session, "Change request sent to your HOD.").await?
        }
        Err(AppError::BadRequest(m)) => shell::flash(&session, m).await?,
        Err(AppError::Forbidden) => {
            shell::flash(&session, "That change is not possible right now.").await?
        }
        Err(e) => return Err(e),
    }
    Ok(Redirect::to("/hub/courses"))
}
