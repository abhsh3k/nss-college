use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect, Response},
    Form,
};
use serde::Deserialize;
use tower_sessions::Session;

use super::{parse_i32, parse_i64};
use crate::{
    auth::{csrf, password, AdminOnly, AuthUser},
    error::{is_unique_violation, AppError},
    services::{
        academics::{self, ProgrammeOption},
        people::{self, NewStudent, NewTeacher, PersonDetail, PersonRow},
        users,
    },
    shell::{self, Shell},
    state::AppState,
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
    phone: String,
    programme_id: String,
    batch_year: String,
    semester: String,
    egrants: Option<String>,
    department_id: String,
    designation: String,
    qualification: String,
    is_hod: Option<String>,
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
            phone: d.phone.clone(),
            programme_id: d.programme_id.to_string(),
            batch_year: if d.batch_year > 0 { d.batch_year.to_string() } else { String::new() },
            semester: d.semester.to_string(),
            egrants: d.egrants.then(|| "on".to_string()),
            department_id: d.department_id.to_string(),
            designation: d.designation.clone(),
            qualification: d.qualification.clone(),
            is_hod: d.is_hod.then(|| "on".to_string()),
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
    Ok(PersonFormTemplate {
        shell: Shell::build(&user, &session).await?,
        is_new: false,
        user_id: d.id,
        is_active: d.is_active,
        is_self: d.id == user.id,
        form: PersonForm::from_detail(&d),
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
    phone: String,
    programme_id: i64,
    batch_year: i32,
    semester: i32,
    egrants: bool,
    department_id: i64,
    designation: String,
    qualification: String,
    is_hod: bool,
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
        phone: f.phone.trim().to_string(),
        programme_id,
        batch_year,
        semester,
        egrants: f.egrants.is_some(),
        department_id: parse_i64(&f.department_id).unwrap_or(0),
        designation: f.designation.trim().to_string(),
        qualification: f.qualification.trim().to_string(),
        is_hod: f.is_hod.is_some(),
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
        form,
        programmes: academics::programme_options(&s.db).await?,
        departments: departments(&s.db).await?,
        error,
    }
    .into_response())
}

// ---------- One-time credentials page ----------

pub struct CredentialRow {
    pub name: String,
    pub login: String,
    pub password: String,
    pub note: String,
}

#[derive(Template)]
#[template(path = "admin/credentials.html")]
pub struct CredentialsTemplate {
    shell: Shell,
    heading: String,
    rows: Vec<CredentialRow>,
    back_href: &'static str,
    back_label: &'static str,
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
                    name: &c.name,
                    email: &c.email,
                    programme_id: c.programme_id,
                    batch_year: c.batch_year,
                    semester: c.semester,
                    phone: &c.phone,
                    egrants: c.egrants,
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
        programme_id: c.programme_id,
        batch_year: c.batch_year,
        semester: c.semester,
        phone: c.phone,
        egrants: c.egrants,
        department_id: c.department_id,
        designation: c.designation,
        qualification: c.qualification,
        is_hod: c.is_hod,
    };
    if let Err(e) = people::update(&s.db, &updated).await {
        if is_unique_violation(&e) {
            let msg = "That email is already used by another account.".to_string();
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
    people::set_active(&s.db, id, active).await?;
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

// ---------- Bulk import of students ----------

#[derive(Template)]
#[template(path = "admin/import.html")]
pub struct ImportTemplate {
    shell: Shell,
    programmes: Vec<ProgrammeOption>,
    error: Option<String>,
    programme_id: String,
    semester: String,
    batch_year: String,
    csv_text: String,
}

pub async fn import_form(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
) -> Result<ImportTemplate, AppError> {
    Ok(ImportTemplate {
        shell: Shell::build(&user, &session).await?,
        programmes: academics::programme_options(&s.db).await?,
        error: None,
        programme_id: String::new(),
        semester: "1".into(),
        batch_year: String::new(),
        csv_text: String::new(),
    })
}

#[derive(Deserialize)]
pub struct ImportForm {
    csrf_token: String,
    programme_id: String,
    semester: String,
    batch_year: String,
    csv_text: String,
}

async fn import_page(
    s: &AppState,
    session: &Session,
    user: &AuthUser,
    f: &ImportForm,
    error: String,
) -> Result<Response, AppError> {
    Ok(ImportTemplate {
        shell: Shell::build(user, session).await?,
        programmes: academics::programme_options(&s.db).await?,
        error: Some(error),
        programme_id: f.programme_id.clone(),
        semester: f.semester.clone(),
        batch_year: f.batch_year.clone(),
        csv_text: f.csv_text.clone(),
    }
    .into_response())
}

const MAX_IMPORT_ROWS: usize = 500;

pub async fn import_submit(
    State(s): State<AppState>,
    session: Session,
    AdminOnly(user): AdminOnly,
    Form(f): Form<ImportForm>,
) -> Result<Response, AppError> {
    csrf::verify(&session, &f.csrf_token).await?;

    let Some(programme_id) = parse_i64(&f.programme_id).filter(|v| *v > 0) else {
        return import_page(&s, &session, &user, &f, "Choose a programme.".into()).await;
    };
    let Some(semester) = parse_i32(&f.semester).filter(|v| (1..=8).contains(v)) else {
        return import_page(&s, &session, &user, &f, "Semester must be between 1 and 8.".into()).await;
    };
    let Some(batch_year) = parse_i32(&f.batch_year).filter(|y| (2000..=2100).contains(y)) else {
        return import_page(&s, &session, &user, &f, "Enter the admission year, for example 2025.".into()).await;
    };

    // Rows copied from Excel are tab-separated; typed lists are comma-separated.
    let first_line = f.csv_text.trim().lines().next().unwrap_or("");
    let delimiter = if first_line.contains('\t') { b'\t' } else { b',' };
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(f.csv_text.trim().as_bytes());
    let headers: Vec<String> = match reader.headers() {
        Ok(h) => h.iter().map(|x| x.trim().to_lowercase()).collect(),
        Err(_) => {
            return import_page(&s, &session, &user, &f, "Could not read the first line. Paste the table with a header row.".into()).await
        }
    };
    let col = |name: &str| headers.iter().position(|h| h == name);
    let (Some(c_adm), Some(c_name)) = (col("admission_no"), col("name")) else {
        return import_page(&s, &session, &user, &f, "The first line must contain the columns admission_no and name (email, phone and egrants are optional).".into()).await;
    };
    let (c_email, c_phone, c_egrants) = (col("email"), col("phone"), col("egrants"));

    let mut rows: Vec<CredentialRow> = Vec::new();
    let mut created = 0usize;
    for (i, record) in reader.records().enumerate() {
        if i >= MAX_IMPORT_ROWS {
            rows.push(CredentialRow {
                name: String::new(),
                login: String::new(),
                password: String::new(),
                note: format!("Stopped after {MAX_IMPORT_ROWS} rows. Import the rest in a second batch."),
            });
            break;
        }
        let Ok(rec) = record else {
            rows.push(CredentialRow { name: String::new(), login: format!("line {}", i + 2), password: String::new(), note: "Could not read this line.".into() });
            continue;
        };
        let get = |c: Option<usize>| c.and_then(|c| rec.get(c)).unwrap_or("").trim().to_string();
        let admission_no = get(Some(c_adm));
        let name = get(Some(c_name));
        let mut email = get(c_email);
        let phone = get(c_phone);
        let egrants = matches!(get(c_egrants).to_lowercase().as_str(), "yes" | "y" | "true" | "1");

        if admission_no.is_empty() || name.is_empty() {
            rows.push(CredentialRow { name, login: admission_no, password: String::new(), note: "Skipped: admission_no and name are both required.".into() });
            continue;
        }
        if email.is_empty() {
            email = format!("{}@students.college.local", admission_no.to_lowercase().replace(' ', ""));
        } else if !email.contains('@') {
            rows.push(CredentialRow { name, login: admission_no, password: String::new(), note: "Skipped: the email address is not valid.".into() });
            continue;
        }

        let temp = password::temporary();
        let hash = password::hash_blocking(temp.clone()).await?;
        let result = people::create_student(
            &s.db,
            &NewStudent {
                admission_no: &admission_no,
                name: &name,
                email: &email,
                programme_id,
                batch_year,
                semester,
                phone: &phone,
                egrants,
            },
            &hash,
        )
        .await;
        match result {
            Ok(new_id) => {
                created += 1;
                let _ = users::audit(&s.db, Some(user.id), "user_created", "user", Some(new_id)).await;
                rows.push(CredentialRow { name, login: admission_no, password: temp, note: "Created.".into() });
            }
            Err(e) if is_unique_violation(&e) => rows.push(CredentialRow {
                name,
                login: admission_no,
                password: String::new(),
                note: "Skipped: that admission number or email already exists.".into(),
            }),
            Err(e) => return Err(e.into()),
        }
    }

    Ok(CredentialsTemplate {
        shell: Shell::build(&user, &session).await?,
        heading: format!("Import finished: {created} account(s) created"),
        rows,
        back_href: "/admin/people",
        back_label: "Back to people",
    }
    .into_response())
}
