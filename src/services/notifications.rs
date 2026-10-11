//! In-app notifications: one actionable feed per user.
//!
//! A notification is a pointer, never an authority: it names the page that owns
//! a decision, and that page re-checks the caller when it is opened. Creating
//! one is best-effort from the caller's point of view — a notification that
//! cannot be written must never fail the operation it describes.

use sqlx::{FromRow, PgPool};

type Res<T> = Result<T, sqlx::Error>;

/// One notification as the feed shows it.
#[derive(Debug, FromRow)]
pub struct Notification {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub href: String,
    pub read: bool,
    pub at_label: String,
}

/// Record a notification for one user. The caller passes a short title, an
/// optional body, and the relative `href` the reader should open.
pub async fn create(
    db: &PgPool,
    user_id: i64,
    kind: &str,
    title: &str,
    body: &str,
    href: &str,
) -> Res<i64> {
    sqlx::query_scalar(
        r#"INSERT INTO notifications (user_id, kind, title, body, href)
           VALUES ($1, $2, $3, $4, $5) RETURNING id"#,
    )
    .bind(user_id)
    .bind(kind)
    .bind(title)
    .bind(body)
    .bind(href)
    .fetch_one(db)
    .await
}

/// Notifications for one user, newest first. Unread items are not separated
/// here; the feed marks them.
pub async fn for_user(db: &PgPool, user_id: i64, limit: i64) -> Res<Vec<Notification>> {
    sqlx::query_as::<_, Notification>(
        r#"SELECT id, kind, title, body, href,
                  (read_at IS NOT NULL) AS read,
                  to_char(created_at, 'Dy DD Mon YYYY HH24:MI') AS at_label
             FROM notifications
            WHERE user_id = $1
            ORDER BY created_at DESC, id DESC
            LIMIT $2"#,
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(db)
    .await
}

/// How many unread notifications the user has.
pub async fn unread_count(db: &PgPool, user_id: i64) -> Res<i64> {
    sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE user_id = $1 AND read_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(db)
    .await
}

/// Mark one notification read. Scoped to its owner, so a forged id is a no-op.
pub async fn mark_read(db: &PgPool, id: i64, user_id: i64) -> Res<bool> {
    let done = sqlx::query(
        "UPDATE notifications SET read_at = now() WHERE id = $1 AND user_id = $2 AND read_at IS NULL",
    )
    .bind(id)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Mark every notification read for one user.
pub async fn mark_all_read(db: &PgPool, user_id: i64) -> Res<u64> {
    let done = sqlx::query(
        "UPDATE notifications SET read_at = now() WHERE user_id = $1 AND read_at IS NULL",
    )
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(done.rows_affected())
}

/// Every active user holding a given account role, for fan-out notifications
/// (for example, a new appointment request reaches every Principal).
pub async fn users_with_role(db: &PgPool, role: &str) -> Res<Vec<i64>> {
    sqlx::query_scalar("SELECT id FROM users WHERE role = $1 AND is_active")
        .bind(role)
        .fetch_all(db)
        .await
}

/// Notify every active user with one of the given roles. Best-effort: the
/// caller decides whether to surface a failure.
pub async fn notify_roles(
    db: &PgPool,
    roles: &[&str],
    kind: &str,
    title: &str,
    body: &str,
    href: &str,
) -> Res<()> {
    for role in roles {
        for user_id in users_with_role(db, role).await? {
            create(db, user_id, kind, title, body, href).await?;
        }
    }
    Ok(())
}
