use sqlx::{FromRow, PgPool};

type Res<T> = Result<T, sqlx::Error>;

#[derive(Debug, FromRow)]
pub struct LoginRow {
    pub id: i64,
    pub password_hash: String,
    pub is_active: bool,
    pub locked: bool,
}

#[derive(Debug, FromRow)]
pub struct SessionUserRow {
    pub id: i64,
    pub full_name: String,
    pub role: String,
    pub must_change_password: bool,
    pub is_hod: bool,
    pub can_manage: bool,
}

/// Sign in with the account email or, for students, the admission number.
pub async fn find_for_login(db: &PgPool, identifier: &str) -> Res<Option<LoginRow>> {
    sqlx::query_as::<_, LoginRow>(
        r#"SELECT u.id,
                  u.password_hash,
                  u.is_active,
                  (u.locked_until IS NOT NULL AND u.locked_until > now()) AS locked
           FROM users u
           LEFT JOIN students s ON s.user_id = u.id
           WHERE lower(u.email) = lower($1) OR lower(s.admission_no) = lower($1)
           ORDER BY (lower(u.email) = lower($1)) DESC
           LIMIT 1"#,
    )
    .bind(identifier)
    .fetch_optional(db)
    .await
}

pub async fn find_active(db: &PgPool, id: i64) -> Res<Option<SessionUserRow>> {
    // The faculty flags ride along here so the sidebar can show HOD links
    // without a second query on every page.
    sqlx::query_as::<_, SessionUserRow>(
        r#"SELECT u.id, u.full_name, u.role, u.must_change_password,
                  COALESCE(f.is_hod, false) AS is_hod,
                  COALESCE(f.can_manage, false) AS can_manage
           FROM users u
           LEFT JOIN faculty f ON f.user_id = u.id
           WHERE u.id = $1 AND u.is_active"#,
    )
    .bind(id)
    .fetch_optional(db)
    .await
}

/// Five wrong passwords lock the account for 15 minutes.
pub async fn record_failed_login(db: &PgPool, id: i64) -> Res<()> {
    sqlx::query(
        r#"UPDATE users
           SET locked_until = CASE WHEN failed_logins + 1 >= 5
                                   THEN now() + interval '15 minutes' ELSE locked_until END,
               failed_logins = CASE WHEN failed_logins + 1 >= 5 THEN 0 ELSE failed_logins + 1 END
           WHERE id = $1"#,
    )
    .bind(id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn record_login(db: &PgPool, id: i64) -> Res<()> {
    sqlx::query("UPDATE users SET failed_logins = 0, locked_until = NULL, last_login_at = now() WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn password_hash(db: &PgPool, id: i64) -> Res<String> {
    sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(id)
        .fetch_one(db)
        .await
}

pub async fn set_password(db: &PgPool, id: i64, new_hash: &str) -> Res<()> {
    sqlx::query(
        "UPDATE users SET password_hash = $2, must_change_password = false, password_changed_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(new_hash)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn create(
    db: &PgPool,
    email: &str,
    full_name: &str,
    role: &str,
    password_hash: &str,
    must_change_password: bool,
) -> Res<i64> {
    sqlx::query_scalar(
        r#"INSERT INTO users (email, full_name, role, password_hash, must_change_password)
           VALUES ($1, $2, $3, $4, $5) RETURNING id"#,
    )
    .bind(email)
    .bind(full_name)
    .bind(role)
    .bind(password_hash)
    .bind(must_change_password)
    .fetch_one(db)
    .await
}

pub async fn audit(
    db: &PgPool,
    actor: Option<i64>,
    action: &str,
    entity: &str,
    entity_id: Option<i64>,
) -> Res<()> {
    sqlx::query("INSERT INTO audit_log (actor_user_id, action, entity, entity_id) VALUES ($1, $2, $3, $4)")
        .bind(actor)
        .bind(action)
        .bind(entity)
        .bind(entity_id)
        .execute(db)
        .await?;
    Ok(())
}

#[derive(Debug, FromRow)]
pub struct Counts {
    pub students: i64,
    pub teachers: i64,
    pub programmes: i64,
    pub news: i64,
}

pub async fn overview_counts(db: &PgPool) -> Res<Counts> {
    sqlx::query_as::<_, Counts>(
        r#"SELECT (SELECT count(*) FROM students WHERE is_active) AS students,
                  (SELECT count(*) FROM faculty WHERE status = 'published') AS teachers,
                  (SELECT count(*) FROM programmes WHERE status = 'published') AS programmes,
                  (SELECT count(*) FROM news WHERE status = 'published') AS news"#,
    )
    .fetch_one(db)
    .await
}

/// Reset: new temporary password, forced change at next sign-in, lock cleared.
pub async fn set_temp_password(db: &PgPool, id: i64, new_hash: &str) -> Res<()> {
    sqlx::query(
        "UPDATE users SET password_hash = $2, must_change_password = true, failed_logins = 0, locked_until = NULL WHERE id = $1",
    )
    .bind(id)
    .bind(new_hash)
    .execute(db)
    .await?;
    Ok(())
}
