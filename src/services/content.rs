//! Queries for the public site. Plain `query_as` (not the compile-time macros)
//! so the project builds without a live database.

use sqlx::PgPool;

use crate::models::*;

type Res<T> = Result<T, sqlx::Error>;

pub async fn notices(db: &PgPool, limit: i64) -> Res<Vec<Notice>> {
    sqlx::query_as::<_, Notice>(
        r#"SELECT title,
                  category,
                  COALESCE(attachment_path, '/notices') AS href,
                  (published_at > now() - interval '7 days') AS is_new
           FROM notices
           WHERE status = 'published' AND audience = 'public' AND published_at <= now()
           ORDER BY is_pinned DESC, published_at DESC
           LIMIT $1"#,
    )
    .bind(limit)
    .fetch_all(db)
    .await
}

const NEWS_COLUMNS: &str = r#"id,
       title,
       to_char(published_at, 'DD Mon YYYY') AS date,
       '/news/' || id::text AS href,
       image_path AS image,
       body"#;

pub async fn latest_news(db: &PgPool, limit: i64) -> Res<Vec<NewsItem>> {
    let sql = format!(
        "SELECT {NEWS_COLUMNS} FROM news
         WHERE status = 'published' AND published_at <= now()
         ORDER BY published_at DESC LIMIT $1"
    );
    sqlx::query_as::<_, NewsItem>(&sql).bind(limit).fetch_all(db).await
}

pub async fn news_by_id(db: &PgPool, id: i64) -> Res<Option<NewsItem>> {
    let sql = format!(
        "SELECT {NEWS_COLUMNS} FROM news
         WHERE id = $1 AND status = 'published' AND published_at <= now()"
    );
    sqlx::query_as::<_, NewsItem>(&sql).bind(id).fetch_optional(db).await
}

pub async fn departments(db: &PgPool) -> Res<Vec<Department>> {
    sqlx::query_as::<_, Department>(
        "SELECT slug, name, summary FROM departments ORDER BY sort_order, name",
    )
    .fetch_all(db)
    .await
}

pub async fn department_by_slug(db: &PgPool, slug: &str) -> Res<Option<Department>> {
    sqlx::query_as::<_, Department>("SELECT slug, name, summary FROM departments WHERE slug = $1")
        .bind(slug)
        .fetch_optional(db)
        .await
}

const PROGRAMME_SELECT: &str = r#"SELECT p.slug,
       p.name,
       d.name AS department,
       d.slug AS department_slug,
       p.level,
       p.summary,
       '/academics/' || p.slug AS href
FROM programmes p
JOIN departments d ON d.id = p.department_id"#;

pub async fn programmes(db: &PgPool) -> Res<Vec<Programme>> {
    let sql = format!("{PROGRAMME_SELECT} WHERE p.status = 'published' ORDER BY p.sort_order, p.name");
    sqlx::query_as::<_, Programme>(&sql).fetch_all(db).await
}

pub async fn programmes_in_department(db: &PgPool, department_slug: &str) -> Res<Vec<Programme>> {
    let sql = format!(
        "{PROGRAMME_SELECT} WHERE p.status = 'published' AND d.slug = $1 ORDER BY p.sort_order, p.name"
    );
    sqlx::query_as::<_, Programme>(&sql)
        .bind(department_slug)
        .fetch_all(db)
        .await
}

pub async fn programme_by_slug(db: &PgPool, slug: &str) -> Res<Option<Programme>> {
    let sql = format!("{PROGRAMME_SELECT} WHERE p.status = 'published' AND p.slug = $1");
    sqlx::query_as::<_, Programme>(&sql)
        .bind(slug)
        .fetch_optional(db)
        .await
}

pub async fn rank_holders(db: &PgPool, limit: i64) -> Res<Vec<RankHolder>> {
    sqlx::query_as::<_, RankHolder>(
        r#"SELECT r.name,
                  r.rank_position AS rank,
                  d.name AS department,
                  r.exam_year AS year,
                  r.photo_path AS photo
           FROM rank_holders r
           LEFT JOIN departments d ON d.id = r.department_id
           WHERE r.status = 'published'
           ORDER BY r.exam_year DESC, r.sort_order, r.rank_position
           LIMIT $1"#,
    )
    .bind(limit)
    .fetch_all(db)
    .await
}

pub async fn featured_units(db: &PgPool) -> Res<Vec<Unit>> {
    sqlx::query_as::<_, Unit>(
        r#"SELECT short_name AS name,
                  full_name,
                  summary AS text,
                  values_text AS "values",
                  '/student-life/' || slug AS href,
                  image_path AS image
           FROM clubs
           WHERE status = 'published' AND featured
           ORDER BY sort_order"#,
    )
    .fetch_all(db)
    .await
}

pub async fn facilities(db: &PgPool) -> Res<Vec<Facility>> {
    sqlx::query_as::<_, Facility>(
        "SELECT name, description AS text FROM facilities ORDER BY sort_order, id",
    )
    .fetch_all(db)
    .await
}

pub async fn milestones(db: &PgPool) -> Res<Vec<Milestone>> {
    sqlx::query_as::<_, Milestone>(
        r#"SELECT when_label AS "when", description AS text FROM milestones ORDER BY sort_order, id"#,
    )
    .fetch_all(db)
    .await
}

pub async fn page_by_path(db: &PgPool, path: &str) -> Res<Option<Page>> {
    sqlx::query_as::<_, Page>(
        "SELECT id, title, lede FROM pages WHERE path = $1 AND status = 'published'",
    )
    .bind(path)
    .fetch_optional(db)
    .await
}

pub async fn page_sections(db: &PgPool, page_id: i64) -> Res<Vec<PageSection>> {
    sqlx::query_as::<_, PageSection>(
        "SELECT heading, body FROM page_sections WHERE page_id = $1 ORDER BY sort_order, id",
    )
    .bind(page_id)
    .fetch_all(db)
    .await
}

pub async fn contact_info(db: &PgPool) -> Res<ContactInfo> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT key, value FROM site_settings WHERE key LIKE 'contact\\_%'")
            .fetch_all(db)
            .await?;
    let get = |k: &str| {
        rows.iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let phones = ["contact_phone_1", "contact_phone_2"]
        .iter()
        .map(|k| get(*k))
        .filter(|v| !v.is_empty())
        .map(|display| {
            let tel = display
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '+')
                .collect();
            Phone { display, tel }
        })
        .collect();
    Ok(ContactInfo {
        address: get("contact_address"),
        email: get("contact_email"),
        phones,
    })
}

/// Every public path, for sitemap.xml.
pub async fn sitemap_paths(db: &PgPool) -> Res<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        r#"SELECT path FROM pages WHERE status = 'published'
           UNION ALL SELECT '/academics/' || slug FROM programmes WHERE status = 'published'
           UNION ALL SELECT '/departments/' || slug FROM departments
           UNION ALL SELECT '/news/' || id::text FROM news
                     WHERE status = 'published' AND published_at <= now()"#,
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|(p,)| p).collect())
}
