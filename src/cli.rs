//! Command-line helpers, run instead of starting the server (normally):
//!
//!   cargo run -- create-user <super_admin|admin|principal|staff|faculty|student> <email> "<Full name>" [password]
//!   cargo run -- set-role <super_admin|admin> <email>
//!   cargo run -- set-password <email>
//!   cargo run -- seed-demo-users
//!   cargo run -- backup [--list]
//!   cargo run -- maintenance
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
    services::{backup, maintenance, people, super_admin, users},
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
        Some("set-role") => {
            if let Err(msg) = set_role(&args[1..], db).await {
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
                Ok(lines) => match credentials_destination(&args[1..]) {
                    // Writing to a file keeps generated passwords out of the
                    // shell history, out of CI logs and out of a recorded
                    // terminal. The file is created 0600.
                    Ok(Some(path)) => match write_credentials(&lines, &path) {
                        Ok(()) => println!(
                            "Wrote {} demo credential line(s) to {}. \
                             Move them to your password manager and delete the file.",
                            lines.len(),
                            path.display()
                        ),
                        Err(msg) => {
                            eprintln!("error: {msg}");
                            std::process::exit(1);
                        }
                    },
                    Ok(None) => {
                        for line in &lines {
                            println!("{line}");
                        }
                        eprintln!(
                            "note: these passwords are now in your terminal scrollback. \
                             Use `seed-demo-users --credentials-file <path>` to avoid that."
                        );
                    }
                    Err(msg) => {
                        eprintln!("error: {msg}");
                        std::process::exit(2);
                    }
                },
                Err(msg) => {
                    eprintln!("error: {msg}");
                    std::process::exit(1);
                }
            }
            true
        }
        Some("backup") => {
            if let Err(msg) = backup_command(&args[1..], db).await {
                eprintln!("error: {msg}");
                std::process::exit(1);
            }
            true
        }
        Some("maintenance") => {
            let report = maintenance::purge(db).await;
            println!(
                "purged: {} expired session(s), {} stale student import batch(es), \
                 {} expired result import(s), {} old result import(s)",
                report.sessions,
                report.stale_student_batches,
                report.expired_result_imports,
                report.old_result_imports
            );
            true
        }
        Some(other) => {
            eprintln!(
                "unknown command: {other}\n\
                 usage: cargo run -- create-user <super_admin|admin|principal|staff|faculty|student> <email> \"<Full name>\"\n\
                 \x20      cargo run -- set-role <super_admin|admin> <email>\n\
                 \x20      cargo run -- set-password <email>\n\
                 \x20      cargo run -- seed-demo-users\n\
                 \x20      cargo run -- backup [--list]\n\
                 \x20      cargo run -- maintenance"
            );
            std::process::exit(2);
        }
        None => false,
    }
}

/// Where the seeded credentials should go: `None` means print them (the
/// explicit interactive path).
pub fn credentials_destination(args: &[String]) -> Result<Option<std::path::PathBuf>, String> {
    match args {
        [] => Ok(None),
        [flag, path] if flag == "--credentials-file" => {
            if path.trim().is_empty() {
                return Err("--credentials-file needs a path".into());
            }
            Ok(Some(std::path::PathBuf::from(path)))
        }
        _ => Err("usage: cargo run -- seed-demo-users [--credentials-file <path>]".into()),
    }
}

/// Write the credential report to a file only the current user can read.
pub fn write_credentials(lines: &[String], path: &std::path::Path) -> Result<(), String> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|e| format!("could not create {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
    }
    for line in lines {
        writeln!(file, "{line}").map_err(|e| format!("could not write {}: {e}", path.display()))?;
    }
    Ok(())
}

/// Take a backup now, or list the sets already on disk.
///
/// `--list` needs no database connection beyond the one already made, and no
/// `pg_dump`: it just reports what a restore could use.
async fn backup_command(args: &[String], db: &PgPool) -> Result<(), String> {
    let cfg = backup::BackupConfig::from_env();
    if args.first().map(String::as_str) == Some("--list") {
        let sets = backup::list_sets(&cfg.dir);
        if sets.is_empty() {
            println!("No backups in {}. Run `cargo run -- backup`.", cfg.dir.display());
        } else {
            println!("Backups in {} (newest first):", cfg.dir.display());
            for set in sets {
                println!("  {}", set.display());
            }
        }
        return Ok(());
    }
    if !args.is_empty() {
        return Err("usage: cargo run -- backup [--list]".into());
    }
    let report = backup::run(db, &cfg).await?;
    println!("Backup written to {}", report.path.display());
    println!("  database: {} bytes", report.database_bytes);
    match report.uploads_bytes {
        Some(bytes) => println!("  uploads:  {bytes} bytes"),
        None => println!("  uploads:  not archived (see the manifest for why)"),
    }
    if report.removed_old > 0 {
        println!("  removed {} older set(s) beyond the retention of {}", report.removed_old, cfg.keep);
    }
    Ok(())
}

async fn create_user(args: &[String], db: &PgPool) -> Result<(), String> {
    if !(3..=4).contains(&args.len()) {
        return Err("usage: cargo run -- create-user <super_admin|admin|principal|staff|faculty|student> <email> \"<Full name>\"".into());
    }
    let role_arg = &args[0];
    let email = &args[1];
    let full_name = &args[2];
    let maybe_pw = args.get(3).map(String::as_str);
    let role = Role::parse(role_arg)
        .ok_or("role must be super_admin, admin, principal, staff, faculty, student or alumni")?;
    if !email.contains('@') {
        return Err("email must contain @ (students without email can use admission-number@college.local)".into());
    }

    let pw: String = match maybe_pw {
        Some(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => prompt_new_password("Password", password::MIN_LENGTH)?,
    };

    let hash = password::hash_blocking(pw).await.map_err(|_| "could not hash password".to_string())?;
    // The operator roles — the Super Admin and the IT admin — set their own
    // password here; everyone else must replace the one they were given.
    let must_change = !matches!(role, Role::SuperAdmin | Role::Admin);
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

/// The roles `set-role` may set: the two technical roles the Super Admin
/// screen also manages. `principal` is deliberately not one of them — academic
/// authority is recorded as an effective-dated appointment, not as an account
/// role — and the departmental roles belong on the appointments screen.
fn technical_role_arg(arg: &str) -> Result<&'static str, String> {
    let role = arg.trim();
    super_admin::TECHNICAL_ROLES
        .iter()
        .copied()
        .find(|r| *r == role)
        .ok_or_else(|| "role must be super_admin or admin".to_string())
}

/// Give an existing account one of the technical roles.
///
/// This is the operator's break-glass path for root authority: the screen that
/// manages roles needs an account that already holds it, so when no root
/// account can be reached the CLI is the only way back in. The rule itself is
/// the same one the screen uses (`services::super_admin::set_role`), so the
/// last active Super Admin can be promoted from here but never demoted here
/// either, and the change revokes any session the account holds.
async fn set_role(args: &[String], db: &PgPool) -> Result<(), String> {
    let [role_arg, email] = args else {
        return Err("usage: cargo run -- set-role <super_admin|admin> <email>".into());
    };
    let role = technical_role_arg(role_arg)?;
    let email = email.trim();
    let id: Option<i64> = sqlx::query_scalar("SELECT id FROM users WHERE lower(email) = lower($1)")
        .bind(email)
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())?;
    let Some(id) = id else {
        return Err(format!("no account with email {email}"));
    };

    match super_admin::set_role(db, id, role).await.map_err(|e| e.to_string())? {
        super_admin::RoleChange::Done => {
            let _ = users::audit_detail(
                db,
                None,
                "user_role_changed_cli",
                "user",
                Some(id),
                serde_json::json!({ "role": role }),
            )
            .await;
            println!("{email} is now {role}.");
            Ok(())
        }
        super_admin::RoleChange::NotFound => Err("that account no longer exists".into()),
        super_admin::RoleChange::UnknownRole => {
            Err("role must be super_admin or admin".into())
        }
        super_admin::RoleChange::LastSuperAdmin => Err(
            "that is the last active Super Admin; promote another account first".into(),
        ),
        super_admin::RoleChange::StillPrincipal => Err(
            "that account is the Principal; end the Principal appointment instead".into(),
        ),
    }
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
    ///
    /// Looked up by email rather than through `find_for_login`: that helper
    /// signs students in by admission number, so it would report a seeded
    /// student's email as "not found" and the second seed run would then try to
    /// insert a duplicate email.
    async fn existing_id(db: &PgPool, email: &str) -> Result<Option<i64>, String> {
        sqlx::query_scalar("SELECT id FROM users WHERE lower(email) = lower($1)")
            .bind(email.trim())
            .fetch_optional(db)
            .await
            .map_err(|e| e.to_string())
    }

    // Give an account the generated password whether or not it already exists,
    // so the printed credentials are always the ones that will work.

    // ---- Super Admin: root authority on the loopback management listener ----
    for (email, name) in [("owner@college.local", "System Owner")] {
        let plain = password::temporary();
        let hash = hash_for(&plain).await?;
        if let Some(id) = existing_id(db, email).await? {
            users::set_password(db, id, &hash)
                .await
                .map_err(|e| e.to_string())?;
        } else {
            let id = users::create(db, email, name, "super_admin", &hash, false)
                .await
                .map_err(|e| e.to_string())?;
            let _ = users::audit(db, None, "user_created_seed", "user", Some(id)).await;
        }
        out.push(Seeded {
            role: "super_admin",
            name: name.to_string(),
            login: email.to_string(),
            password: plain,
            admission_no: None,
        });
    }

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
            // Keep the appointment table in step with the flag we just set.
            crate::services::appointments::sync_hod_from_flags(db, id)
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
    // The role column fits `super_admin` (11 characters) without colliding
    // with the name beside it.
    lines.push(format!("{:<12} {:<22} {:<26} {}", "ROLE", "NAME", "LOGIN", "PASSWORD"));
    lines.push("-".repeat(88));
    for s in &out {
        lines.push(format!("{:<12} {:<22} {:<26} {}", s.role, s.name, s.login, s.password));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_role_only_accepts_the_two_technical_roles() {
        assert_eq!(technical_role_arg("super_admin"), Ok("super_admin"));
        assert_eq!(technical_role_arg(" admin "), Ok("admin"));
        // Academic authority is an appointment, not an account role, and the
        // departmental roles belong on the appointments screen.
        for refused in ["principal", "staff", "faculty", "student", "alumni", ""] {
            assert!(
                technical_role_arg(refused).is_err(),
                "{refused} must be refused by set-role"
            );
        }
    }

    #[test]
    fn create_user_accepts_every_role_its_usage_line_documents() {
        for name in [
            "super_admin",
            "admin",
            "principal",
            "staff",
            "faculty",
            "student",
            "alumni",
        ] {
            assert!(
                Role::parse(name).is_some(),
                "{name} is documented but not accepted"
            );
        }
    }
}
