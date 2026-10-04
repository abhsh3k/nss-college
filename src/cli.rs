//! Command-line helpers, run instead of starting the server:
//!
//!   cargo run -- create-user <admin|staff|faculty|student> <email> "<Full name>"
//!   cargo run -- seed-demo-users
//!
//! The password is typed at a hidden prompt, never passed on the command line.
//! `seed-demo-users` needs no prompt: it generates a password per account and
//! prints the whole set, which is what makes it scriptable.

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
        Some("seed-demo-users") => {
            if let Err(msg) = seed_demo_users(db).await {
                eprintln!("error: {msg}");
                std::process::exit(1);
            }
            true
        }
        Some(other) => {
            eprintln!(
                "unknown command: {other}\n\
                 usage: cargo run -- create-user <admin|staff|faculty|student> <email> \"<Full name>\"\n\
                 \x20      cargo run -- seed-demo-users"
            );
            std::process::exit(2);
        }
        None => false,
    }
}

async fn create_user(args: &[String], db: &PgPool) -> Result<(), String> {
    let [role_arg, email, full_name] = args else {
        return Err("usage: cargo run -- create-user <admin|staff|faculty|student> <email> \"<Full name>\"".into());
    };
    let role = Role::parse(role_arg).ok_or("role must be admin, staff, faculty, student or alumni")?;
    if !email.contains('@') {
        return Err("email must contain @ (students without email can use admission-number@college.local)".into());
    }

    let min_len = if role == Role::Admin { 12 } else { password::MIN_LENGTH };
    let pw = rpassword::prompt_password(format!("Password (at least {min_len} characters): "))
        .map_err(|e| e.to_string())?;
    let again = rpassword::prompt_password("Repeat password: ").map_err(|e| e.to_string())?;
    if pw != again {
        return Err("passwords did not match".into());
    }
    if pw.chars().count() < min_len {
        return Err(format!("password must be at least {min_len} characters"));
    }

    let hash = password::hash_blocking(pw).await.map_err(|_| "could not hash password".to_string())?;
    // Everyone except the first administrator must replace the password they were given.
    let must_change = role != Role::Admin;
    let id = users::create(db, email.trim(), full_name.trim(), role_arg, &hash, must_change)
        .await
        .map_err(|e| match e.as_database_error().and_then(|d| d.code()).as_deref() {
            Some("23505") => "an account with that email already exists".to_string(),
            _ => e.to_string(),
        })?;
    let _ = users::audit(db, None, "user_created_cli", "user", Some(id)).await;

    println!("Created {} account for {} (id {id}).", role.label(), email.trim());
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

/// Create a ready-to-use set of demo accounts and print their credentials.
///
/// Skips anything that already exists, so it is safe to run twice. Passwords
/// are generated rather than prompted for, which is what lets this run from a
/// script or a container without a TTY.
async fn seed_demo_users(db: &PgPool) -> Result<(), String> {
    let mut out: Vec<Seeded> = Vec::new();

    async fn hash_for(plain: &str) -> Result<String, String> {
        password::hash_blocking(plain.to_string())
            .await
            .map_err(|_| "could not hash password".to_string())
    }

    // ---- IT administrator (must_change_password = false, like create-user) ----
    for (email, name) in [("admin@college.local", "Site Administrator")] {
        if users::find_for_login(db, email).await.map_err(|e| e.to_string())?.is_some() {
            continue;
        }
        let plain = password::temporary();
        let hash = hash_for(&plain).await?;
        let id = users::create(db, email, name, "admin", &hash, false)
            .await
            .map_err(|e| e.to_string())?;
        let _ = users::audit(db, None, "user_created_seed", "user", Some(id)).await;
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
        if users::find_for_login(db, email).await.map_err(|e| e.to_string())?.is_some() {
            continue;
        }
        let plain = password::temporary();
        let hash = hash_for(&plain).await?;
        let id = people::create_staff(db, name, email, &hash)
            .await
            .map_err(|e| e.to_string())?;
        out.push(Seeded {
            role: "staff",
            name: name.to_string(),
            login: email.to_string(),
            password: plain,
            admission_no: None,
        });
        let _ = id;
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
        if users::find_for_login(db, email).await.map_err(|e| e.to_string())?.is_some() {
            continue;
        }
        let plain = password::temporary();
        let hash = hash_for(&plain).await?;
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
        let admission_no = random_admission_no();
        let email = format!("student{n}@college.local");
        if users::find_for_login(db, &email).await.map_err(|e| e.to_string())?.is_some() {
            continue;
        }
        let name = format!("Student {n}");
        let plain = password::temporary();
        let hash = hash_for(&plain).await?;
        people::create_student(
            db,
            &people::NewStudent {
                admission_no: &admission_no,
                name: &name,
                email: &email,
                programme_id,
                batch_year: 2024,
                semester: 3,
                phone: "",
                egrants: n % 2 == 0,
            },
            &hash,
        )
        .await
        .map_err(|e| e.to_string())?;
        out.push(Seeded {
            role: "student",
            name,
            // Students sign in with the admission number, not the email.
            login: admission_no.clone(),
            password: plain,
            admission_no: Some(admission_no),
        });
    }

    if out.is_empty() {
        println!("Everything already exists; nothing was created.");
        return Ok(());
    }

    println!("\nDemo accounts created. Sign in at /login with these:\n");
    println!("{:<9} {:<24} {:<26} {}", "ROLE", "NAME", "LOGIN", "PASSWORD");
    println!("{}", "-".repeat(92));
    for s in &out {
        println!("{:<9} {:<24} {:<26} {}", s.role, s.name, s.login, s.password);
    }
    println!("\nStudent admission numbers: {}\n", out.iter().filter_map(|s| s.admission_no.as_ref()).cloned().collect::<Vec<_>>().join(", "));
    Ok(())
}
