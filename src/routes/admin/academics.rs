use askama::Template;
use axum::{
    extract::{Path, State},
    response::Redirect,
    Form,
};
use serde::Deserialize;
use tower_sessions::Session;

use super::{parse_i32, parse_i64};
use crate::{
    auth::{csrf, AdminOnly},
    error::{is_foreign_key_violation, is_unique_violation, AppError},
    services::{
        academics::{self, CourseDetail, CourseInput, CourseRow, ProgrammeOption, ProgrammeSummary, TeacherOption},
        users,
    },
    shell::{self, Shell},
    state::AppState,
};

// ---------- Programme list ----------

#[derive(Template)]
#[template(path = "admin/academics.html")]
pub struct AcademicsTemplate {
    shell: Shell,
    programmes: Vec<ProgrammeSummary>,
}

pub async fn index(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
) -> Result<AcademicsTemplate, AppError> {
    Ok(AcademicsTemplate {
        shell: Shell::build(&user, &session).await?,
        programmes: academics::programme_summaries(&s.db).await?,
    })
}

// ---------- One programme: courses by semester ----------

pub struct SemesterView {
    pub number: i32,
    pub courses: Vec<CourseRow>,
}

#[derive(Template)]
#[template(path = "admin/programme.html")]
pub struct ProgrammeTemplate {
    shell: Shell,
    programme: ProgrammeOption,
    semesters: Vec<SemesterView>,
    teachers: Vec<TeacherOption>,
    semester_numbers: Vec<i32>,
}

pub async fn programme(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
) -> Result<ProgrammeTemplate, AppError> {
    let programme = academics::programme(&s.db, id).await?.ok_or(AppError::NotFound)?;
    let all = academics::courses(&s.db, id).await?;
    // Two-year programmes have four semesters; honours degrees have eight.
    let count = if programme.level.contains("2-year") { 4 } else { 8 };
    let semester_numbers: Vec<i32> = (1..=count).collect();
    let semesters = semester_numbers
        .iter()
        .map(|n| SemesterView {
            number: *n,
            courses: all.iter().filter(|c| c.semester == *n).cloned().collect(),
        })
        .collect();
    Ok(ProgrammeTemplate {
        shell: Shell::build(&user, &session).await?,
        programme,
        semesters,
        teachers: academics::teacher_options(&s.db).await?,
        semester_numbers,
    })
}

// ---------- Add / edit / delete a course ----------

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct CourseForm {
    csrf_token: String,
    semester: String,
    code: String,
    title: String,
    credits: String,
    faculty_id: String,
}

struct CheckedCourse {
    code: String,
    title: String,
    semester: i32,
    credits: i32,
    faculty_id: i64,
}

fn check_course(f: &CourseForm) -> Result<CheckedCourse, &'static str> {
    let code = f.code.trim().to_string();
    let title = f.title.trim().to_string();
    if code.is_empty() || title.is_empty() {
        return Err("Enter both the course code and the title.");
    }
    let semester = parse_i32(&f.semester).filter(|v| (1..=8).contains(v)).ok_or("Choose a semester from 1 to 8.")?;
    let credits = parse_i32(&f.credits).filter(|v| (0..=40).contains(v)).ok_or("Credits must be a number from 0 to 40.")?;
    Ok(CheckedCourse {
        code,
        title,
        semester,
        credits,
        faculty_id: parse_i64(&f.faculty_id).unwrap_or(0),
    })
}

pub async fn add_course(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(programme_id): Path<i64>,
    Form(f): Form<CourseForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let back = format!("/admin/academics/programmes/{programme_id}");
    academics::programme(&s.db, programme_id).await?.ok_or(AppError::NotFound)?;

    let c = match check_course(&f) {
        Ok(c) => c,
        Err(msg) => {
            shell::flash(&session, msg).await?;
            return Ok(Redirect::to(&back));
        }
    };
    let input = CourseInput { code: &c.code, title: &c.title, semester: c.semester, credits: c.credits, faculty_id: c.faculty_id };
    match academics::create_course(&s.db, programme_id, &input).await {
        Ok(id) => {
            users::audit(&s.db, Some(user.id), "course_created", "course", Some(id)).await?;
            shell::flash(&session, format!("Added {} {}.", c.code, c.title)).await?;
        }
        Err(e) if is_unique_violation(&e) => {
            shell::flash(&session, "That course code already exists in this programme.").await?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(Redirect::to(&back))
}

#[derive(Template)]
#[template(path = "admin/course_edit.html")]
pub struct CourseEditTemplate {
    shell: Shell,
    course: CourseDetail,
    teachers: Vec<TeacherOption>,
}

pub async fn course_form(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
) -> Result<CourseEditTemplate, AppError> {
    Ok(CourseEditTemplate {
        shell: Shell::build(&user, &session).await?,
        course: academics::course(&s.db, id).await?.ok_or(AppError::NotFound)?,
        teachers: academics::teacher_options(&s.db).await?,
    })
}

pub async fn course_update(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
    Form(f): Form<CourseForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let existing = academics::course(&s.db, id).await?.ok_or(AppError::NotFound)?;
    let back = format!("/admin/academics/programmes/{}", existing.programme_id);
    let edit = format!("/admin/academics/courses/{id}");

    let c = match check_course(&f) {
        Ok(c) => c,
        Err(msg) => {
            shell::flash(&session, msg).await?;
            return Ok(Redirect::to(&edit));
        }
    };
    let input = CourseInput { code: &c.code, title: &c.title, semester: c.semester, credits: c.credits, faculty_id: c.faculty_id };
    match academics::update_course(&s.db, id, &input).await {
        Ok(()) => {
            users::audit(&s.db, Some(user.id), "course_updated", "course", Some(id)).await?;
            shell::flash(&session, "Course saved. If you moved it to another semester, use \"Enroll students\" for that semester.").await?;
            Ok(Redirect::to(&back))
        }
        Err(e) if is_unique_violation(&e) => {
            shell::flash(&session, "That course code already exists in this programme.").await?;
            Ok(Redirect::to(&edit))
        }
        Err(e) => Err(e.into()),
    }
}

#[derive(Deserialize)]
pub struct TokenForm {
    csrf_token: String,
    #[serde(default)]
    semester: String,
}

pub async fn course_delete(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let existing = academics::course(&s.db, id).await?.ok_or(AppError::NotFound)?;
    let back = format!("/admin/academics/programmes/{}", existing.programme_id);
    match academics::delete_course(&s.db, id).await {
        Ok(()) => {
            users::audit(&s.db, Some(user.id), "course_deleted", "course", Some(id)).await?;
            shell::flash(&session, format!("Removed {} {}.", existing.code, existing.title)).await?;
        }
        Err(e) if is_foreign_key_violation(&e) => {
            shell::flash(&session, "This course already has attendance records, so it can't be deleted.").await?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(Redirect::to(&back))
}

pub async fn enroll(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(programme_id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let back = format!("/admin/academics/programmes/{programme_id}");
    let Some(semester) = parse_i32(&f.semester).filter(|v| (1..=8).contains(v)) else {
        shell::flash(&session, "Choose a semester.").await?;
        return Ok(Redirect::to(&back));
    };
    let added = academics::enroll_semester(&s.db, programme_id, semester).await?;
    users::audit(&s.db, Some(user.id), "semester_enrolled", "programme", Some(programme_id)).await?;
    shell::flash(
        &session,
        if added == 0 {
            format!("Semester {semester}: everyone is already enrolled in all its courses.")
        } else {
            format!("Semester {semester}: added {added} enrollment(s).")
        },
    )
    .await?;
    Ok(Redirect::to(&back))
}
