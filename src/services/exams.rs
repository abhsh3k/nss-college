//! Exam timetable and semester results.
//!
//! The head of department assembles an exam timetable for a programme and
//! semester and pushes it: students then see the timetable between the push
//! and the date of the last exam.
//!
//! Two kinds of result are pushed, each under its own key:
//!
//! * the university's semester exam, keyed by the student's PRN;
//! * an internal exam the college itself conducts, keyed by the admission
//!   number and named, so one course can carry several of them.
//!
//! Both are filed against the semester picked on the page, so a result pushed
//! after the student has been promoted still reads back under the semester it
//! was earned in.

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

/// Course options for one semester of a programme: its own courses taught in
/// that semester, plus the catalogue courses offered to it that semester — a
/// catalogue course carries no `semester` of its own, so its offering decides
/// which semester it belongs to.
pub async fn course_options_for_semester(
    db: &PgPool,
    programme_id: i64,
    semester: i32,
) -> Res<Vec<ExamCourseOption>> {
    Ok(sqlx::query_as::<_, ExamCourseOption>(
        r#"SELECT id, code, title FROM courses
           WHERE COALESCE(is_active, true)
             AND ( (programme_id = $1 AND semester = $2)
                OR (programme_id IS NULL
                    AND EXISTS (SELECT 1 FROM course_offerings o
                                  JOIN course_offering_targets t ON t.offering_id = o.id
                                 WHERE o.course_id = courses.id
                                   AND o.semester = $2
                                   AND t.programme_id = $1)) )
           ORDER BY code"#,
    )
    .bind(programme_id)
    .bind(semester)
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
    /// True for coursework (internal, assignment, practical, external); the
    /// coursework total on the Results page only adds these rows.
    pub is_coursework: bool,
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

/// How a mark line reads on the student's Results page: the exam kind wins
/// over the raw assessment, because the same `exam` assessment covers both the
/// university's semester result and a college internal exam.
pub fn assessment_label(assessment: &str, exam_kind: &str, exam_name: &str) -> String {
    let base = match exam_kind {
        "university" => "University exam".to_string(),
        "internal" if exam_name.is_empty() => "Internal exam".to_string(),
        "internal" => format!("Internal exam — {exam_name}"),
        _ => match assessment {
            "internal" => "Internal".into(),
            "assignment" => "Assignment".into(),
            "practical" => "Practical".into(),
            "exam" => "Exam".into(),
            "external" => "External".into(),
            other => {
                let mut c = other.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    None => String::new(),
                }
            }
        },
    };
    base
}

fn label(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v:.2}")
    }
}

/// Every semester this student has results for: the ones they enrolled in,
/// the one they are in now, and any semester a result was filed under — so a
/// student three semesters in can still read semester one and two.
pub async fn result_semesters(db: &PgPool, student_id: i64) -> Res<Vec<i32>> {
    let rows = sqlx::query_scalar::<_, i32>(
        r#"SELECT DISTINCT sem FROM (
               SELECT COALESCE(e.semester, c.semester) AS sem
                 FROM enrollments e JOIN courses c ON c.id = e.course_id
                WHERE e.student_id = $1
               UNION
               SELECT semester FROM students WHERE id = $1
               UNION
               SELECT semester FROM marks
                WHERE student_id = $1 AND published AND semester IS NOT NULL
           ) t
           WHERE sem IS NOT NULL
           ORDER BY sem"#,
    )
    .bind(student_id)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// Results for one semester: one block per course, one line per published mark
/// (coursework, the college's internal exams, and the university's semester
/// exam). Courses with nothing published still appear.
///
/// Two sources are joined: the enrollments of that semester, and any result
/// filed under it — a result stands on its own once it exists, so promotion
/// away from the semester can never take it off the page.
pub async fn results_for(db: &PgPool, student_id: i64, semester: i32) -> Res<Vec<ResultCourse>> {
    let rows = sqlx::query_as::<_, (
        String,
        String,
        i32,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<f64>,
        Option<f64>,
        Option<String>,
    )>(
        r#"SELECT c.code, c.title, c.credits,
                  m.assessment, m.exam_kind, m.exam_name,
                  m.marks_obtained::float8, m.max_marks::float8,
                  to_char(ex.exam_date, 'Dy DD Mon YYYY')
             FROM enrollments e
             JOIN courses c ON c.id = e.course_id
             LEFT JOIN marks m
                    ON m.course_id = e.course_id
                   AND m.student_id = e.student_id
                   AND m.published
                   AND (m.semester = $2 OR m.semester IS NULL)
             LEFT JOIN exams ex ON ex.id = m.exam_id
            WHERE e.student_id = $1
              AND e.status = 'active'
              AND COALESCE(e.semester, c.semester) = $2
           UNION ALL
           SELECT c.code, c.title, c.credits,
                  m.assessment, m.exam_kind, m.exam_name,
                  m.marks_obtained::float8, m.max_marks::float8,
                  to_char(ex.exam_date, 'Dy DD Mon YYYY')
             FROM marks m
             JOIN courses c ON c.id = m.course_id
             LEFT JOIN exams ex ON ex.id = m.exam_id
            WHERE m.student_id = $1
              AND m.published
              AND m.semester = $2
              AND NOT EXISTS (
                  SELECT 1
                    FROM enrollments e JOIN courses ec ON ec.id = e.course_id
                   WHERE e.student_id = m.student_id
                     AND e.course_id = m.course_id
                     AND e.status = 'active'
                     AND COALESCE(e.semester, ec.semester) = $2)
            ORDER BY 1, 4, 5, 6"#,
    )
    .bind(student_id)
    .bind(semester)
    .fetch_all(db)
    .await?;

    let mut out: Vec<ResultCourse> = Vec::new();
    // Raw sums kept alongside the blocks so totals never round twice.
    let mut sums: Vec<(f64, f64)> = Vec::new();
    for (code, title, credits, assessment, exam_kind, exam_name, obtained, maximum, exam_date) in rows {
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
        let (
            Some(assessment),
            Some(exam_kind),
            Some(obtained),
            Some(maximum),
        ) = (assessment, exam_kind, obtained, maximum)
        else {
            continue; // course with no published marks yet
        };
        let exam_name = exam_name.unwrap_or_default();
        let last = out.len() - 1;
        let percent = if maximum > 0.0 {
            100.0 * obtained / maximum
        } else {
            0.0
        };
        out[last].marks.push(ResultMark {
            label: assessment_label(&assessment, &exam_kind, &exam_name),
            obtained_label: label(obtained),
            max_label: label(maximum),
            percent_label: format!("{percent:.0}%"),
            exam_date,
            is_coursework: exam_kind != "university" && exam_kind != "internal",
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

/// Which exam a push belongs to. It decides the key the rows are matched on,
/// how the result is filed, and how the student's Results page labels it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushKind {
    /// The university's semester exam, matched on the PRN.
    University,
    /// An internal exam the college itself conducts, matched on the
    /// admission number and named, so a course can carry several.
    Internal,
}

impl PushKind {
    pub fn parse(raw: &str) -> Self {
        if raw.trim().eq_ignore_ascii_case("internal") {
            PushKind::Internal
        } else {
            PushKind::University
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            PushKind::University => "university",
            PushKind::Internal => "internal",
        }
    }

    /// What the first column of a row carries, for the form's help text.
    pub fn key_label(self) -> &'static str {
        match self {
            PushKind::University => "PRN",
            PushKind::Internal => "admission number",
        }
    }

    /// What an internal row is called when it does not carry its own name.
    fn default_exam_name(self) -> &'static str {
        "Internal exam"
    }
}

/// One parsed line of the paste box.
///
/// University rows: `PRN COURSE OBTAINED/MAX`, `PRN COURSE OBTAINED MAX`
/// (the semester exam by default) or `PRN COURSE ASSESSMENT OBTAINED MAX`.
///
/// Internal rows: `ADMISSION_NO COURSE EXAM_NAME OBTAINED/MAX`, with the exam
/// name left out when the marks come as a ratio or as two numbers
/// (`ADMISSION_NO COURSE 42/50`).
#[derive(Debug, Clone)]
pub struct PushRow {
    /// The student's PRN or admission number, depending on [`PushKind`].
    pub key: String,
    pub course: String,
    pub assessment: String,
    /// The internal exam's own name; empty for every other kind.
    pub exam_name: String,
    pub obtained: f64,
    pub max: f64,
    /// The line this row came from in the paste box or the file, 1-based, so a
    /// problem can point at it even when blank, comment or header lines were
    /// skipped along the way. 0 for a row built rather than parsed.
    pub line: usize,
}

#[derive(Debug)]
pub struct PushProblem {
    pub line: usize,
    pub text: String,
    pub problem: String,
}

const ASSESSMENTS: [&str; 5] = ["internal", "external", "practical", "assignment", "exam"];

/// Comma- or tab-separated when the line looks like one, space-separated
/// otherwise, with double quotes protecting a field that carries spaces
/// (`2501 BCA101 "Internal 1" 42/50`). Empty fields survive in the
/// delimited form, as they do in a spreadsheet export.
fn split_fields(line: &str) -> Vec<String> {
    let delimited = line.contains(',') || line.contains('\t');
    let sep = |c: char| if delimited { c == ',' || c == '\t' } else { c.is_whitespace() };
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            _ if !quoted && sep(c) => {
                if delimited || !cur.is_empty() {
                    out.push(cur.trim().to_string());
                    cur.clear();
                }
            }
            _ => cur.push(c),
        }
    }
    if delimited || !cur.is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// Parse a CSV byte slice into result rows. The CSV must have columns:
/// `key,course,assessment,obtained,max` (university) or
/// `key,course,exam_name,obtained,max` (internal).
/// The first row may be a header row (which is skipped).
pub fn parse_csv(bytes: &[u8], kind: PushKind) -> Result<Vec<PushRow>, String> {
    let text = String::from_utf8(bytes.to_vec())
        .map_err(|e| format!("CSV file is not valid UTF-8: {e}"))?;
    let mut rows = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = split_fields(line);
        // Skip header rows like "prn,course,marks" or "key,course,obtained,max"
        if i == 0 && is_header_row(&fields) {
            continue;
        }
        let built = if fields.len() < 2 {
            Err(format!(
                "Line {}: expected at least 2 fields (key, course, ...), got {}",
                i + 1,
                fields.len()
            ))
        } else {
            match kind {
                PushKind::University => university_row(&fields, i + 1),
                PushKind::Internal => internal_row(&fields, i + 1),
            }
        };
        match built {
            Ok(row) => rows.push(row),
            Err(e) => return Err(e),
        }
    }
    Ok(rows)
}

fn ratio(part: &str) -> Option<(f64, f64)> {
    let (a, b) = part.split_once('/')?;
    let obtained = a.trim().parse::<f64>().ok()?;
    let max = b.trim().parse::<f64>().ok()?;
    Some((obtained, max))
}

/// The first column is a header like `prn` or `admission_no`, not a student.
fn is_header_row(fields: &[String]) -> bool {
    fields
        .first()
        .map(|f| {
            let norm: String = f
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            matches!(norm.as_str(), "prn" | "admissionno" | "admno" | "admission" | "key")
        })
        .unwrap_or(false)
}

/// A university semester-exam row, or the reason the line is unusable.
fn university_row(fields: &[String], line: usize) -> Result<PushRow, String> {
    let row = |assessment: String, obtained: f64, max: f64| PushRow {
        key: fields[0].clone(),
        course: fields[1].clone(),
        assessment,
        exam_name: String::new(),
        obtained,
        max,
        line,
    };
    match fields.len() {
        3 => ratio(&fields[2])
            .map(|(o, m)| row("exam".into(), o, m))
            .ok_or_else(|| "write the marks as obtained/max, e.g. 42/50".to_string()),
        4 if ASSESSMENTS.contains(&fields[2].to_ascii_lowercase().as_str()) => ratio(&fields[3])
            .map(|(o, m)| row(fields[2].to_ascii_lowercase(), o, m))
            .ok_or_else(|| "write the marks as obtained/max, e.g. 42/50".to_string()),
        4 => match (fields[2].parse::<f64>(), fields[3].parse::<f64>()) {
            (Ok(obtained), Ok(max)) => Ok(row("exam".into(), obtained, max)),
            _ => Err("expected two numbers: obtained and max".to_string()),
        },
        5 => {
            let assessment = fields[2].to_ascii_lowercase();
            if !ASSESSMENTS.contains(&assessment.as_str()) {
                return Err(format!("assessment must be one of: {}", ASSESSMENTS.join(", ")));
            }
            match (fields[3].parse::<f64>(), fields[4].parse::<f64>()) {
                (Ok(obtained), Ok(max)) => Ok(row(assessment, obtained, max)),
                _ => Err("expected two numbers: obtained and max".to_string()),
            }
        }
        _ => Err("expected 3, 4 or 5 fields (PRN, course, [assessment], obtained, max)".to_string()),
    }
}

/// A college internal-exam row: `admission_no course exam_name obtained/max`.
fn internal_row(fields: &[String], line: usize) -> Result<PushRow, String> {
    let default_name = PushKind::Internal.default_exam_name();
    // Empty or whitespace-only names fall back to the generic one, so two
    // unnamed pushes still agree instead of piling up as separate exams.
    let row = |name: &str, obtained: f64, max: f64| {
        let exam_name = if name.trim().is_empty() {
            default_name.to_string()
        } else {
            name.trim().to_string()
        };
        PushRow {
            key: fields[0].clone(),
            course: fields[1].clone(),
            assessment: "exam".into(),
            exam_name,
            obtained,
            max,
            line,
        }
    };
    match fields.len() {
        3 => ratio(&fields[2])
            .map(|(o, m)| row(default_name, o, m))
            .ok_or_else(|| "write the marks as obtained/max, e.g. 42/50".to_string()),
        4 => {
            // `admission_no course 42 50` (no name) or `… course name 42/50`.
            if let (Ok(obtained), Ok(max)) = (fields[2].parse::<f64>(), fields[3].parse::<f64>()) {
                Ok(row(default_name, obtained, max))
            } else {
                ratio(&fields[3])
                    .map(|(o, m)| row(&fields[2], o, m))
                    .ok_or_else(|| {
                        "write the exam name, then the marks as obtained/max, e.g. \"Internal 1 42/50\""
                            .to_string()
                    })
            }
        }
        5 => match (fields[3].parse::<f64>(), fields[4].parse::<f64>()) {
            (Ok(obtained), Ok(max)) => Ok(row(&fields[2], obtained, max)),
            _ => Err("expected two numbers: obtained and max".to_string()),
        },
        _ => Err(
            "expected 3, 4 or 5 fields (admission no, course, [exam name], obtained, max)"
                .to_string(),
        ),
    }
}

/// Parse the paste box for one kind of exam, collecting a per-line problem
/// for anything malformed.
pub fn parse_push(text: &str, kind: PushKind) -> (Vec<PushRow>, Vec<PushProblem>) {
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
        if is_header_row(&fields) {
            continue;
        }
        let built = if fields.len() < 2 {
            Err(format!(
                "expected 3, 4 or 5 fields ({}, course, [exam name], obtained, max)",
                kind.key_label()
            ))
        } else {
            match kind {
                PushKind::University => university_row(&fields, line_no),
                PushKind::Internal => internal_row(&fields, line_no),
            }
        };
        match built {
            Ok(row) => rows.push(row),
            Err(problem) => problems.push(PushProblem {
                line: line_no,
                text: line.to_string(),
                problem,
            }),
        }
    }
    (rows, problems)
}

/// A student found by the push's key, plus what the push needs to check and
/// file the row. (The type itself crosses into the route, so it is public;
/// its fields stay private to this module.)
pub struct Candidate {
    student_id: i64,
    programme_id: i64,
    course_id: i64,
}

/// Validate every row against the database: the key must find an active
/// student the manager may act for, the course must be real and belong to the
/// student's programme (catalogue courses belong to everyone), and the marks
/// must fit. Returns the rows that are good to file, with a problem per line
/// that is not.
pub async fn validate_push(
    db: &PgPool,
    department: Option<i64>,
    kind: PushKind,
    rows: &[PushRow],
) -> Res<(Vec<(PushRow, Candidate)>, Vec<PushProblem>)> {
    let mut good = Vec::new();
    let mut problems = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        // Where the row came from, so a problem names the line the head wrote.
        let line = if row.line > 0 { row.line } else { i + 1 };
        let fail = |problems: &mut Vec<PushProblem>, text: String, problem: String| {
            problems.push(PushProblem { line, text, problem })
        };
        // What the line is echoed back as in the problem list.
        let text = match kind {
            PushKind::Internal => format!(
                "{} {} {} {}/{}",
                row.key, row.course, row.exam_name, row.obtained, row.max
            ),
            PushKind::University => format!(
                "{} {} {} {}/{}",
                row.key, row.course, row.assessment, row.obtained, row.max
            ),
        };
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
        // The university exam is the candidate's PRN; an internal exam is the
        // college's own list, which is keyed by admission number.
        let candidate = match kind {
            PushKind::University => {
                sqlx::query_as::<_, (i64, i64)>(
                    r#"SELECT id, programme_id FROM students
                       WHERE prn = $1 AND is_active"#,
                )
                .bind(&row.key)
                .fetch_optional(db)
                .await?
            }
            PushKind::Internal => {
                sqlx::query_as::<_, (i64, i64)>(
                    r#"SELECT id, programme_id FROM students
                       WHERE lower(btrim(admission_no)) = lower(btrim($1)) AND is_active"#,
                )
                .bind(&row.key)
                .fetch_optional(db)
                .await?
            }
        };
        let Some((student_id, programme_id)) = candidate else {
            let what = match kind {
                PushKind::University => "PRN",
                PushKind::Internal => "admission number",
            };
            fail(&mut problems, text, format!("no active student with that {what}"));
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
                course_id,
            },
        ));
    }
    Ok((good, problems))
}

/// One validated row, filed under the semester picked on the page and
/// upserted per (student, course, assessment, exam_kind, exam_name) so
/// re-filing corrects a result without disturbing the other kinds. An internal
/// exam attaches to the college's published exam for that course and semester
/// when one exists, so the date shows beside the score; every other kind
/// carries no college exam date.
#[allow(clippy::too_many_arguments)]
async fn file_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    semester: i32,
    c: &Candidate,
    assessment: &str,
    exam_kind: &str,
    exam_name: &str,
    obtained: f64,
    max: f64,
) -> Res<()> {
    let exam_id: Option<i64> = if exam_kind == "internal" {
        sqlx::query_scalar(
            r#"SELECT id FROM exams
               WHERE course_id = $1 AND programme_id = $2 AND semester = $3
                 AND status = 'published'
               ORDER BY exam_date LIMIT 1"#,
        )
        .bind(c.course_id)
        .bind(c.programme_id)
        .bind(semester)
        .fetch_optional(&mut **tx)
        .await?
    } else {
        None
    };
    sqlx::query(
        r#"INSERT INTO marks (student_id, course_id, exam_id, assessment,
                              marks_obtained, max_marks, published,
                              exam_kind, exam_name, semester)
           VALUES ($1, $2, $3, $4, $5, $6, true, $7, $8, $9)
           ON CONFLICT (student_id, course_id, assessment, exam_kind, exam_name)
           DO UPDATE SET marks_obtained = EXCLUDED.marks_obtained,
                         max_marks      = EXCLUDED.max_marks,
                         exam_id        = COALESCE(EXCLUDED.exam_id, marks.exam_id),
                         published      = true,
                         semester       = EXCLUDED.semester,
                         updated_at     = now()"#,
    )
    .bind(c.student_id)
    .bind(c.course_id)
    .bind(exam_id)
    .bind(assessment)
    .bind(obtained)
    .bind(max)
    .bind(exam_kind)
    .bind(exam_name)
    .bind(semester)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// File the validated rows, published immediately. `semester` is the semester
/// the results are filed under — the one picked on the page — which is what
/// lets a result pushed after promotion still read back under the semester it
/// was earned in. What a row *is* comes from the row itself: the semester exam
/// for a `exam` assessment on a university push, coursework alongside it, and
/// the named internal exam otherwise.
pub async fn push_marks(
    db: &PgPool,
    kind: PushKind,
    semester: i32,
    rows: &[(PushRow, Candidate)],
) -> Res<u64> {
    let mut tx = db.begin().await?;
    let mut filed = 0u64;
    for (row, c) in rows {
        let exam_kind = match kind {
            PushKind::Internal => "internal",
            PushKind::University if row.assessment == "exam" => "university",
            PushKind::University => "coursework",
        };
        let exam_name = if exam_kind == "internal" { row.exam_name.as_str() } else { "" };
        file_row(
            &mut tx,
            semester,
            c,
            &row.assessment,
            exam_kind,
            exam_name,
            row.obtained,
            row.max,
        )
        .await?;
        filed += 1;
    }
    tx.commit().await?;
    Ok(filed)
}

/// File the validated rows under an explicit assessment key — the marks-entry
/// CSV import, where the sheet the head has open decides what every row is
/// (its assessment, exam kind and exam name), whatever the file carries.
pub async fn push_marks_as(
    db: &PgPool,
    semester: i32,
    assessment: &str,
    exam_kind: &str,
    exam_name: &str,
    rows: &[(PushRow, Candidate)],
) -> Res<u64> {
    let mut tx = db.begin().await?;
    let mut filed = 0u64;
    for (row, c) in rows {
        file_row(
            &mut tx,
            semester,
            c,
            assessment,
            exam_kind,
            exam_name,
            row.obtained,
            row.max,
        )
        .await?;
        filed += 1;
    }
    tx.commit().await?;
    Ok(filed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str, kind: PushKind) -> PushRow {
        let (rows, problems) = parse_push(text, kind);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(rows.len(), 1);
        rows.into_iter().next().unwrap()
    }

    #[test]
    fn university_rows_are_keyed_by_prn() {
        let row = one("2025BCA0001 BCA101 42/50", PushKind::University);
        assert_eq!(row.key, "2025BCA0001");
        assert_eq!(row.course, "BCA101");
        assert_eq!(row.assessment, "exam");
        assert_eq!(row.exam_name, "");
        assert_eq!((row.obtained, row.max), (42.0, 50.0));

        let row = one("2025BCA0001 BCA101 42 50", PushKind::University);
        assert_eq!((row.obtained, row.max), (42.0, 50.0));

        let row = one("2025BCA0001,BCA101,internal,18,25", PushKind::University);
        assert_eq!(row.assessment, "internal");
        assert_eq!((row.obtained, row.max), (18.0, 25.0));
    }

    #[test]
    fn internal_rows_are_keyed_by_admission_number_and_named() {
        let row = one("2501 BCA101 \"Internal 1\" 42/50", PushKind::Internal);
        assert_eq!(row.key, "2501");
        assert_eq!(row.course, "BCA101");
        assert_eq!(row.assessment, "exam");
        assert_eq!(row.exam_name, "Internal 1");
        assert_eq!((row.obtained, row.max), (42.0, 50.0));

        let row = one("2501,BCA101,Internal 2,42,50", PushKind::Internal);
        assert_eq!(row.exam_name, "Internal 2");
        assert_eq!((row.obtained, row.max), (42.0, 50.0));

        // The name may be left out; both shapes fall back to the same one.
        assert_eq!(one("2501 BCA101 42/50", PushKind::Internal).exam_name, "Internal exam");
        assert_eq!(one("2501 BCA101 42 50", PushKind::Internal).exam_name, "Internal exam");
    }

    #[test]
    fn header_rows_are_skipped_for_both_keys() {
        let (rows, problems) = parse_push(
            "admission_no,course,marks\n2501 BCA101 42/50",
            PushKind::Internal,
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(rows.len(), 1);

        let (rows, problems) = parse_push(
            "PRN,Course,Marks\n2025BCA0001 BCA101 42/50",
            PushKind::University,
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn a_bad_line_is_a_problem_not_a_silent_drop() {
        let (rows, problems) = parse_push("2501 BCA101 42/50\n2501 BCA101", PushKind::Internal);
        assert_eq!(rows.len(), 1);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].line, 2);
    }

    #[test]
    fn labels_tell_the_two_exams_apart() {
        assert_eq!(assessment_label("exam", "university", ""), "University exam");
        assert_eq!(
            assessment_label("exam", "internal", "Internal 1"),
            "Internal exam — Internal 1"
        );
        assert_eq!(assessment_label("exam", "internal", ""), "Internal exam");
        assert_eq!(assessment_label("internal", "coursework", ""), "Internal");
        assert_eq!(assessment_label("assignment", "coursework", ""), "Assignment");
    }

    #[test]
    fn csv_parsing_university() {
        let csv = b"prn,course,assessment,obtained,max\n2025BCA0001,BCA101,exam,42,50\n2025BCA0002,BCA101,assignment,18,25";
        let rows = parse_csv(csv, PushKind::University).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key, "2025BCA0001");
        assert_eq!(rows[0].course, "BCA101");
        assert_eq!((rows[0].obtained, rows[0].max), (42.0, 50.0));
        assert_eq!(rows[1].key, "2025BCA0002");
        assert_eq!(rows[1].assessment, "assignment");
    }

    #[test]
    fn csv_parsing_internal() {
        let csv = b"admission_no,course,exam_name,obtained,max\n2501,BCA101,Internal 1,42,50\n2502,BCA101,Internal 2,38,50";
        let rows = parse_csv(csv, PushKind::Internal).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key, "2501");
        assert_eq!(rows[0].exam_name, "Internal 1");
        assert_eq!((rows[0].obtained, rows[0].max), (42.0, 50.0));
    }

    #[test]
    fn csv_header_is_skipped() {
        let csv = b"prn,course,marks\n2025BCA0001,BCA101,42/50";
        let rows = parse_csv(csv, PushKind::University).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "2025BCA0001");
    }

    #[test]
    fn csv_invalid_utf8_is_rejected() {
        let csv = vec![0xFF, 0xFE, 0xFD];
        let result = parse_csv(&csv, PushKind::University);
        assert!(result.is_err());
    }
}
