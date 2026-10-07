//! Command-line helpers, run instead of starting the server (normally):
//!
//!   cargo run -- create-user <admin|staff|faculty|student> <email> "<Full name>" [password]
//!   cargo run -- set-password <email>
//!   cargo run -- seed-demo-users
//!
//! The password is typed at a hidden prompt, never passed on the command line,
//! *unless* a fourth argument is given — that path is only intended for
//! automation / first-admin bootstrapping. `seed-demo-users` needs no prompt:
//! it generates a password per account and prints the whole set, which is what
//! makes it scriptable. When a password is supplied as an argument it is still
//! validated against the same minimum length rule as the interactive prompt.

use sqlx::PgPool;

use crate::{
    auth::{password, Role},
    services::{people, users},
};

/// Returns true when a command was handled and the server should not start.
pub async fn run(args: &[String], db: &PgPool) -> bool {
    match args.first().map(String::as_str) {
        Some("create-user") => {
            if let Err(msg) = create_user(&args[1..], db).await {
                eprintln!("error: {msg}");
                std::process::exit(1);
            }
            true
        }
        Some("set-password") => {
            if let Err(msg) = set_password(&args[1..], db).await {
                eprintln!("error: {msg}");
                std::process::exit(1);
            }
            true
        }
        Some("seed-demo-users") => {
            match seed_demo_users(db).await {
                Ok(lines) => {
                    for line in &lines {
                        println!("{line}");
                    }
                }
                Err(msg) => {
                    eprintln!("error: {msg}");
                    std::process::exit(1);
                }
            }
            true
        }
        Some(other) => {
            eprintln!(
                "unknown command: {other}\n\
                 usage: cargo run -- create-user <admin|staff|faculty|student> <email> \"<Full name>\"\n\
                 \x20      cargo run -- set-password <email>\n\
                 \x20      cargo run -- seed-demo-users"
            );
            std::process::exit(2);
        }
        None => false,
    }
}

async fn create_user(args: &[String], db: &PgPool) -> Result<(), String> {
    let [role_arg, email, full_name, maybe_pw] = args
    else {
        return Err("usage: cargo run -- create-user <admin|staff|faculty|student> <email> \"<Full name>\"".into());
    };
    let role = Role::parse(role_arg).ok_or("role must be admin, staff, faculty, student or alumni")?;
    if !email.contains('@') {
        return Err("email must contain @ (students without email can use admission-number@college.local)".into());
    }

    let pw: String = args
        .get(3)
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| prompt_new_password("Password", password::MIN_LENGTH).unwrap());

    let hash = password::hash_blocking(pw).await.map_err(|_| "could not hash password".to_string())?;
    // Everyone except the first administrator must replace the password they were given.
    let must_change = role != Role::Admin;
    let id = users::create(db, email.trim(), full_name.trim(), role_arg, &hash, must_change)
        .await
        .map_err(|e| match e.as_database_error().and_then(|d| d.code()).as_deref() {
            Some("23505") => "an account with that email already exists".to_string(),
            _ => e.to_string(),
        })?;
    let _ = users::audit(db, None, "user_created_cli", "user", Some(id)).await;        let _ = users::audit(db, None, "user_created_cli", "user", Some(id)).await; // defensive re-audit

        println!("Created {} account for {} (id {id}).", role.label(), email.trim());
        Ok(())
}

/// Ask for a new password twice, typed at a hidden prompt, and check it
/// against the minimum length.
fn prompt_new_password(label: &str, min_len: usize) -> Result<String, String> {
    let pw = rpassword::prompt_password(format!("{label} (at least {min_len} characters): "))
        .map_err(|e| e.to_string())?;
    let again = rpassword::prompt_password("Repeat password: ").map_err(|e| e.to_string())?;
    if pw != again {
        return Err("passwords did not match".into());
    }
    if pw.chars().count() < min_len {
        return Err(format!("password must be at least {min_len} characters"));
    }
    Ok(pw)
}

/// Replace the password on an existing account.
///
/// The account keeps its role and flags; only the hash changes, so this is the
/// way to hand a known password to an operator without an email reset flow.
async fn set_password(args: &[String], db: &PgPool) -> Result<(), String> {
    let [email] = args else {
        return Err("usage: cargo run -- set-password <email>".into());
    };
    let id: Option<i64> = sqlx::query_scalar("SELECT id FROM users WHERE lower(email) = lower($1)")
        .bind(email.trim())
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())?;
    let id = id.ok_or_else(|| "no account with that email".to_string())?;

    let pw = prompt_new_password("New password", password::MIN_LENGTH)?;
    let hash = password::hash_blocking(pw).await.map_err(|_| "could not hash password".to_string())?;
    users::set_password(db, id, &hash)
        .await
        .map_err(|e| e.to_string())?;
    // Clear any lockout from earlier failed attempts, so the new password works at once.
    sqlx::query("UPDATE users SET failed_logins = 0, locked_until = NULL WHERE id = $1")
        .bind(id)
        .execute(db)
        .await
        .map_err(|e| e.to_string())?;
    let _ = users::audit(db, None, "password_set_cli", "user", Some(id)).await;

    println!("Password updated for {email}.");
    Ok(())
}

/// One seeded account, ready to be printed.
struct Seeded {
    role: &'static str,
    name: String,
    login: String,
    password: String,
    admission_no: Option<String>,
}

/// A random admission number: the year of admission plus six digits.
///
/// Random rather than sequential so the mock students look like real ones and
/// so two seeds never collide on the unique admission_no index.
fn random_admission_no() -> String {
    use argon2::password_hash::rand_core::{OsRng, RngCore};
    let mut bytes = [0u8; 3];
    OsRng.fill_bytes(&mut bytes);
    let n = u32::from_be_bytes([0, bytes[0], bytes[1], bytes[2]]) % 1_000_000;
    format!("2026{n:06}")
}

/// Create a ready-to-use set of demo accounts and return a report of their
/// credentials.
///
/// Skips anything that already exists, so it is safe to run twice. Passwords
/// are generated rather than prompted for, which is what lets this run from a
/// script or a container without a TTY.
///
/// The report is returned rather than printed so the startup path can log it
/// where the deploy operator can see it.
pub async fn seed_demo_users(db: &PgPool) -> Result<Vec<String>, String> {
    let mut out: Vec<Seeded> = Vec::new();

    async fn hash_for(plain: &str) -> Result<String, String> {
        password::hash_blocking(plain.to_string())
            .await
            .map_err(|_| "could not hash password".to_string())
    }

    /// The id of an account with this email, if it already exists.
    async fn existing_id(db: &PgPool, email: &str) -> Result<Option<i64>, String> {
        Ok(users::find_for_login(db, email)
            .await
            .map_err(|e| e.to_string())?
            .map(|u| u.id))
    }

    /// Give an account the generated password whether or not it already exists,
    /// so the printed credentials are always the ones that will work.

    // ---- IT administrator (must_change_password = false, like create-user) ----
    for (email, name) in [("admin@college.local", "Site Administrator")] {
        let plain = password::temporary();
        let hash = hash_for(&plain).await?;
        if let Some(id) = existing_id(db, email).await? {
            users::set_password(db, id, &hash)
                .await
                .map_err(|e| e.to_string())?;
        } else {
            let id = users::create(db, email, name, "admin", &hash, false)
                .await
                .map_err(|e| e.to_string())?;
            let _ = users::audit(db, None, "user_created_seed", "user", Some(id)).await;
        }
        out.push(Seeded {
            role: "admin",
            name: name.to_string(),
            login: email.to_string(),
            password: plain,
            admission_no: None,
        });
    }

    // ---- Office staff ----
    for (email, name) in [("office@college.local", "Office Staff")] {
        let plain = password::temporary();
        let hash = hash_for(&plain).await?;
        if let Some(id) = existing_id(db, email).await? {
            users::set_password(db, id, &hash)
                .await
                .map_err(|e| e.to_string())?;
        } else {
            people::create_staff(db, name, email, &hash)
                .await
                .map_err(|e| e.to_string())?;
        }
        out.push(Seeded {
            role: "staff",
            name: name.to_string(),
            login: email.to_string(),
            password: plain,
            admission_no: None,
        });
    }

    // ---- Teachers: the first is the head of their department ----
    let department_id: Option<i64> =
        sqlx::query_scalar("SELECT id FROM departments ORDER BY sort_order, name LIMIT 1")
            .fetch_one(db)
            .await
            .ok();

    for (index, (email, name)) in [
        ("hod@college.local", "Hod Teacher"),
        ("teacher1@college.local", "Second Teacher"),
        ("teacher2@college.local", "Third Teacher"),
    ]
    .into_iter()
    .enumerate()
    {
        let plain = password::temporary();
        let hash = hash_for(&plain).await?;
        if let Some(id) = existing_id(db, email).await? {
            users::set_password(db, id, &hash)
                .await
                .map_err(|e| e.to_string())?;
            // Keep the head-of-department flag in step with the seeder.
            sqlx::query("UPDATE faculty SET is_hod = $2, department_id = NULLIF($3, 0) WHERE user_id = $1")
                .bind(id)
                .bind(index == 0)
                .bind(department_id.unwrap_or(0))
                .execute(db)
                .await
                .map_err(|e| e.to_string())?;
        } else {
            people::create_teacher(
                db,
                &people::NewTeacher {
                    name,
                    email,
                    department_id: department_id.unwrap_or(0),
                    designation: if index == 0 { "Head of Department" } else { "Lecturer" },
                    qualification: "M.Sc.",
                    is_hod: index == 0,
                    can_manage: false,
                },
                &hash,
            )
            .await
            .map_err(|e| e.to_string())?;
        }
        out.push(Seeded {
            role: "faculty",
            name: name.to_string(),
            login: email.to_string(),
            password: plain,
            admission_no: None,
        });
    }

    // ---- Students, each with a random admission number ----
    let programme_id: i64 = sqlx::query_scalar(
        "SELECT id FROM programmes WHERE status = 'published' ORDER BY sort_order, name LIMIT 1",
    )
    .fetch_one(db)
    .await
    .map_err(|_| "no published programme to enrol students in".to_string())?;

    for n in 1..=5 {
        let email = format!("student{n}@college.local");
        let name = format!("Student {n}");
        let plain = password::temporary();
        let hash = hash_for(&plain).await?;

        let admission_no = match existing_id(db, &email).await? {
            Some(id) => {
                users::set_password(db, id, &hash)
                    .await
                    .map_err(|e| e.to_string())?;
                // Students sign in with the admission number already on file,
                // so report that rather than a freshly generated one.
                sqlx::query_scalar("SELECT admission_no FROM students WHERE user_id = $1")
                    .bind(id)
                    .fetch_one(db)
                    .await
                    .map_err(|e| e.to_string())?
            }
            None => {
                let adm = random_admission_no();
                people::create_student(
                    db,
                    &people::NewStudent {
                        admission_no: &adm,
                        prn: "",
                        name: &name,
                        email: &email,
                        programme_id,
                        batch_year: 2024,
                        semester: 3,
                        phone: "",
                    },
                    &hash,
                )
                .await
                .map_err(|e| e.to_string())?;
                adm
            }
        };

        out.push(Seeded {
            role: "student",
            name,
            // Students sign in with the admission number, not the email.
            login: admission_no.clone(),
            password: plain,
            admission_no: Some(admission_no),
        });
    }

    let mut lines = Vec::new();
    lines.push("SEED: demo accounts ready. Sign in at /login with these:".to_string());
    lines.push(format!("{:<9} {:<22} {:<26} {}", "ROLE", "NAME", "LOGIN", "PASSWORD"));
    lines.push("-".repeat(88));
    for s in &out {
        lines.push(format!("{:<9} {:<22} {:<26} {}", s.role, s.name, s.login, s.password));
    }
    lines.push(format!(
        "SEED: student admission numbers: {}",
        out.iter()
            .filter_map(|s| s.admission_no.as_ref())
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    ));
    Ok(lines)
}
