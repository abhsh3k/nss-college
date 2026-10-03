use askama::Template;
use axum::extract::State;
use sqlx::types::time::OffsetDateTime;
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

pub struct TeacherClassItem {
    pub timetable_entry_id: i64,
    pub course_id: i64,
    pub course_code: String,
    pub course_title: String,
    pub start_time: String,
    pub end_time: String,
    pub room: String,
    pub is_substitution: bool,
    pub is_marked: bool,
}

#[derive(Template)]
#[template(path = "dashboard/teacher.html")]
pub struct TeacherTemplate {
    shell: Shell,
    today_str: String,
    classes: Vec<TeacherClassItem>,
}

pub async fn teacher(
    State(s): State<AppState>,
    session: Session,
    TeacherOnly(user): TeacherOnly,
) -> Result<TeacherTemplate, AppError> {
    let now = OffsetDateTime::now_utc();
    let today = now.date();
    let today_str = today.to_string();

    // Directly returns Monday=1 .. Sunday=7 as i16
    let weekday_num = today.weekday().number_from_monday() as i16;

    // Fetch faculty ID linked to logged-in user
    let faculty = sqlx::query!(
        "SELECT id FROM faculty WHERE user_id = $1",
        user.id
    )
    .fetch_optional(&s.db)
    .await?;

    let classes = if let Some(fac) = faculty {
        let rows = sqlx::query!(
            r#"
            SELECT 
                te.id AS timetable_entry_id,
                c.id AS course_id,
                c.code AS course_code,
                c.title AS course_title,
                te.start_time::text AS "start_time!",
                te.end_time::text AS "end_time!",
                COALESCE(te.room, '') AS "room!",
                COALESCE(s.id IS NOT NULL, false) AS "is_substitution!",
                COALESCE(att.id IS NOT NULL, false) AS "is_marked!"
            FROM timetable_entries te
            JOIN courses c ON c.id = te.course_id
            LEFT JOIN substitutions s 
                   ON s.timetable_entry_id = te.id 
                  AND s.on_date = $2
            LEFT JOIN attendance_sessions att 
                   ON att.timetable_entry_id = te.id 
                  AND att.on_date = $2
            WHERE ((te.faculty_id = $1 AND te.weekday = $3 AND s.id IS NULL) 
               OR s.substitute_faculty_id = $1)
            ORDER BY te.start_time ASC
            "#,
            fac.id,
            today,
            weekday_num
        )
        .fetch_all(&s.db)
        .await?;

        rows.into_iter()
            .map(|r| TeacherClassItem {
                timetable_entry_id: r.timetable_entry_id,
                course_id: r.course_id,
                course_code: r.course_code,
                course_title: r.course_title,
                start_time: r.start_time,
                end_time: r.end_time,
                room: r.room,
                is_substitution: r.is_substitution,
                is_marked: r.is_marked,
            })
            .collect()
    } else {
        Vec::new()
    };

    Ok(TeacherTemplate {
        shell: Shell::build(&user, &session).await?,
        today_str,
        classes,
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