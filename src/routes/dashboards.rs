use askama::Template;
use axum::extract::State;
use tower_sessions::Session;

use crate::{
    auth::{OfficeOrAdmin, Role, StudentOnly, TeacherOnly},
    error::AppError,
    services::users::{self, Counts},
    shell::Shell,
    state::AppState,
};

#[derive(Template)]
#[template(path = "dashboard/admin.html")]
pub struct AdminTemplate {
    shell: Shell,
    is_admin: bool,
    counts: Counts,
}

pub async fn admin(
    State(s): State<AppState>,
    session: Session,
    OfficeOrAdmin(user): OfficeOrAdmin,
) -> Result<AdminTemplate, AppError> {
    Ok(AdminTemplate {
        shell: Shell::build(&user, &session).await?,
        is_admin: user.role == Role::Admin,
        counts: users::overview_counts(&s.db).await?,
    })
}

#[derive(Template)]
#[template(path = "dashboard/teacher.html")]
pub struct TeacherTemplate {
    shell: Shell,
}

pub async fn teacher(session: Session, TeacherOnly(user): TeacherOnly) -> Result<TeacherTemplate, AppError> {
    Ok(TeacherTemplate {
        shell: Shell::build(&user, &session).await?,
    })
}

#[derive(Template)]
#[template(path = "dashboard/student.html")]
pub struct StudentTemplate {
    shell: Shell,
}

pub async fn student(session: Session, StudentOnly(user): StudentOnly) -> Result<StudentTemplate, AppError> {
    Ok(StudentTemplate {
        shell: Shell::build(&user, &session).await?,
    })
}
