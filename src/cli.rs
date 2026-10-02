//! Command-line helpers, run instead of starting the server:
//!
//!   cargo run -- create-user <admin|staff|faculty|student> <email> "<Full name>"
//!
//! The password is typed at a hidden prompt, never passed on the command line.

use sqlx::PgPool;

use crate::{
    auth::{password, Role},
    services::users,
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
        Some(other) => {
            eprintln!("unknown command: {other}\nusage: cargo run -- create-user <admin|staff|faculty|student> <email> \"<Full name>\"");
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
