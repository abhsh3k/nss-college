use std::collections::{BTreeMap, HashMap};

use askama::Template;
use axum::extract::State;
use sqlx::{types::time::OffsetDateTime, FromRow};
use tower_sessions::Session;

use crate::{
    auth::{OfficeOrAdmin, Role, StudentOnly},
    error::AppError,
    models::Notice,
    services::{hub, users::{self, Counts}},
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

<<<<<<< HEAD
=======
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

>>>>>>> c857d6ecb1855cec020ebac0e8e009a8e81b9665
// ---------- Student Hub (Layer 4d) ----------

/// Programme details shown under the welcome heading.
pub struct HubProfile {
    pub programme: String,
    pub semester_label: String,
    pub admission_no: String,
}

/// The e-grants warning card (only rendered when below the threshold).
pub struct EgrantsAlert {
    pub month_label: String,
    pub percent_label: String,
    pub attended: i64,
    pub marked: i64,
    pub threshold: i32,
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

/// Attendance for one course, inside the detailed attendance section.
pub struct HubCourseAttendance {
    pub code: String,
    pub title: String,
    pub marked: i64,
    pub attended: i64,
    pub absent: i64,
    pub leave: i64,
    pub percent_label: String,
    pub bar_width: String,
    pub is_low: bool,
}

/// One month of the attendance trend.
pub struct HubMonth {
    pub label: String,
    pub marked: i64,
    pub attended: i64,
    pub percent_label: String,
    pub bar_width: String,
    pub is_low: bool,
}

/// One recently marked session.
pub struct HubRecent {
    pub date_label: String,
    pub time_label: String,
    pub code: String,
    pub title: String,
    pub status_label: &'static str,
    pub tone: &'static str,
}

/// The detailed attendance section: overall standing, per-course breakdown,
/// month-by-month trend and the latest marked sessions.
pub struct HubAttendance {
    pub present: i64,
    pub absent: i64,
    pub leave: i64,
    pub unmarked: i64,
    pub held: i64,
    pub percent_label: String,
    pub bar_width: String,
    pub is_low: bool,
    pub has_data: bool,
    pub per_course: Vec<HubCourseAttendance>,
    pub months: Vec<HubMonth>,
    pub recent: Vec<HubRecent>,
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

#[derive(Template)]
#[template(path = "dashboard/student.html")]
pub struct StudentTemplate {
    shell: Shell,
    today_str: String,
    feed: Vec<Notice>,
    profile: Option<HubProfile>,
    alert: Option<EgrantsAlert>,
    min_percent: i32,
    today_periods: Vec<HubPeriod>,
    has_week: bool,
    days: Vec<HubDay>,
    rows: Vec<HubRow>,
    courses: Vec<HubCourse>,
    total_credits: i32,
    attendance: Option<HubAttendance>,
}

/// Percentage helper shared by every attendance figure on the page.
fn percent(attended: i64, marked: i64) -> f64 {
    if marked > 0 {
        100.0 * attended as f64 / marked as f64
    } else {
        0.0
    }
}

/// Present/absent/leave tone for a recent session badge.
fn status_badge(status: &str) -> (&'static str, &'static str) {
    match status {
        "present" => (
            "Present",
            "border-emerald-200 bg-emerald-50 text-emerald-700",
        ),
        "leave" => ("On leave", "border-amber-200 bg-amber-50 text-amber-700"),
        _ => ("Absent", "border-rose-200 bg-rose-50 text-rose-700"),
    }
}

/// "Room 12 · Dr. Name", with sensible fallbacks for missing data.
fn period_detail(room: &str, faculty: &str) -> String {
    let room = if room.is_empty() { "Room TBA" } else { room };
    if faculty.is_empty() {
        room.to_string()
    } else {
        format!("{room} · {faculty}")
    }
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
            alert: None,
            min_percent: 75,
            today_periods: Vec::new(),
            has_week: false,
            days: Vec::new(),
            rows: Vec::new(),
            courses: Vec::new(),
            total_credits: 0,
            attendance: None,
        });
    };

    let min_percent = hub::setting(&s.db, "attendance_min_percent", 75).await?;

    // Today's timeline + the whole week, from one timetable query.
    let week = hub::week_schedule(&s.db, p.programme_id, p.semester).await?;
    let today_periods: Vec<HubPeriod> = week
        .iter()
        .filter(|x| x.weekday == weekday_today)
        .map(|x| HubPeriod {
            code: x.code.clone(),
            title: x.title.clone(),
            start_time: x.start_time.clone(),
            end_time: x.end_time.clone(),
            detail: period_detail(&x.room, &x.faculty),
            is_now: x.start_time <= now_hm && now_hm < x.end_time,
            is_past: now_hm >= x.end_time,
        })
        .collect();

    // Weekly matrix: rows = distinct start times, columns = days (Sunday only if used).
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

    // Detailed attendance: overall standing, per course, month trend and recent sessions.
    let summary = hub::attendance_summary(&s.db, p.id).await?;
    let per_course: Vec<HubCourseAttendance> = hub::course_attendance_detail(&s.db, p.id)
        .await?
        .into_iter()
        .map(|d| {
            let percent = percent(d.attended, d.marked);
            HubCourseAttendance {
                code: d.code,
                title: d.title,
                marked: d.marked,
                attended: d.attended,
                absent: d.absent,
                leave: d.leave,
                percent_label: if d.marked == 0 {
                    "—".into()
                } else {
                    format!("{percent:.0}%")
                },
                bar_width: format!("{}%", percent.round().clamp(0.0, 100.0) as i32),
                is_low: d.marked > 0 && percent < min_percent as f64,
            }
        })
        .collect();

    let months: Vec<HubMonth> = hub::month_attendance(&s.db, p.id, 6)
        .await?
        .into_iter()
        .map(|m| {
            let percent = percent(m.attended, m.marked);
            HubMonth {
                percent_label: format!("{percent:.0}%"),
                bar_width: format!("{}%", percent.round().clamp(0.0, 100.0) as i32),
                is_low: percent < min_percent as f64,
                label: m.month_label,
                marked: m.marked,
                attended: m.attended,
            }
        })
        .collect();

    let recent: Vec<HubRecent> = hub::recent_sessions(&s.db, p.id, 8)
        .await?
        .into_iter()
        .map(|r| {
            let (status_label, tone) = status_badge(&r.status);
            HubRecent {
                date_label: r.date_label,
                time_label: r.time_label,
                code: r.code,
                title: r.title,
                status_label,
                tone,
            }
        })
        .collect();

    let overall = percent(summary.present + summary.leave, summary.marked);
    let attendance = HubAttendance {
        present: summary.present,
        absent: summary.absent,
        leave: summary.leave,
        unmarked: (summary.held - summary.marked).max(0),
        held: summary.held,
        percent_label: if summary.marked == 0 {
            "—".into()
        } else {
            format!("{overall:.0}%")
        },
        bar_width: format!("{}%", overall.round().clamp(0.0, 100.0) as i32),
        is_low: summary.marked > 0 && overall < min_percent as f64,
        has_data: summary.marked > 0,
        per_course,
        months,
        recent,
    };

    // E-grants students get a warning card when this month's attendance dips
    // below the configured threshold.
    let alert = if p.egrants {
        let threshold = hub::setting(&s.db, "attendance_egrants_monthly_min_percent", 75).await?;
        let m = hub::monthly_attendance(&s.db, p.id).await?;
        let percent = if m.marked > 0 {
            100.0 * m.attended as f64 / m.marked as f64
        } else {
            100.0
        };
        if m.marked > 0 && percent < threshold as f64 {
            Some(EgrantsAlert {
                month_label: m.month_label,
                percent_label: format!("{percent:.0}%"),
                attended: m.attended,
                marked: m.marked,
                threshold,
            })
        } else {
            None
        }
    } else {
        None
    };

    Ok(StudentTemplate {
        shell,
        today_str,
        feed,
        profile: Some(HubProfile {
            programme: p.programme,
            semester_label: format!("Semester {}", p.semester),
            admission_no: p.admission_no,
        }),
        alert,
        min_percent,
        today_periods,
        has_week,
        days,
        rows,
        courses,
        total_credits,
        attendance: Some(attendance),
    })
}