//! Exam timetable and semester results.
//!
//! The head of department assembles an exam timetable for a programme and
//! semester and pushes it: students then see the timetable between the push
//! and the date of the last exam. Semester results are pushed as rows keyed
//! by the student's PRN — the university's candidate key — and published for
//! the student's Results page straight away.

use sqlx::{FromRow, PgPool};

type Res<T> = Result<T, sqlx::Error>;

// ---------- Exam timetable (editor side) ----------

/// One exam in the timetable a head of department is assembling.
#[derive(Debug, FromRow)]
pub struct ExamEntry {
    pub id: i64,
    pub code: String,
    pub title: String,
    pub name: String,
    pub date_label: String,
    pub start_time: String,
    pub end_time: String,
    pub venue: String,
    pub status: String,
}

/// The state of the whole timetable for one programme and semester: rows are
/// pushed and unpublished as a group, so the window runs from the push to the
/// last exam date across every published row.
#[derive(Debug, FromRow)]
pub struct ExamGroup {
    pub pushed: bool,
    pub exam_count: i64,
    pub pushed_label: Option<String>,
    pub last_date_label: Option<String>,
}

/// Course options for the exam form: the programme's own courses plus every
/// catalogue course (a cross-department offering can still be examined).
#[derive(Debug, FromRow)]
pub struct ExamCourseOption {
    pub id: i64,
    pub code: String,
    pub title: String,
}

pub async fn course_options(db: &PgPool, programme_id: i64) -> Res<Vec<ExamCourseOption>> {
    Ok(sqlx::query_as::<_, ExamCourseOption>(
        r#"SELECT id, code, title FROM courses
           WHERE (programme_id = $1 OR programme_id IS NULL)
             AND COALESCE(is_active, true)
           ORDER BY code"#,
    )
    .bind(programme_id)
    .fetch_all(db)
    .await?)
}

pub async fn exams_for(db: &PgPool, programme_id: i64, semester: i32) -> Res<Vec<ExamEntry>> {
    Ok(sqlx::query_as::<_, ExamEntry>(
        r#"SELECT e.id, c.code, c.title, e.name,
                  to_char(e.exam_date, 'Dy DD Mon YYYY')  AS date_label,
                  to_char(e.start_time, 'HH24:MI')        AS start_time,
                  to_char(e.end_time,   'HH24:MI')        AS end_time,
                  COALESCE(e.venue, '')                    AS venue,
                  e.status
           FROM exams e
           JOIN courses c ON c.id = e.course_id
           WHERE e.programme_id = $1 AND e.semester = $2
           ORDER BY e.exam_date, e.start_time, c.code"#,
    )
    .bind(programme_id)
    .bind(semester)
    .fetch_all(db)
    .await?)
}

pub async fn group_status(db: &PgPool, programme_id: i64, semester: i32) -> Res<ExamGroup> {
    Ok(sqlx::query_as::<_, ExamGroup>(
        r#"SELECT COALESCE(bool_or(status = 'published'), false)      AS pushed,
                  count(*)                                             AS exam_count,
                  to_char(min(published_at), 'Dy DD Mon YYYY HH24:MI') AS pushed_label,
                  to_char(max(exam_date) FILTER (WHERE status = 'published'),
                          'Dy DD Mon YYYY')                            AS last_date_label
           FROM exams
           WHERE programme_id = $1 AND semester = $2"#,
    )
    .bind(programme_id)
    .bind(semester)
    .fetch_one(db)
    .await?)
}

pub struct NewExam<'a> {
    pub programme_id: i64,
    pub semester: i32,
    pub course_id: i64,
    pub name: &'a str,
    pub exam_date: &'a str,
    pub start_time: &'a str,
    pub end_time: &'a str,
    pub venue: &'a str,
}

pub async fn create_exam(db: &PgPool, n: &NewExam<'_>) -> Res<i64> {
    let id = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO exams (programme_id, semester, course_id, name, exam_date,
                              start_time, end_time, venue, status)
           VALUES ($1, $2, $3, $4, $5::date, $6::time, $7::time, NULLIF($8, ''), 'draft')
           RETURNING id"#,
    )
    .bind(n.programme_id)
    .bind(n.semester)
    .bind(n.course_id)
    .bind(n.name)
    .bind(n.exam_date)
    .bind(n.start_time)
    .bind(n.end_time)
    .bind(n.venue)
    .fetch_one(db)
    .await?;
    Ok(id)
}

/// Only an unpublished row can be removed; a pushed timetable is unpublished
/// first so students never see a row vanish from under the window.
pub async fn delete_draft(db: &PgPool, id: i64) -> Res<bool> {
    let done = sqlx::query("DELETE FROM exams WHERE id = $1 AND status = 'draft'")
        .bind(id)
        .execute(db)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Push (or re-push) the whole timetable: every row becomes visible now and
/// the window reopens until the last exam date.
pub async fn push_group(db: &PgPool, programme_id: i64, semester: i32) -> Res<u64> {
    let done = sqlx::query(
        r#"UPDATE exams SET status = 'published', published_at = now(), updated_at = now()
           WHERE programme_id = $1 AND semester = $2"#,
    )
    .bind(programme_id)
    .bind(semester)
    .execute(db)
    .await?;
    Ok(done.rows_affected())
}

pub async fn unpush_group(db: &PgPool, programme_id: i64, semester: i32) -> Res<u64> {
    let done = sqlx::query(
        r#"UPDATE exams SET status = 'draft', published_at = NULL, updated_at = now()
           WHERE programme_id = $1 AND semester = $2 AND status = 'published'"#,
    )
    .bind(programme_id)
    .bind(semester)
    .execute(db)
    .await?;
    Ok(done.rows_affected())
}

// ---------- Exam timetable (student side) ----------

#[derive(Debug, FromRow)]
pub struct StudentExam {
    pub code: String,
    pub title: String,
    pub name: String,
    pub date_label: String,
    pub start_time: String,
    pub end_time: String,
    pub venue: String,
}

/// The window every student-side query shares: the row is published, the push
/// has happened, and today is still within the last exam date of that push
/// (the section disappears the day after the last exam).
const LIVE: &str = r#"
    e.status = 'published'
    AND e.published_at IS NOT NULL
    AND e.published_at <= now()
    AND now() <= (SELECT max(w.exam_date) + interval '1 day'
                    FROM exams w
                   WHERE w.programme_id = e.programme_id
                     AND w.semester = e.semester
                     AND w.status = 'published')"#;

/// The exam timetable this student can see right now, or nothing.
pub async fn student_exams(db: &PgPool, student_id: i64) -> Res<Vec<StudentExam>> {
    Ok(sqlx::query_as::<_, StudentExam>(
        &format!(
            r#"SELECT c.code, c.title, e.name,
                      to_char(e.exam_date, 'Dy DD Mon YYYY') AS date_label,
                      to_char(e.start_time, 'HH24:MI')       AS start_time,
                      to_char(e.end_time,   'HH24:MI')       AS end_time,
                      COALESCE(e.venue, '')                   AS venue
               FROM exams e
               JOIN courses c ON c.id = e.course_id
               JOIN students st ON st.id = $1
               WHERE e.programme_id = st.programme_id
                 AND e.semester = st.semester
                 AND {LIVE}
               ORDER BY e.exam_date, e.start_time, c.code"#,
        ),
    )
    .bind(student_id)
    .fetch_all(db)
    .await?)
}

/// A one-line summary for the overview card while the window is open.
#[derive(Debug, FromRow)]
pub struct ExamNotice {
    pub count: i64,
    pub first_label: String,
    pub last_label: String,
}

pub async fn live_exam_notice(db: &PgPool, student_id: i64) -> Res<Option<ExamNotice>> {
    let row = sqlx::query_as::<_, (i64, Option<String>, Option<String>)>(
        &format!(
            r#"SELECT count(*),
                      to_char(min(e.exam_date), 'Dy DD Mon YYYY'),
                      to_char(max(e.exam_date), 'Dy DD Mon YYYY')
               FROM exams e
               JOIN students st ON st.id = $1
               WHERE e.programme_id = st.programme_id
                 AND e.semester = st.semester
                 AND {LIVE}"#,
        ),
    )
    .bind(student_id)
    .fetch_one(db)
    .await?;
    Ok(if row.0 > 0 {
        Some(ExamNotice {
            count: row.0,
            first_label: row.1.unwrap_or_default(),
            last_label: row.2.unwrap_or_default(),
        })
    } else {
        None
    })
}

// ---------- Results (student side) ----------

/// One published mark row, shown as its own line under the course.
#[derive(Debug)]
pub struct ResultMark {
    pub label: String,
    pub obtained_label: String,
    pub max_label: String,
    pub percent_label: String,
    pub exam_date: Option<String>,
}

/// One course in the Results table, with every published assessment row.
#[derive(Debug)]
pub struct ResultCourse {
    pub code: String,
    pub title: String,
    pub credits: i32,
    pub marks: Vec<ResultMark>,
    pub total_label: String,
    pub percent_label: String,
    pub has_marks: bool,
}

fn assessment_label(assessment: &str) -> String {
    match assessment {
        "internal" => "Internal".into(),
        "assignment" => "Assignment".into(),
        "practical" => "Practical".into(),
        "exam" => "Internal exam".into(),
        "external" => "External".into(),
        other => {
            let mut c = other.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        }
    }
}

fn label(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v:.2}")
    }
}

/// Every semester this student has (or had) enrollments for, always including
/// the current one, so the selector can offer all of them.
pub async fn result_semesters(db: &PgPool, student_id: i64) -> Res<Vec<i32>> {
    let rows = sqlx::query_scalar::<_, i32>(
        r#"SELECT DISTINCT sem FROM (
               SELECT COALESCE(e.semester, c.semester) AS sem
                 FROM enrollments e JOIN courses c ON c.id = e.course_id
                WHERE e.student_id = $1
               UNION
               SELECT semester FROM students WHERE id = $1
           ) t
           WHERE sem IS NOT NULL
           ORDER BY sem"#,
    )
    .bind(student_id)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// Results for one semester: one block per enrolled course, one line per
/// published mark (internal, assignment, practical, and the pushed internal
/// exam result). Courses with nothing published still appear.
pub async fn results_for(db: &PgPool, student_id: i64, semester: i32) -> Res<Vec<ResultCourse>> {
    let rows = sqlx::query_as::<_, (String, String, i32, Option<String>, Option<f64>, Option<f64>, Option<String>)>(
        r#"SELECT c.code, c.title, c.credits,
                  m.assessment, m.marks_obtained::float8, m.max_marks::float8,
                  to_char(ex.exam_date, 'Dy DD Mon YYYY')
           FROM enrollments e
           JOIN courses c ON c.id = e.course_id
           LEFT JOIN marks m
                  ON m.course_id = e.course_id
                 AND m.student_id = e.student_id
                 AND m.published
           LEFT JOIN exams ex ON ex.id = m.exam_id
           WHERE e.student_id = $1
             AND e.status = 'active'
             AND COALESCE(e.semester, c.semester) = $2
           ORDER BY c.code, m.assessment"#,
    )
    .bind(student_id)
    .bind(semester)
    .fetch_all(db)
    .await?;

    let mut out: Vec<ResultCourse> = Vec::new();
    // Raw sums kept alongside the blocks so totals never round twice.
    let mut sums: Vec<(f64, f64)> = Vec::new();
    for (code, title, credits, assessment, obtained, maximum, exam_date) in rows {
        let same_course = out
            .last()
            .map(|c| c.code == code && c.title == title)
            .unwrap_or(false);
        if !same_course {
            out.push(ResultCourse {
                code,
                title,
                credits,
                marks: Vec::new(),
                total_label: String::new(),
                percent_label: "—".into(),
                has_marks: false,
            });
            sums.push((0.0, 0.0));
        }
        let (Some(assessment), Some(obtained), Some(maximum)) = (assessment, obtained, maximum)
        else {
            continue; // course with no published marks yet
        };
        let last = out.len() - 1;
        let percent = if maximum > 0.0 {
            100.0 * obtained / maximum
        } else {
            0.0
        };
        out[last].marks.push(ResultMark {
            label: assessment_label(&assessment),
            obtained_label: label(obtained),
            max_label: label(maximum),
            percent_label: format!("{percent:.0}%"),
            exam_date,
        });
        sums[last].0 += obtained;
        sums[last].1 += maximum;
    }

    for (c, (obtained, maximum)) in out.iter_mut().zip(sums) {
        if c.marks.is_empty() {
            continue;
        }
        c.total_label = format!("{} / {}", label(obtained), label(maximum));
        c.percent_label = if maximum > 0.0 {
            format!("{:.0}%", 100.0 * obtained / maximum)
        } else {
            "—".into()
        };
        c.has_marks = true;
    }
    Ok(out)
}

// ---------- Results push (head of department side) ----------

/// One parsed line of the paste box: `PRN COURSE OBTAINED/MAX`,
/// `PRN COURSE OBTAINED MAX` (exam by default) or
/// `PRN COURSE ASSESSMENT OBTAINED MAX`.
#[derive(Debug, Clone)]
pub struct PushRow {
    pub prn: String,
    pub course: String,
    pub assessment: String,
    pub obtained: f64,
    pub max: f64,
}

#[derive(Debug)]
pub struct PushProblem {
    pub line: usize,
    pub text: String,
    pub problem: String,
}

const ASSESSMENTS: [&str; 5] = ["internal", "external", "practical", "assignment", "exam"];

fn split_fields(line: &str) -> Vec<String> {
    if line.contains(',') || line.contains('\t') {
        line.split([',', '\t']).map(|f| f.trim().to_string()).collect()
    } else {
        line.split_whitespace().map(|f| f.to_string()).collect()
    }
}

fn ratio(part: &str) -> Option<(f64, f64)> {
    let (a, b) = part.split_once('/')?;
    let obtained = a.trim().parse::<f64>().ok()?;
    let max = b.trim().parse::<f64>().ok()?;
    Some((obtained, max))
}

/// Parse the paste box, collecting a per-line problem for anything malformed.
pub fn parse_push(text: &str) -> (Vec<PushRow>, Vec<PushProblem>) {
    let mut rows = Vec::new();
    let mut problems = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let line_no = i + 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = split_fields(line);
        // A header row like `prn,course,marks` is common in exports.
        if fields.first().map(|f| f.eq_ignore_ascii_case("prn")).unwrap_or(false) {
            continue;
        }
        let mut push = |prn: String, course: String, assessment: String, obtained: f64, max: f64| {
            rows.push(PushRow {
                prn,
                course,
                assessment,
                obtained,
                max,
            })
        };
        match fields.len() {
            3 => match ratio(&fields[2]) {
                Some((obtained, max)) => push(
                    fields[0].clone(),
                    fields[1].clone(),
                    "exam".into(),
                    obtained,
                    max,
                ),
                None => problems.push(PushProblem {
                    line: line_no,
                    text: line.to_string(),
                    problem: "write the marks as obtained/max, e.g. 42/50".into(),
                }),
            },
            4 if ASSESSMENTS.contains(&fields[2].to_ascii_lowercase().as_str()) => {
                match ratio(&fields[3]) {
                    Some((obtained, max)) => push(
                        fields[0].clone(),
                        fields[1].clone(),
                        fields[2].to_ascii_lowercase(),
                        obtained,
                        max,
                    ),
                    None => problems.push(PushProblem {
                        line: line_no,
                        text: line.to_string(),
                        problem: "write the marks as obtained/max, e.g. 42/50".into(),
                    }),
                }
            }
            4 => match (fields[2].parse::<f64>(), fields[3].parse::<f64>()) {
                (Ok(obtained), Ok(max)) => push(
                    fields[0].clone(),
                    fields[1].clone(),
                    "exam".into(),
                    obtained,
                    max,
                ),
                _ => problems.push(PushProblem {
                    line: line_no,
                    text: line.to_string(),
                    problem: "expected two numbers: obtained and max".into(),
                }),
            },
            5 => {
                let assessment = fields[2].to_ascii_lowercase();
                if !ASSESSMENTS.contains(&assessment.as_str()) {
                    problems.push(PushProblem {
                        line: line_no,
                        text: line.to_string(),
                        problem: format!("assessment must be one of: {}", ASSESSMENTS.join(", ")),
                    });
                } else {
                    match (fields[3].parse::<f64>(), fields[4].parse::<f64>()) {
                        (Ok(obtained), Ok(max)) => {
                            push(fields[0].clone(), fields[1].clone(), assessment, obtained, max)
                        }
                        _ => problems.push(PushProblem {
                            line: line_no,
                            text: line.to_string(),
                            problem: "expected two numbers: obtained and max".into(),
                        }),
                    }
                }
            }
            _ => problems.push(PushProblem {
                line: line_no,
                text: line.to_string(),
                problem: "expected 3, 4 or 5 fields (PRN, course, [assessment], obtained, max)".into(),
            }),
        }
    }
    (rows, problems)
}

/// A student found by PRN, plus what the push needs to check and file it.
/// (The type itself crosses into the route, so it is public; its fields stay
/// private to this module.)
pub struct Candidate {
    student_id: i64,
    programme_id: i64,
    semester: i32,
    course_id: i64,
}

/// Validate every row against the database: the PRN must find an active
/// student the manager may act for, the course must be real and belong to the
/// student's programme (catalogue courses belong to everyone), and the marks
/// must fit. Returns the rows that are good to file, with a problem per line
/// that is not.
pub async fn validate_push(
    db: &PgPool,
    department: Option<i64>,
    rows: &[PushRow],
) -> Res<(Vec<(PushRow, Candidate)>, Vec<PushProblem>)> {
    let mut good = Vec::new();
    let mut problems = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let line = i + 1;
        let fail = |problems: &mut Vec<PushProblem>, text: String, problem: String| {
            problems.push(PushProblem { line, text, problem })
        };
        let text = format!(
            "{} {} {} {}/{}",
            row.prn, row.course, row.assessment, row.obtained, row.max
        );
        if !ASSESSMENTS.contains(&row.assessment.as_str()) {
            fail(
                &mut problems,
                text,
                format!("assessment must be one of: {}", ASSESSMENTS.join(", ")),
            );
            continue;
        }
        if row.max <= 0.0 || row.obtained < 0.0 || row.obtained > row.max {
            fail(
                &mut problems,
                text,
                "max must be above 0 and obtained between 0 and max".into(),
            );
            continue;
        }
        let candidate = sqlx::query_as::<_, (i64, i64, i32)>(
            r#"SELECT id, programme_id, semester FROM students
               WHERE prn = $1 AND is_active"#,
        )
        .bind(&row.prn)
        .fetch_optional(db)
        .await?;
        let Some((student_id, programme_id, semester)) = candidate else {
            fail(&mut problems, text, "no active student with that PRN".into());
            continue;
        };
        if !crate::services::academics::may_manage_programme(db, programme_id, department).await? {
            fail(
                &mut problems,
                text,
                "that student's programme is not in your department".into(),
            );
            continue;
        }
        let course = sqlx::query_as::<_, (i64, Option<i64>)>(
            r#"SELECT id, programme_id FROM courses
               WHERE upper(code) = upper($1) AND COALESCE(is_active, true)
               ORDER BY id LIMIT 1"#,
        )
        .bind(&row.course)
        .fetch_optional(db)
        .await?;
        let Some((course_id, course_programme)) = course else {
            fail(&mut problems, text, "no active course with that code".into());
            continue;
        };
        if let Some(cp) = course_programme {
            if cp != programme_id {
                fail(
                    &mut problems,
                    text,
                    "that course does not belong to the student's programme".into(),
                );
                continue;
            }
        }
        good.push((
            row.clone(),
            Candidate {
                student_id,
                programme_id,
                semester,
                course_id,
            },
        ));
    }
    Ok((good, problems))
}

/// File the validated rows: published immediately, upserted on
/// (student, course, assessment) so re-pushing corrects a result. Exam rows
/// attach to the student's published exam for that course when one exists.
pub async fn push_marks(db: &PgPool, rows: &[(PushRow, Candidate)]) -> Res<u64> {
    let mut tx = db.begin().await?;
    let mut filed = 0u64;
    for (row, c) in rows {
        // An exam result attaches to the published exam it belongs to, so the
        // student's result page can show the exam date alongside the score.
        let exam_id: Option<i64> = if row.assessment == "exam" {
            sqlx::query_scalar(
                r#"SELECT id FROM exams
                   WHERE course_id = $1 AND programme_id = $2 AND semester = $3
                     AND status = 'published'
                   ORDER BY exam_date LIMIT 1"#,
            )
            .bind(c.course_id)
            .bind(c.programme_id)
            .bind(c.semester)
            .fetch_optional(&mut *tx)
            .await?
        } else {
            None
        };
        sqlx::query(
            r#"INSERT INTO marks (student_id, course_id, exam_id, assessment,
                                  marks_obtained, max_marks, published)
               VALUES ($1, $2, $3, $4, $5, $6, true)
               ON CONFLICT (student_id, course_id, assessment)
               DO UPDATE SET marks_obtained = EXCLUDED.marks_obtained,
                             max_marks      = EXCLUDED.max_marks,
                             exam_id        = COALESCE(EXCLUDED.exam_id, marks.exam_id),
                             published      = true,
                             updated_at     = now()"#,
        )
        .bind(c.student_id)
        .bind(c.course_id)
        .bind(exam_id)
        .bind(&row.assessment)
        .bind(row.obtained)
        .bind(row.max)
        .execute(&mut *tx)
        .await?;
        filed += 1;
    }
    tx.commit().await?;
    Ok(filed)
}
