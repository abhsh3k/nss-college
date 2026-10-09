//! Admin CRUD for the content the public site publishes: notices, news and events.
//!
//! Plain `query_as` (not the compile-time macros) so the project still builds
//! without a live database, matching `services::content`.

use sqlx::{FromRow, PgPool};

type Res<T> = Result<T, sqlx::Error>;

/// The statuses the tables' CHECK constraint allows.
pub const STATUSES: [&str; 3] = ["draft", "published", "archived"];
/// The audiences the tables' CHECK constraint allows.
pub const AUDIENCES: [&str; 3] = ["public", "students", "faculty"];

/// Normalise a status coming from a form, falling back to `draft`.
pub fn clean_status(value: &str) -> &'static str {
    match value.trim() {
        "published" => "published",
        "archived" => "archived",
        _ => "draft",
    }
}

/// Normalise an audience coming from a form, falling back to `public`.
pub fn clean_audience(value: &str) -> &'static str {
    match value.trim() {
        "students" => "students",
        "faculty" => "faculty",
        _ => "public",
    }
}

// ---------- Notices ----------

#[derive(Debug, FromRow)]
pub struct NoticeRow {
    pub id: i64,
    pub title: String,
    pub category: String,
    pub audience: String,
    pub is_pinned: bool,
    pub status: String,
    pub published_label: String,
    pub has_attachment: bool,
}

#[derive(Debug)]
pub struct NoticeInput {
    pub title: String,
    pub category: String,
    pub body: String,
    pub attachment_path: String,
    pub audience: String,
    pub is_pinned: bool,
    pub status: String,
}

/// One notice as the edit form needs it: raw values, no formatting.
#[derive(Debug, FromRow)]
pub struct NoticeEdit {
    pub id: i64,
    pub title: String,
    pub category: String,
    pub body: String,
    pub attachment_path: String,
    pub audience: String,
    pub is_pinned: bool,
    pub status: String,
}

/// `status` empty means every status, so the list can show drafts too.
pub async fn notices(db: &PgPool, status: &str) -> Res<Vec<NoticeRow>> {
    sqlx::query_as::<_, NoticeRow>(
        r#"SELECT id, title, category, audience, is_pinned, status,
                  COALESCE(to_char(published_at, 'DD Mon YYYY HH24:MI'), '—') AS published_label,
                  (attachment_path IS NOT NULL) AS has_attachment
           FROM notices
           WHERE ($1 = '' OR status = $1)
           ORDER BY is_pinned DESC, created_at DESC"#,
    )
    .bind(status)
    .fetch_all(db)
    .await
}

pub async fn notice_for_edit(db: &PgPool, id: i64) -> Res<Option<NoticeEdit>> {
    sqlx::query_as::<_, NoticeEdit>(
        r#"SELECT id, title, category, body,
                  COALESCE(attachment_path, '') AS attachment_path,
                  audience, is_pinned, status
           FROM notices WHERE id = $1"#,
    )
    .bind(id)
    .fetch_optional(db)
    .await
}

pub async fn create_notice(db: &PgPool, n: &NoticeInput, author: Option<i64>) -> Res<i64> {
    sqlx::query_scalar(
        r#"INSERT INTO notices
             (title, category, body, attachment_path, audience, is_pinned, status, published_at, created_by)
           VALUES ($1, $2, $3, NULLIF($4, ''), $5, $6, $7,
                   CASE WHEN $7 = 'published' THEN now() ELSE NULL END, $8)
           RETURNING id"#,
    )
    .bind(&n.title)
    .bind(&n.category)
    .bind(&n.body)
    .bind(&n.attachment_path)
    .bind(clean_audience(&n.audience))
    .bind(n.is_pinned)
    .bind(clean_status(&n.status))
    .bind(author)
    .fetch_one(db)
    .await
}

pub async fn update_notice(db: &PgPool, id: i64, n: &NoticeInput) -> Res<()> {
    sqlx::query(
        r#"UPDATE notices
           SET title = $2, category = $3, body = $4,
               attachment_path = NULLIF($5, ''), audience = $6, is_pinned = $7, status = $8,
               published_at = CASE WHEN $8 = 'published' THEN COALESCE(published_at, now()) ELSE published_at END
           WHERE id = $1"#,
    )
    .bind(id)
    .bind(&n.title)
    .bind(&n.category)
    .bind(&n.body)
    .bind(&n.attachment_path)
    .bind(clean_audience(&n.audience))
    .bind(n.is_pinned)
    .bind(clean_status(&n.status))
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete_notice(db: &PgPool, id: i64) -> Res<()> {
    sqlx::query("DELETE FROM notices WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

// ---------- News ----------

#[derive(Debug, FromRow)]
pub struct NewsRow {
    pub id: i64,
    pub title: String,
    pub status: String,
    pub published_label: String,
    pub has_image: bool,
}

#[derive(Debug)]
pub struct NewsInput {
    pub title: String,
    pub body: String,
    pub image_path: String,
    pub status: String,
}

#[derive(Debug, FromRow)]
pub struct NewsEdit {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub image_path: String,
    pub status: String,
}

pub async fn news(db: &PgPool, status: &str) -> Res<Vec<NewsRow>> {
    sqlx::query_as::<_, NewsRow>(
        r#"SELECT id, title, status,
                  COALESCE(to_char(published_at, 'DD Mon YYYY HH24:MI'), '—') AS published_label,
                  (image_path IS NOT NULL) AS has_image
           FROM news
           WHERE ($1 = '' OR status = $1)
           ORDER BY created_at DESC"#,
    )
    .bind(status)
    .fetch_all(db)
    .await
}

pub async fn news_for_edit(db: &PgPool, id: i64) -> Res<Option<NewsEdit>> {
    sqlx::query_as::<_, NewsEdit>(
        r#"SELECT id, title, body, COALESCE(image_path, '') AS image_path, status
           FROM news WHERE id = $1"#,
    )
    .bind(id)
    .fetch_optional(db)
    .await
}

pub async fn create_news(db: &PgPool, n: &NewsInput, author: Option<i64>) -> Res<i64> {
    sqlx::query_scalar(
        r#"INSERT INTO news (title, body, image_path, status, published_at, created_by)
           VALUES ($1, $2, NULLIF($3, ''), $4,
                   CASE WHEN $4 = 'published' THEN now() ELSE NULL END, $5)
           RETURNING id"#,
    )
    .bind(&n.title)
    .bind(&n.body)
    .bind(&n.image_path)
    .bind(clean_status(&n.status))
    .bind(author)
    .fetch_one(db)
    .await
}

pub async fn update_news(db: &PgPool, id: i64, n: &NewsInput) -> Res<()> {
    sqlx::query(
        r#"UPDATE news
           SET title = $2, body = $3, image_path = NULLIF($4, ''), status = $5,
               published_at = CASE WHEN $5 = 'published' THEN COALESCE(published_at, now()) ELSE published_at END
           WHERE id = $1"#,
    )
    .bind(id)
    .bind(&n.title)
    .bind(&n.body)
    .bind(&n.image_path)
    .bind(clean_status(&n.status))
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete_news(db: &PgPool, id: i64) -> Res<()> {
    sqlx::query("DELETE FROM news WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

// ---------- Events ----------

#[derive(Debug, FromRow)]
pub struct EventRow {
    pub id: i64,
    pub title: String,
    pub location: String,
    pub starts_label: String,
    pub audience: String,
    pub status: String,
    pub is_past: bool,
    pub has_image: bool,
}

#[derive(Debug)]
pub struct EventInput {
    pub title: String,
    pub description: String,
    pub location: String,
    /// Wall-clock time as "YYYY-MM-DD HH:MM", cast by Postgres in the server timezone.
    pub starts_at: String,
    pub ends_at: String,
    pub image_path: String,
    pub audience: String,
    pub status: String,
}

#[derive(Debug, FromRow)]
pub struct EventEdit {
    pub id: i64,
    pub title: String,
    pub description: String,
    pub location: String,
    pub starts_value: String,
    pub ends_value: String,
    pub image_path: String,
    pub audience: String,
    pub status: String,
}

pub async fn events(db: &PgPool, status: &str) -> Res<Vec<EventRow>> {
    sqlx::query_as::<_, EventRow>(
        r#"SELECT id, title, COALESCE(location, '') AS location,
                  to_char(starts_at, 'DD Mon YYYY HH24:MI') AS starts_label,
                  audience, status, (starts_at < now()) AS is_past,
                  (image_path IS NOT NULL) AS has_image
           FROM events
           WHERE ($1 = '' OR status = $1)
           ORDER BY starts_at DESC"#,
    )
    .bind(status)
    .fetch_all(db)
    .await
}

pub async fn event_for_edit(db: &PgPool, id: i64) -> Res<Option<EventEdit>> {
    sqlx::query_as::<_, EventEdit>(
        r#"SELECT id, title, description, COALESCE(location, '') AS location,
                  to_char(starts_at, 'YYYY-MM-DD"T"HH24:MI') AS starts_value,
                  COALESCE(to_char(ends_at, 'YYYY-MM-DD"T"HH24:MI'), '') AS ends_value,
                  COALESCE(image_path, '') AS image_path,
                  audience, status
           FROM events WHERE id = $1"#,
    )
    .bind(id)
    .fetch_optional(db)
    .await
}

pub async fn create_event(db: &PgPool, e: &EventInput, author: Option<i64>) -> Res<i64> {
    sqlx::query_scalar(
        r#"INSERT INTO events
             (title, description, location, starts_at, ends_at, image_path, audience, status, created_by)
           VALUES ($1, $2, NULLIF($3, ''), $4::timestamptz, NULLIF($5, '')::timestamptz,
                   NULLIF($6, ''), $7, $8, $9)
           RETURNING id"#,
    )
    .bind(&e.title)
    .bind(&e.description)
    .bind(&e.location)
    .bind(&e.starts_at)
    .bind(&e.ends_at)
    .bind(&e.image_path)
    .bind(clean_audience(&e.audience))
    .bind(clean_status(&e.status))
    .bind(author)
    .fetch_one(db)
    .await
}

pub async fn update_event(db: &PgPool, id: i64, e: &EventInput) -> Res<()> {
    sqlx::query(
        r#"UPDATE events
           SET title = $2, description = $3, location = NULLIF($4, ''),
               starts_at = $5::timestamptz, ends_at = NULLIF($6, '')::timestamptz,
               image_path = NULLIF($7, ''), audience = $8, status = $9
           WHERE id = $1"#,
    )
    .bind(id)
    .bind(&e.title)
    .bind(&e.description)
    .bind(&e.location)
    .bind(&e.starts_at)
    .bind(&e.ends_at)
    .bind(&e.image_path)
    .bind(clean_audience(&e.audience))
    .bind(clean_status(&e.status))
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete_event(db: &PgPool, id: i64) -> Res<()> {
    sqlx::query("DELETE FROM events WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

// ---------- Archive / publish without touching the rest of the row ----------

pub async fn set_notice_status(db: &PgPool, id: i64, status: &str) -> Res<()> {
    sqlx::query(
        r#"UPDATE notices
           SET status = $2,
               published_at = CASE WHEN $2 = 'published' THEN COALESCE(published_at, now()) ELSE published_at END
           WHERE id = $1"#,
    )
    .bind(id)
    .bind(clean_status(status))
    .execute(db)
    .await?;
    Ok(())
}

pub async fn set_news_status(db: &PgPool, id: i64, status: &str) -> Res<()> {
    sqlx::query(
        r#"UPDATE news
           SET status = $2,
               published_at = CASE WHEN $2 = 'published' THEN COALESCE(published_at, now()) ELSE published_at END
           WHERE id = $1"#,
    )
    .bind(id)
    .bind(clean_status(status))
    .execute(db)
    .await?;
    Ok(())
}

pub async fn set_event_status(db: &PgPool, id: i64, status: &str) -> Res<()> {
    sqlx::query("UPDATE events SET status = $2 WHERE id = $1")
        .bind(id)
        .bind(clean_status(status))
        .execute(db)
        .await?;
    Ok(())
}

/// Rows that reference an upload, so the file can be removed from disk when the
/// content row is deleted for good.
pub async fn upload_path_for_notice(db: &PgPool, id: i64) -> Res<Option<String>> {
    sqlx::query_scalar("SELECT attachment_path FROM notices WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await
}

pub async fn upload_path_for_news(db: &PgPool, id: i64) -> Res<Option<String>> {
    sqlx::query_scalar("SELECT image_path FROM news WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await
}

pub async fn upload_path_for_event(db: &PgPool, id: i64) -> Res<Option<String>> {
    sqlx::query_scalar("SELECT image_path FROM events WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await
}

/// Drop the uploads row and the file itself for a path we stored earlier.
/// Errors are logged rather than returned, because callers use this during
/// deletes/updates where a cleanup failure should not roll back the main action.
pub async fn discard_upload(db: &PgPool, upload_dir: &str, path: &str) {
    if path.is_empty() {
        return;
    }
    // Only ever touch a path inside our own uploads directory.
    let relative = match path.strip_prefix("/uploads/") {
        Some(r) if !r.contains("..") => r,
        _ => return,
    };
    match sqlx::query("DELETE FROM uploads WHERE path = $1 RETURNING id")
        .bind(path)
        .fetch_optional(db)
        .await
    {
        Ok(Some(_)) => {
            let full = std::path::Path::new(upload_dir).join(relative);
            if let Err(e) = std::fs::remove_file(full) {
                tracing::warn!(error = %e, path = %relative, "could not delete the uploaded file");
            }
        }
        Ok(None) => {
            tracing::warn!(path = %relative, "upload row not found for discard");
        }
        Err(e) => {
            tracing::warn!(error = ?e, path = %relative, "could not delete the upload row");
        }
    }
}

