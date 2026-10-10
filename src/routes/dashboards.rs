use std::collections::HashMap;

use askama::Template;
use axum::extract::State;
use sqlx::{types::time::OffsetDateTime, FromRow};
use tower_sessions::Session;

use crate::{
    auth::{OfficeOrAdmin, Role, StudentOnly, TeacherOnly},
    error::AppError,
    models::Notice,
    sections::{self, Area},
    services::{
        exams::{self, ExamNotice},
        hub,
        users::{self, Counts},
    },
    shell::Shell,
    state::AppState,
};

#[derive(Template)]
#[template(path = "dashboard/admin.html")]
pub struct AdminTemplate {
    shell: Shell,
    is_admin: bool,
    counts: Counts,
    /// The management-area cards. Office staff only ever see the content area,
    /// because that is all they are guarded for.
    areas: &'static [Area],
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
        areas: if user.role == Role::Admin {
            sections::ADMIN
        } else {
            sections::STAFF
        },
    })
}

/// Renders the admin overview with a hand-built sidebar, so a test can read
/// the HTML the layout actually produces.
#[cfg(test)]
pub(crate) fn tests_render_shell(nav: Vec<crate::shell::NavGroup>) -> String {
    let mut shell = crate::sections::test_shell("IT administrator");
    shell.nav = nav;
    AdminTemplate {
        shell,
        is_admin: true,
        counts: Counts { students: 0, teachers: 0, programmes: 0, news: 0 },
        areas: sections::ADMIN,
    }
    .render()
    .expect("the overview renders")
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

/// One class in today's timeline, straight from the query.
#[derive(FromRow)]
struct TodayClassRow {
    timetable_entry_id: i64,
    course_id: i64,
    course_code: String,
    course_title: String,
    start_time: String,
    end_time: String,
    room: String,
    is_substitution: bool,
    is_marked: bool,
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
    let faculty_id: Option<i64> = sqlx::query_scalar("SELECT id FROM faculty WHERE user_id = $1")
        .bind(user.id)
        .fetch_optional(&s.db)
        .await?;

    let classes = if let Some(faculty_id) = faculty_id {
        let rows = sqlx::query_as::<_, TodayClassRow>(
            r#"
            SELECT
                te.id AS timetable_entry_id,
                c.id AS course_id,
                c.code AS course_code,
                c.title AS course_title,
                te.start_time::text AS start_time,
                te.end_time::text AS end_time,
                COALESCE(te.room, '') AS room,
                COALESCE(s.id IS NOT NULL, false) AS is_substitution,
                COALESCE(att.id IS NOT NULL, false) AS is_marked
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
        )
        .bind(faculty_id)
        .bind(today)
        .bind(weekday_num)
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

// ---------- Student Hub (Layer 4d) ----------

/// Programme details shown under the welcome heading.
pub struct HubProfile {
    pub programme: String,
    pub semester_label: String,
    pub admission_no: String,
    /// The university's permanent registration number, once assigned.
    pub prn: Option<String>,
}

/// One class in today's timeline.
pub struct HubPeriod {
    pub code: String,
    pub title: String,
    pub start_time: String,
    pub end_time: String,
    pub detail: String, // "Room 12 · Dr. Name"
    pub is_now: bool,
    pub is_past: bool,
}

/// One day column of the weekly matrix.
pub struct HubDay {
    pub num: i16,
    pub short: &'static str,
    pub is_today: bool,
}

/// One timetable cell in the weekly matrix (None = free period).
#[derive(Clone)]
pub struct HubCell {
    pub code: String,
    pub title: String,
    pub detail: String,
    pub is_today: bool,
}

/// One row of the weekly matrix: a start time plus a cell per day column.
pub struct HubRow {
    pub time: String,
    pub cells: Vec<Option<HubCell>>,
}

/// A course card: enrolment details plus the live attendance percentage.
pub struct HubCourse {
    pub code: String,
    pub title: String,
    pub credits: i32,
    pub teacher: String,
    pub marked: i64,
    pub attended: i64,
    pub percent_label: String,
    pub bar_width: String,
    pub is_low: bool,
}

/// The student overview: profile, the exam banner while a timetable is
/// pushed, today's classes, attendance and notices.
/// Timetable, results and the exam timetable have their own pages.
#[derive(Template)]
#[template(path = "dashboard/student.html")]
pub struct StudentTemplate {
    shell: Shell,
    today_str: String,
    feed: Vec<Notice>,
    profile: Option<HubProfile>,
    /// Open only while an exam timetable has been pushed to this student.
    exam_notice: Option<ExamNotice>,
    min_percent: i32,
    today_periods: Vec<HubPeriod>,
    courses: Vec<HubCourse>,
    total_credits: i32,
}

/// "Room 12 · Dr. Name", with sensible fallbacks for missing data.
pub(crate) fn period_detail(room: &str, faculty: &str) -> String {
    let room = if room.is_empty() { "Room TBA" } else { room };
    if faculty.is_empty() {
        room.to_string()
    } else {
        format!("{room} · {faculty}")
    }
}

/// Today's timeline out of one week's query, shared by the overview and the
/// timetable page.
pub(crate) fn today_periods(week: &[hub::Period], weekday_today: i16, now_hm: &str) -> Vec<HubPeriod> {
    week
        .iter()
        .filter(|x| x.weekday == weekday_today)
        .map(|x| HubPeriod {
            code: x.code.clone(),
            title: x.title.clone(),
            start_time: x.start_time.clone(),
            end_time: x.end_time.clone(),
            detail: period_detail(&x.room, &x.faculty),
            is_now: x.start_time.as_str() <= now_hm && now_hm < x.end_time.as_str(),
            is_past: now_hm >= x.end_time.as_str(),
        })
        .collect()
}

pub async fn student(
    State(s): State<AppState>,
    session: Session,
    StudentOnly(user): StudentOnly,
) -> Result<StudentTemplate, AppError> {
    let shell = Shell::build(&user, &session).await?;
    let feed = hub::student_feed(&s.db, 5).await?;

    let now = OffsetDateTime::now_utc();
    let today = now.date();
    let today_str = today.to_string();
    let now_hm = format!("{:02}:{:02}", now.hour(), now.minute());
    let weekday_today = today.weekday().number_from_monday() as i16;

    // Without a student row there is nothing to show yet (account just created).
    let Some(p) = hub::student_profile(&s.db, user.id).await? else {
        return Ok(StudentTemplate {
            shell,
            today_str,
            feed,
            profile: None,
            exam_notice: None,
            min_percent: 75,
            today_periods: Vec::new(),
            courses: Vec::new(),
            total_credits: 0,
        });
    };

    let min_percent = hub::setting(&s.db, "attendance_min_percent", 75).await?;

    // Today's timeline from the week's timetable. Offering periods the student
    // is enrolled in come along with their programme's class periods.
    let week = hub::week_schedule(&s.db, p.id, p.programme_id, p.semester).await?;
    let today_periods = today_periods(&week, weekday_today, &now_hm);

    // The exam timetable card appears only while the push window is open.
    let exam_notice = exams::live_exam_notice(&s.db, p.id).await?;

    // Enrolled courses with live attendance percentages.
    let attendance: HashMap<i64, hub::CourseAttendance> = hub::course_attendance(&s.db, p.id)
        .await?
        .into_iter()
        .map(|a| (a.course_id, a))
        .collect();
    let mut total_credits = 0;
    let courses: Vec<HubCourse> = hub::enrolled_courses(&s.db, p.id)
        .await?
        .into_iter()
        .map(|c| {
            total_credits += c.credits;
            let (marked, attended) = attendance
                .get(&c.course_id)
                .map(|a| (a.marked, a.attended))
                .unwrap_or((0, 0));
            let percent = if marked > 0 {
                100.0 * attended as f64 / marked as f64
            } else {
                0.0
            };
            HubCourse {
                code: c.code,
                title: c.title,
                credits: c.credits,
                teacher: if c.teacher.is_empty() {
                    "Not assigned yet".into()
                } else {
                    c.teacher
                },
                marked,
                attended,
                percent_label: if marked == 0 {
                    "—".into()
                } else {
                    format!("{percent:.0}%")
                },
                bar_width: format!("{}%", percent.round() as i32),
                is_low: marked > 0 && percent < min_percent as f64,
            }
        })
        .collect();

    // Results moved to their own page (/hub/results) with a semester picker.

    Ok(StudentTemplate {
        shell,
        today_str,
        feed,
        profile: Some(HubProfile {
            programme: p.programme,
            semester_label: format!("Semester {}", p.semester),
            admission_no: p.admission_no,
            prn: if p.prn.is_empty() { None } else { Some(p.prn) },
        }),
        exam_notice,
        min_percent,
        today_periods,
        courses,
        total_credits,
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    fn overview(is_admin: bool, areas: &'static [crate::sections::Area]) -> String {
        AdminTemplate {
            shell: crate::sections::test_shell(if is_admin { "IT administrator" } else { "Office staff" }),
            is_admin,
            counts: Counts { students: 3, teachers: 4, programmes: 5, news: 6 },
            areas,
        }
        .render()
        .expect("the overview renders")
    }

    #[test]
    fn the_admin_overview_is_a_grid_of_management_areas() {
        let html = overview(true, crate::sections::ADMIN);
        assert!(html.contains("Management areas"));
        for a in crate::sections::ADMIN {
            assert!(html.contains(&format!("/admin/manage/{}", a.key)), "no card for {}", a.key);
            let title = a.title.replace('&', "&amp;");
            assert!(html.contains(&title), "no title for {}", a.key);
        }
    }

    #[test]
    fn office_staff_only_see_the_content_area() {
        let html = overview(false, crate::sections::STAFF);
        assert!(html.contains("/admin/manage/website"));
        assert!(!html.contains("/admin/manage/users"));
        assert!(!html.contains("/admin/manage/academic"));
    }
}

#[cfg(test)]
mod sidebar_tests {
    use crate::auth::{AuthUser, Role};
    use crate::shell::test_nav_for;

    fn admin() -> AuthUser {
        AuthUser {
            id: 1,
            full_name: "Test Admin".into(),
            role: Role::Admin,
            must_change_password: false,
            is_hod: false,
            can_manage: false,
        }
    }

    /// The grouped sidebar actually renders: one heading per group, and every
    /// link still reachable from the rail.
    #[test]
    fn the_sidebar_renders_the_group_headings_and_the_links() {
        let html = super::tests_render_shell(test_nav_for(&admin()));
        for heading in [
            "User &amp; accounts",
            "Academic management",
            "Course operations",
            "Timetable &amp; scheduling",
            "Attendance",
            "Examinations &amp; results",
            "Website &amp; content",
            "Settings",
        ] {
            assert!(html.contains(heading), "the sidebar is missing the {heading} group");
        }
        for href in ["/admin/people", "/admin/marks", "/admin/rank-holders", "/admin/work-queue"] {
            assert!(html.contains(&format!("href=\"{href}\"")), "the sidebar lost {href}");
        }
    }
}
