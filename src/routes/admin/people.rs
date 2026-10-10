use std::collections::HashSet;

use askama::Template;
use axum::{
    extract::{Form, Multipart, Path, Query, State},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{parse_i32, parse_i64};
use crate::{
    auth::{csrf, password, AdminOnly, AuthUser},
    error::{internal, is_unique_violation, AppError},
    services::{
        academics::{self, ProgrammeOption},
        import_students::{self, Batch, CredentialRow, RowEdit, StagedRow},
        people::{self, NewStudent, NewTeacher, PersonDetail, PersonRow},
        users,
    },
    shell::{self, Shell},
    state::AppState,
    uploads,
};

// ---------- List ----------

#[derive(Deserialize)]
pub struct ListQuery {
    role: Option<String>,
    q: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/people.html")]
pub struct PeopleTemplate {
    shell: Shell,
    role: String,
    q: String,
    people: Vec<PersonRow>,
    /// Offered by the end-of-term promotion panel.
    programmes: Vec<ProgrammeOption>,
}

pub async fn list(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Query(query): Query<ListQuery>,
) -> Result<PeopleTemplate, AppError> {
    let role = match query.role.as_deref() {
        Some(r @ ("student" | "faculty" | "staff" | "admin")) => r.to_string(),
        _ => String::new(),
    };
    let q = query.q.unwrap_or_default().trim().to_string();
    Ok(PeopleTemplate {
        shell: Shell::build(&user, &session).await?,
        people: people::list(&s.db, &role, &q).await?,
        role,
        q,
        programmes: academics::programme_options(&s.db).await?,
    })
}

// ---------- Create / edit form ----------

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct PersonForm {
    csrf_token: String,
    kind: String,
    full_name: String,
    email: String,
    admission_no: String,
    prn: String,
    phone: String,
    programme_id: String,
    batch_year: String,
    semester: String,
    department_id: String,
    designation: String,
    qualification: String,
    is_hod: Option<String>,
    can_manage: Option<String>,
}

impl PersonForm {
    fn blank(kind: &str) -> Self {
        PersonForm {
            kind: kind.to_string(),
            semester: "1".into(),
            ..Default::default()
        }
    }

    fn from_detail(d: &PersonDetail) -> Self {
        let kind = match d.role.as_str() {
            "student" => "student",
            "faculty" => "teacher",
            _ => "staff",
        };
        PersonForm {
            csrf_token: String::new(),
            kind: kind.into(),
            full_name: d.full_name.clone(),
            email: d.email.clone(),
            admission_no: d.admission_no.clone(),
            prn: d.prn.clone(),
            phone: d.phone.clone(),
            programme_id: d.programme_id.to_string(),
            batch_year: if d.batch_year > 0 { d.batch_year.to_string() } else { String::new() },
            semester: d.semester.to_string(),
            department_id: d.department_id.to_string(),
            designation: d.designation.clone(),
            qualification: d.qualification.clone(),
            is_hod: d.is_hod.then(|| "on".to_string()),
            can_manage: d.can_manage.then(|| "on".to_string()),
        }
    }
}

pub struct DepartmentOption {
    pub id: i64,
    pub name: String,
}

#[derive(Template)]
#[template(path = "admin/person_form.html")]
pub struct PersonFormTemplate {
    shell: Shell,
    is_new: bool,
    user_id: i64,
    is_active: bool,
    is_self: bool,
    form: PersonForm,
    programmes: Vec<ProgrammeOption>,
    departments: Vec<DepartmentOption>,
    error: Option<String>,
    /// The semester an existing student would move into, or None when there is
    /// no promotion on offer (new form, or not a student).
    next_semester: Option<i32>,
}

/// Promotion is offered on an existing student who is not already at the top
/// semester the edit form knows about.
fn next_semester(is_new: bool, form: &PersonForm) -> Option<i32> {
    if is_new || form.kind != "student" {
        return None;
    }
    form.semester
        .parse::<i32>()
        .ok()
        .filter(|n| (1..people::MAX_SEMESTER).contains(n))
        .map(|n| n + 1)
}

async fn departments(db: &sqlx::PgPool) -> Result<Vec<DepartmentOption>, AppError> {
    let rows: Vec<(i64, String)> = sqlx::query_as("SELECT id, name FROM departments ORDER BY sort_order, name")
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(|(id, name)| DepartmentOption { id, name }).collect())
}

#[derive(Deserialize)]
pub struct NewQuery {
    kind: Option<String>,
}

fn clean_kind(kind: &str) -> &'static str {
    match kind {
        "teacher" => "teacher",
        "staff" => "staff",
        _ => "student",
    }
}

pub async fn new_form(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Query(q): Query<NewQuery>,
) -> Result<PersonFormTemplate, AppError> {
    let kind = clean_kind(q.kind.as_deref().unwrap_or("student"));
    Ok(PersonFormTemplate {
        shell: Shell::build(&user, &session).await?,
        is_new: true,
        user_id: 0,
        is_active: true,
        is_self: false,
        form: PersonForm::blank(kind),
        next_semester: None,
        programmes: academics::programme_options(&s.db).await?,
        departments: departments(&s.db).await?,
        error: None,
    })
}

pub async fn edit_form(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
) -> Result<PersonFormTemplate, AppError> {
    let d = people::detail(&s.db, id).await?.ok_or(AppError::NotFound)?;
    let form = PersonForm::from_detail(&d);
    let next = next_semester(false, &form);
    Ok(PersonFormTemplate {
        shell: Shell::build(&user, &session).await?,
        is_new: false,
        user_id: d.id,
        is_active: d.is_active,
        is_self: d.id == user.id,
        form,
        next_semester: next,
        programmes: academics::programme_options(&s.db).await?,
        departments: departments(&s.db).await?,
        error: None,
    })
}

/// Validated values common to create and update.
struct Checked {
    kind: &'static str,
    name: String,
    email: String,
    admission_no: String,
    prn: String,
    phone: String,
    programme_id: i64,
    batch_year: i32,
    semester: i32,
    department_id: i64,
    designation: String,
    qualification: String,
    is_hod: bool,
    can_manage: bool,
}

fn check(f: &PersonForm) -> Result<Checked, String> {
    let kind = clean_kind(&f.kind);
    let name = f.full_name.trim().to_string();
    if name.is_empty() {
        return Err("Enter the person's full name.".into());
    }
    let mut email = f.email.trim().to_string();
    let admission_no = f.admission_no.trim().to_string();
    let mut programme_id = 0;
    let mut batch_year = 0;
    let mut semester = 1;

    match kind {
        "student" => {
            if admission_no.is_empty() {
                return Err("Enter the admission number.".into());
            }
            programme_id = parse_i64(&f.programme_id).filter(|v| *v > 0).ok_or("Choose a programme.")?;
            batch_year = parse_i32(&f.batch_year)
                .filter(|y| (2000..=2100).contains(y))
                .ok_or("Enter the admission year, for example 2025.")?;
            semester = parse_i32(&f.semester)
                .filter(|v| (1..=8).contains(v))
                .ok_or("Semester must be between 1 and 8.")?;
            if email.is_empty() {
                // Students without an email still sign in with their admission number.
                email = format!("{}@students.college.local", admission_no.to_lowercase().replace(' ', ""));
            }
        }
        _ => {
            if !email.contains('@') {
                return Err("Enter a valid email address; it is the sign-in name.".into());
            }
        }
    }
    if kind == "student" && !email.contains('@') {
        return Err("The email address is not valid.".into());
    }
    Ok(Checked {
        kind,
        name,
        email,
        admission_no,
        prn: f.prn.trim().to_string(),
        phone: f.phone.trim().to_string(),
        programme_id,
        batch_year,
        semester,
        department_id: parse_i64(&f.department_id).unwrap_or(0),
        designation: f.designation.trim().to_string(),
        qualification: f.qualification.trim().to_string(),
        is_hod: f.is_hod.is_some(),
        can_manage: f.can_manage.is_some(),
    })
}

struct FormMeta {
    is_new: bool,
    user_id: i64,
    is_active: bool,
    is_self: bool,
}

async fn person_form_page(
    s: &AppState,
    session: &Session,
    user: &AuthUser,
    meta: FormMeta,
    form: PersonForm,
    error: Option<String>,
) -> Result<Response, AppError> {
    Ok(PersonFormTemplate {
        shell: Shell::build(user, session).await?,
        is_new: meta.is_new,
        user_id: meta.user_id,
        is_active: meta.is_active,
        is_self: meta.is_self,
        next_semester: next_semester(meta.is_new, &form),
        form,
        programmes: academics::programme_options(&s.db).await?,
        departments: departments(&s.db).await?,
        error,
    }
    .into_response())
}

// ---------- One-time credentials page ----------

#[derive(Template)]
#[template(path = "admin/credentials.html")]
pub struct CredentialsTemplate {
    shell: Shell,
    heading: String,
    rows: Vec<CredentialRow>,
    back_href: &'static str,
    back_label: &'static str,
    /// Set when these credentials can also be downloaded, as after a bulk import.
    download_href: Option<String>,
    /// Set when the generated passwords can be cleared from the server.
    finish_batch: Option<i64>,
}

// ---------- Create ----------

pub async fn create(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Form(f): Form<PersonForm>,
) -> Result<Response, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;

    let new_meta = || FormMeta { is_new: true, user_id: 0, is_active: true, is_self: false };

    let c = match check(&f) {
        Ok(c) => c,
        Err(msg) => return person_form_page(&s, &session, &user, new_meta(), f.clone(), Some(msg)).await,
    };

    let temp = password::temporary();
    let hash = password::hash_blocking(temp.clone()).await?;

    let created = match c.kind {
        "student" => {
            people::create_student(
                &s.db,
                &NewStudent {
                    admission_no: &c.admission_no,
                    prn: &c.prn,
                    name: &c.name,
                    email: &c.email,
                    programme_id: c.programme_id,
                    batch_year: c.batch_year,
                    semester: c.semester,
                    phone: &c.phone,
                },
                &hash,
            )
            .await
        }
        "teacher" => {
            people::create_teacher(
                &s.db,
                &NewTeacher {
                    name: &c.name,
                    email: &c.email,
                    department_id: c.department_id,
                    designation: &c.designation,
                    qualification: &c.qualification,
                    is_hod: c.is_hod,
                    can_manage: c.can_manage,
                },
                &hash,
            )
            .await
        }
        _ => people::create_staff(&s.db, &c.name, &c.email, &hash).await,
    };

    let new_id = match created {
        Ok(id) => id,
        Err(e) if is_unique_violation(&e) => {
            let msg = "That email or admission number is already used by another account.".to_string();
            return person_form_page(&s, &session, &user, new_meta(), f.clone(), Some(msg)).await;
        }
        Err(e) => return Err(e.into()),
    };
    users::audit(&s.db, Some(user.id), "user_created", "user", Some(new_id)).await?;

    let login = if c.kind == "student" { c.admission_no.clone() } else { c.email.clone() };
    Ok(CredentialsTemplate {
        shell: Shell::build(&user, &session).await?,
        heading: "Account created".into(),
        rows: vec![CredentialRow {
            name: c.name,
            login,
            password: temp,
            note: "Must choose a new password at first sign-in.".into(),
        }],
        back_href: "/admin/people",
        back_label: "Back to people",
        download_href: None,
        finish_batch: None,
    }
    .into_response())
}

// ---------- Update, reset password, activate ----------

pub async fn update(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
    Form(f): Form<PersonForm>,
) -> Result<Response, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let current = people::detail(&s.db, id).await?.ok_or(AppError::NotFound)?;

    let edit_meta = || FormMeta {
        is_new: false,
        user_id: current.id,
        is_active: current.is_active,
        is_self: current.id == user.id,
    };

    // The kind is fixed by the account's role; ignore whatever the form says.
    let mut f = f;
    f.kind = match current.role.as_str() {
        "student" => "student",
        "faculty" => "teacher",
        _ => "staff",
    }
    .into();
    let c = match check(&f) {
        Ok(c) => c,
        Err(msg) => return person_form_page(&s, &session, &user, edit_meta(), f.clone(), Some(msg)).await,
    };

    let updated = PersonDetail {
        id: current.id,
        full_name: c.name,
        email: c.email,
        role: current.role.clone(),
        is_active: current.is_active,
        admission_no: current.admission_no.clone(), // the admission number is not editable
        prn: f.prn.trim().to_string(),
        programme_id: c.programme_id,
        batch_year: c.batch_year,
        semester: c.semester,
        phone: c.phone,
        department_id: c.department_id,
        designation: c.designation,
        qualification: c.qualification,
        is_hod: c.is_hod,
        can_manage: c.can_manage,
    };
    if let Err(e) = people::update(&s.db, &updated).await {
        if is_unique_violation(&e) {
            let msg = "That email or PRN is already used by another account."
                .to_string();
            return person_form_page(&s, &session, &user, edit_meta(), f.clone(), Some(msg)).await;
        }
        return Err(e.into());
    }
    users::audit(&s.db, Some(user.id), "user_updated", "user", Some(id)).await?;
    shell::flash(&session, "Saved.").await?;
    Ok(Redirect::to("/admin/people").into_response())
}

#[derive(Deserialize)]
pub struct TokenForm {
    csrf_token: String,
    #[serde(default)]
    active: String,
}

pub async fn reset_password(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Response, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let d = people::detail(&s.db, id).await?.ok_or(AppError::NotFound)?;

    let temp = password::temporary();
    let hash = password::hash_blocking(temp.clone()).await?;
    users::set_temp_password(&s.db, id, &hash).await?;
    users::audit(&s.db, Some(user.id), "password_reset", "user", Some(id)).await?;

    let login = if d.admission_no.is_empty() { d.email.clone() } else { d.admission_no.clone() };
    Ok(CredentialsTemplate {
        shell: Shell::build(&user, &session).await?,
        heading: "New temporary password".into(),
        rows: vec![CredentialRow {
            name: d.full_name,
            login,
            password: temp,
            note: "Must choose a new password at next sign-in. Any lock-out was cleared.".into(),
        }],
        back_href: "/admin/people",
        back_label: "Back to people",
        download_href: None,
        finish_batch: None,
    }
    .into_response())
}

pub async fn set_active(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
    Form(f): Form<TokenForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    if id == user.id {
        shell::flash(&session, "You can't deactivate your own account.").await?;
        return Ok(Redirect::to("/admin/people"));
    }
    let active = f.active == "1";
    if active {
        people::set_active(&s.db, id, true).await?;
    } else {
        people::archive_user(&s.db, id).await?;
    }
    users::audit(
        &s.db,
        Some(user.id),
        if active { "user_activated" } else { "user_deactivated" },
        "user",
        Some(id),
    )
    .await?;
    shell::flash(&session, if active { "Account re-activated." } else { "Account deactivated. They can no longer sign in." }).await?;
    Ok(Redirect::to("/admin/people"))
}

// ---------- Promotion ----------

/// A form carrying nothing but the token, for a one-button action.
#[derive(Deserialize)]
pub struct CsrfForm {
    csrf_token: String,
}

/// The end-of-term roll: programme plus the semester to promote from.
#[derive(Deserialize)]
pub struct PromoteForm {
    csrf_token: String,
    programme: String,
    semester: String,
}

/// Moves one student into the next semester. Results and enrollments stay
/// filed under the semester they were earned in, so nothing leaves the page.
pub async fn promote_one(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Path(id): Path<i64>,
    Form(f): Form<CsrfForm>,
) -> Result<Redirect, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    match people::promote(&s.db, id).await? {
        people::Promoted::To(next) => {
            users::audit(&s.db, Some(user.id), "student_promoted", "user", Some(id)).await?;
            shell::flash(&session, format!("Moved to Semester {next}. Past results stay where they were earned.")).await?;
        }
        people::Promoted::AtEnd => {
            shell::flash(&session, "Already in the final semester, so there is nothing to promote.").await?;
        }
        people::Promoted::NotAStudent => {
            shell::flash(&session, "That account is not a student.").await?;
        }
    }
    Ok(Redirect::to(&format!("/admin/people/{id}")))
}

/// Promotes a whole class at the end of the term: every active student of the
/// programme sitting in the chosen semester moves up one.
pub async fn promote_cohort(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Form(f): Form<PromoteForm>,
) -> Result<Response, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    let programme_id = parse_i64(&f.programme).filter(|v| *v > 0);
    let semester = parse_i32(&f.semester).filter(|v| (1..people::MAX_SEMESTER).contains(v));
    let (Some(programme_id), Some(semester)) = (programme_id, semester) else {
        shell::flash(&session, "Choose a programme and a semester to promote from.").await?;
        return Ok(Redirect::to("/admin/people?role=student").into_response());
    };
    let moved = people::promote_cohort(&s.db, programme_id, semester).await?;
    users::audit(
        &s.db,
        Some(user.id),
        "students_promoted",
        "programme",
        Some(programme_id),
    )
    .await?;
    let msg = if moved == 0 {
        format!("No active student was sitting in semester {semester} — nothing moved.")
    } else {
        format!(
            "Promoted {moved} student(s) from semester {semester} to semester {}.",
            semester + 1
        )
    };
    shell::flash(&session, msg).await?;
    Ok(Redirect::to("/admin/people?role=student").into_response())
}

// ---------- File import of students ----------
//
// Four steps: choose a file, read it into staged rows, review and edit those
// rows, then commit the ticked ones. Accounts are only created on the last
// step, so a bad file never leaves half a class behind.

const MAX_IMPORT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Template)]
#[template(path = "admin/import.html")]
pub struct ImportTemplate {
    shell: Shell,
    programmes: Vec<ProgrammeOption>,
    error: Option<String>,
    programme_id: String,
    semester: String,
    batch_year: String,
    pending: Option<i64>,
}

pub async fn import_form(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
) -> Result<Response, AppError> {
    Ok(upload_page(&s, &session, &user, None, "", "1", "").await?.into_response())
}

/// The upload form, redrawn with whatever was already chosen.
async fn upload_page(
    s: &AppState,
    session: &Session,
    user: &AuthUser,
    error: Option<&str>,
    programme_id: &str,
    semester: &str,
    batch_year: &str,
) -> Result<ImportTemplate, AppError> {
    Ok(ImportTemplate {
        shell: Shell::build(user, session).await?,
        programmes: academics::programme_options(&s.db).await?,
        error: error.map(str::to_string),
        programme_id: programme_id.to_string(),
        semester: semester.to_string(),
        batch_year: batch_year.to_string(),
        pending: import_students::latest_batch(&s.db).await?.map(|b| b.id),
    })
}

/// Redraws the upload form with a message instead of giving up on the file.
async fn upload_retry(
    s: &AppState,
    session: &Session,
    user: &AuthUser,
    body: &uploads::ParsedForm,
    msg: &str,
) -> Result<Response, AppError> {
    Ok(upload_page(
        s,
        session,
        user,
        Some(msg),
        body.field("programme_id"),
        body.field("semester"),
        body.field("batch_year"),
    )
    .await?
    .into_response())
}

pub async fn import_upload(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let body = uploads::read(multipart).await?;
    csrf::verify(&session, body.field("csrf_token")).await?;

    let programme_id = parse_i64(body.field("programme_id")).filter(|v| *v > 0);
    let semester = parse_i32(body.field("semester"));
    let batch_year = parse_i32(body.field("batch_year"));

    // Check the batch defaults before spending any time on the file itself.
    let problem = if programme_id.is_none() {
        Some("Choose a programme.")
    } else if !semester.is_some_and(|v| (1..=12).contains(&v)) {
        Some("Semester must be between 1 and 12.")
    } else if !batch_year.is_some_and(|y| (2000..=2100).contains(&y)) {
        Some("Enter the admission year, for example 2025.")
    } else {
        None
    };
    if let Some(msg) = problem {
        return upload_retry(&s, &session, &user, &body, msg).await;
    }

    let Some(file) = body.file("list") else {
        return upload_retry(&s, &session, &user, &body, "Choose a CSV or Excel file to upload.").await;
    };
    if file.bytes.len() > MAX_IMPORT_BYTES {
        return upload_retry(&s, &session, &user, &body, "That file is larger than 8 MB. Split it into smaller batches.").await;
    }
    let parsed = match import_students::parse(&file.bytes, &file.filename) {
        Ok(parsed) => parsed,
        Err(msg) => return upload_retry(&s, &session, &user, &body, &msg).await,
    };

    // Long-dead batches still hold their one-time passwords; drop them.
    let _ = import_students::purge_stale(&s.db).await;
    if parsed.dropped > 0 {
        shell::flash(
            &session,
            &format!(
                "Read the first {} rows. The other {} were left out, so import the rest in a second batch.",
                import_students::MAX_ROWS,
                parsed.dropped
            ),
        )
        .await?;
    }

    let batch_id = import_students::stage(
        &s.db,
        &file.filename,
        programme_id.unwrap(),
        semester.unwrap(),
        batch_year.unwrap(),
        Some(user.id),
        &parsed.rows,
    )
    .await?;
    users::audit(&s.db, Some(user.id), "import_staged", "import_batch", Some(batch_id)).await?;
    Ok(Redirect::to("/admin/people/import/review").into_response())
}

// ---------- Review ----------

#[derive(Deserialize)]
pub struct ReviewQuery {
    batch: Option<i64>,
}

#[derive(Template)]
#[template(path = "admin/import_review.html")]
pub struct ImportReviewTemplate {
    shell: Shell,
    batch: Batch,
    rows: Vec<StagedRow>,
    programme_names: Vec<String>,
    ready: usize,
    faulty: usize,
    created: usize,
    error: Option<String>,
}

pub async fn import_review(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Query(q): Query<ReviewQuery>,
) -> Result<Response, AppError> {
    let Some(batch) = review_batch(&s, q.batch).await? else {
        return Ok(Redirect::to("/admin/people/import").into_response());
    };
    Ok(review_page(&s, &session, &user, batch, None).await?.into_response())
}

async fn review_batch(s: &AppState, id: Option<i64>) -> Result<Option<Batch>, AppError> {
    Ok(match id {
        Some(id) => import_students::batch(&s.db, id).await?,
        None => import_students::latest_batch(&s.db).await?,
    })
}

async fn review_page(
    s: &AppState,
    session: &Session,
    user: &AuthUser,
    batch: Batch,
    error: Option<String>,
) -> Result<ImportReviewTemplate, AppError> {
    let rows = import_students::rows(&s.db, batch.id).await?;
    let (mut ready, mut faulty, mut created) = (0, 0, 0);
    for r in &rows {
        if r.is_created() {
            created += 1;
        } else if r.problems().is_empty() {
            ready += 1;
        } else {
            faulty += 1;
        }
    }
    let programme_names = import_students::programme_names(&s.db).await?;
    Ok(ImportReviewTemplate {
        shell: Shell::build(user, session).await?,
        batch,
        rows,
        programme_names,
        ready,
        faulty,
        created,
        error,
    })
}



/// One editable column of the review table, as it arrives from the form.
const EDIT_COLUMNS: [&str; 8] = [
    "admission_no",
    "prn",
    "name",
    "email",
    "phone",
    "programme_text",
    "semester_text",
    "year_text",
];

const BAD_TABLE: &str = "The review table came back incomplete, so nothing was changed. Reload the page and try again.";

/// Every button on the review page posts here, as multipart so that the table's
/// repeated inputs survive the trip.
pub async fn import_review_submit(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let form = uploads::read(multipart).await?;
    csrf::verify(&session, form.field("csrf_token")).await?;

    let Some(batch_id) = parse_i64(form.field("batch_id")).filter(|v| *v > 0) else {
        return Err(AppError::BadRequest(BAD_TABLE.into()));
    };
    let Some(batch) = import_students::batch(&s.db, batch_id).await? else {
        return Ok(Redirect::to("/admin/people/import").into_response());
    };

    match form.field("intent") {
        "discard" => {
            import_students::discard(&s.db, batch.id).await?;
            let _ = users::audit(&s.db, Some(user.id), "import_discarded", "import_batch", Some(batch.id)).await;
            shell::flash(&session, "Import discarded. Nothing was saved.").await?;
            return Ok(Redirect::to("/admin/people/import").into_response());
        }
        "add" => {
            import_students::add_row(&s.db, batch.id).await?;
            return Ok(Redirect::to("/admin/people/import/review").into_response());
        }
        how @ ("all" | "none" | "valid") => {
            import_students::set_all(&s.db, batch.id, how).await?;
            return Ok(Redirect::to("/admin/people/import/review").into_response());
        }
        _ => {}
    }

    // Save what is on screen first, so what was ticked is what gets committed.
    match collect_edits(&form) {
        Some(edits) => import_students::save_rows(&s.db, batch.id, &edits).await?,
        None => {
            return Ok(review_page(&s, &session, &user, batch, Some(BAD_TABLE.into()))
                .await?
                .into_response())
        }
    }
    for id in form.repeated("remove") {
        if let Some(id) = parse_i64(id) {
            import_students::delete_row(&s.db, batch.id, id).await?;
        }
    }

    if form.field("intent") == "commit" {
        return commit(&s, &session, &user, batch.id).await;
    }
    shell::flash(&session, "Changes saved.").await?;
    Ok(Redirect::to("/admin/people/import/review").into_response())
}

/// Pairs the repeated fields into one edit per row, or `None` if the columns do
/// not line up, which means the form cannot be trusted.
fn collect_edits(form: &uploads::ParsedForm) -> Option<Vec<RowEdit>> {
    let ids = form.repeated("row_id");
    let columns: Vec<&[String]> = EDIT_COLUMNS.iter().map(|c| form.repeated(c)).collect();
    if ids.is_empty() || columns.iter().any(|c| c.len() != ids.len()) {
        return None;
    }
    let ticked = |name: &str| -> Vec<bool> {
        let ticked: Vec<&String> = form.repeated(name).iter().collect();
        ids.iter().map(|id| ticked.iter().any(|v| *v == id)).collect()
    };
    let include = ticked("include");

    (0..ids.len())
        .map(|i| {
            let id = parse_i64(&ids[i])?;
            Some(RowEdit {
                id,
                admission_no: columns[0][i].clone(),
                prn: columns[1][i].clone(),
                name: columns[2][i].clone(),
                email: columns[3][i].clone(),
                phone: columns[4][i].clone(),
                programme_text: columns[5][i].clone(),
                semester_text: columns[6][i].clone(),
                year_text: columns[7][i].clone(),
                include: include[i],
            })
        })
        .collect()
}
// ---------- Commit ----------

async fn commit(
    s: &AppState,
    session: &Session,
    user: &AuthUser,
    batch_id: i64,
) -> Result<Response, AppError> {
    let rows = import_students::rows(&s.db, batch_id).await?;
    let wanted: Vec<&StagedRow> = rows.iter().filter(|r| r.ready()).collect();

    // One lookup each for the whole batch rather than a query per row.
    let admission_nos: Vec<String> = wanted.iter().map(|r| r.admission_no.to_lowercase()).collect();
    let emails: Vec<String> = wanted
        .iter()
        .filter(|r| !r.email.trim().is_empty())
        .map(|r| r.email.to_lowercase())
        .collect();
    let taken_admission = taken(&s.db, "SELECT lower(admission_no) FROM students WHERE lower(admission_no) = ANY($1)", &admission_nos).await?;
    let taken_email = taken(&s.db, "SELECT lower(email) FROM users WHERE lower(email) = ANY($1)", &emails).await?;
    // The PRN is the university's candidate key, so it carries a unique index.
    let prns: Vec<String> = wanted
        .iter()
        .filter(|r| !r.prn.trim().is_empty())
        .map(|r| r.prn.trim().to_lowercase())
        .collect();
    let taken_prn = taken(
        &s.db,
        "SELECT lower(prn) FROM students WHERE prn IS NOT NULL AND lower(prn) = ANY($1)",
        &prns,
    )
    .await?;

    let mut seen: HashSet<String> = HashSet::new();
    let mut seen_prn: HashSet<String> = HashSet::new();
    let mut creds = Vec::new();
    let (mut created, mut skipped) = (0usize, 0usize);

    for row in &wanted {
        let admission_no = row.admission_no.trim().to_string();
        let prn = row.prn.trim().to_string();
        let name = row.name.trim().to_string();

        // A row that cannot be created is reported and stepped over, never fatal.
        let skip = |reason: &str| CredentialRow {
            name: name.clone(),
            login: admission_no.clone(),
            password: String::new(),
            note: reason.to_string(),
        };

        if !seen.insert(admission_no.to_lowercase()) {
            skipped += 1;
            let reason = "Skipped: that admission number appears twice in the file.";
            mark_skipped(s, row.id, reason).await?;
            creds.push(skip(reason));
            continue;
        }
        if taken_admission.contains(&admission_no.to_lowercase()) {
            skipped += 1;
            let reason = "Skipped: that admission number already exists.";
            mark_skipped(s, row.id, reason).await?;
            creds.push(skip(reason));
            continue;
        }
        if !prn.is_empty() && !seen_prn.insert(prn.to_lowercase()) {
            skipped += 1;
            let reason = "Skipped: that PRN appears twice in the file.";
            mark_skipped(s, row.id, reason).await?;
            creds.push(skip(reason));
            continue;
        }
        if !prn.is_empty() && taken_prn.contains(&prn.to_lowercase()) {
            skipped += 1;
            let reason = "Skipped: that PRN already exists.";
            mark_skipped(s, row.id, reason).await?;
            creds.push(skip(reason));
            continue;
        }
        let given_email = row.email.trim();
        if !given_email.is_empty() && taken_email.contains(&given_email.to_lowercase()) {
            skipped += 1;
            let reason = "Skipped: that email address is already in use.";
            mark_skipped(s, row.id, reason).await?;
            creds.push(skip(reason));
            continue;
        }
        // Students sign in with their admission number, so the address only has
        // to be unique. Take a suffixed one rather than lose the row.
        let Some(email) = unique_email(&s.db, &import_students::placeholder_email(&admission_no)).await? else {
            skipped += 1;
            let reason = "Skipped: no free sign-in email could be made for this admission number.";
            mark_skipped(s, row.id, reason).await?;
            creds.push(skip(reason));
            continue;
        };

        let temp = password::temporary();
        let hash = password::hash_blocking(temp.clone()).await?;
        let outcome = people::create_student(
            &s.db,
            &NewStudent {
                admission_no: &admission_no,
                prn: &prn,
                name: &name,
                email: &email,
                programme_id: row.programme_id().unwrap_or_default(),
                batch_year: row.year().unwrap_or_default(),
                semester: row.semester().unwrap_or_default(),
                phone: row.phone.trim(),
            },
            &hash,
        )
        .await;
        match outcome {
            Ok(user_id) => {
                created += 1;
                let note = "Created. Must choose a new password at first sign-in.";
                import_students::mark(&s.db, row.id, "created", note, &temp, Some(user_id)).await?;
                let _ = users::audit(&s.db, Some(user.id), "user_created", "user", Some(user_id)).await;
                creds.push(CredentialRow { name, login: admission_no, password: temp, note: note.into() });
            }
            Err(e) if is_unique_violation(&e) => {
                skipped += 1;
                let reason = "Skipped: that admission number, PRN or email already exists.";
                mark_skipped(s, row.id, reason).await?;
                creds.push(skip(reason));
            }
            Err(e) => return Err(e.into()),
        }
    }

    let batch = import_students::batch(&s.db, batch_id).await?;
    let _ = users::audit(
        &s.db,
        Some(user.id),
        "import_committed",
        "import_batch",
        Some(batch_id),
    )
    .await;
    let mut heading = format!("Import finished: {created} account(s) created");
    if skipped > 0 {
        heading.push_str(&format!(", {skipped} skipped"));
    }
    Ok(CredentialsTemplate {
        shell: Shell::build(user, session).await?,
        heading,
        rows: creds,
        back_href: "/admin/people",
        back_label: "Back to people",
        download_href: match &batch {
            Some(_) => Some(format!("/admin/people/import/credentials.csv?batch={batch_id}")),
            None => None,
        },
        finish_batch: match &batch {
            Some(_) => Some(batch_id),
            None => None,
        },
    }
    .into_response())
}

async fn mark_skipped(s: &AppState, row_id: i64, reason: &str) -> Result<(), AppError> {
    import_students::mark(&s.db, row_id, "skipped", reason, "", None).await?;
    Ok(())
}

/// The lower-cased values in `column` that are already present in `table`.
async fn taken(db: &PgPool, table: &str, values: &[String]) -> Result<HashSet<String>, AppError> {
    if values.is_empty() {
        return Ok(HashSet::new());
    }
    let found: Vec<String> = sqlx::query_scalar(table).bind(values).fetch_all(db).await?;
    Ok(found.into_iter().collect())
}

/// `base` if it is free, otherwise the first `base2`, `base3`... that is not.
async fn unique_email(db: &PgPool, base: &str) -> Result<Option<String>, AppError> {
    for n in 1..=50 {
        let candidate = if n == 1 { base.to_string() } else { format!("{base}{n}") };
        let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE lower(email) = lower($1))")
            .bind(&candidate)
            .fetch_one(db)
            .await?;
        if !exists {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

// ---------- Results ----------

pub async fn import_credentials_csv(
    State(s): State<AppState>,
    user: AdminOnly,
    Query(q): Query<ReviewQuery>,
) -> Result<Response, AppError> {
    let Some(batch) = review_batch(&s, q.batch).await? else {
        return Err(AppError::NotFound);
    };
    let rows = import_students::credentials(&s.db, batch.id).await?;
    let bytes = import_students::credentials_csv(&rows)
        .map_err(|e| internal(format!("Could not build the credentials file: {e}")))?;
    let name = credentials_name(&batch.source_name);
    Ok((
        [
            (
                axum::http::header::CONTENT_TYPE,
                "text/csv; charset=utf-8".to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        bytes,
    )
        .into_response())
}

/// A safe download name based on the uploaded file, e.g. "class-list-credentials.csv".
fn credentials_name(source: &str) -> String {
    let stem: String = source
        .rsplit('/')
        .next()
        .unwrap_or("import")
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or("import")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let stem = if stem.trim_matches('-').is_empty() { "import".to_string() } else { stem };
    format!("{stem}-credentials.csv")
}

/// Once the passwords have been handed over, drop the batch so the temporary
/// ones stop sitting on the server.
pub async fn import_finished(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Form(f): Form<FinishForm>,
) -> Result<Response, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;
    if let Some(batch) = import_students::batch(&s.db, f.batch).await? {
        import_students::discard(&s.db, batch.id).await?;
        let _ = users::audit(&s.db, Some(user.id), "import_discarded", "import_batch", Some(batch.id)).await;
    }
    shell::flash(&session, "Import cleared. The passwords are gone from the server.").await?;
    Ok(Redirect::to("/admin/people").into_response())
}

#[derive(Deserialize)]
pub struct FinishForm {
    csrf_token: String,
    batch: i64,
}

// ---------- Bulk delete ----------

/// The ticked rows arrive as repeated `ids` fields, and serde_urlencoded hands
/// a struct field one value as a string rather than a list, so no typed body
/// can carry them (a `Vec` field fails outright and an untagged enum never
/// matches a string). The pairs are read the way the attendance and department
/// sheets read theirs: `Form<Vec<(String, String)>>`.
pub async fn bulk_delete(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Form(fields): Form<Vec<(String, String)>>,
) -> Result<Redirect, AppError> {
    let token = fields
        .iter()
        .find(|(k, _)| k == "csrf_token")
        .map(|(_, v)| v.as_str())
        .unwrap_or("");
    csrf::verify(&session, token).await?;

    let ids: Vec<i64> = fields
        .iter()
        .filter(|(k, _)| k == "ids")
        .filter_map(|(_, v)| v.trim().parse::<i64>().ok())
        .collect();
    if ids.is_empty() {
        shell::flash(&session, "No one selected.").await?;
        return Ok(Redirect::to("/admin/people"));
    }
    let archived = people::archive_users(&s.db, &ids, user.id).await?;
    if archived > 0 {
        users::audit(&s.db, Some(user.id), "users_archived", "user", None).await?;
    }
    shell::flash(&session, format!("Archived {archived} account(s); academic history was preserved.")).await?;
    Ok(Redirect::to("/admin/people"))
}
