//! Queries for the public site. Plain `query_as` (not the compile-time macros)
//! so the project builds without a live database.

use sqlx::PgPool;

use crate::layout::DisplayOptions;
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

const NEWS_COLUMNS: &str = r#"title,
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
    // The six display columns come straight from the section's own row, so a page
    // section carries its own look. They are read as plain values and folded into
    // a `DisplayOptions` here, because the stored strings are cleaned on the way in.
    let sql = format!(
        r#"SELECT heading, body, photo_path,
                  layout, grid_columns, image_align, text_align, photo_shape, photo_size
           FROM page_sections WHERE page_id = $1 ORDER BY sort_order, id"#
    );
    let rows: Vec<(String, String, Option<String>, String, i32, String, String, String, String)> =
        sqlx::query_as(&sql).bind(page_id).fetch_all(db).await?;

    Ok(rows
        .into_iter()
        .map(|(heading, body, photo, l, c, ia, ta, ps, pz)| PageSection {
            heading,
            body,
            photo,
            opts: DisplayOptions::from_row(&l, c, &ia, &ta, &ps, &pz),
        })
        .collect())
}

/// The college's own facts, assembled from `site_settings`.
///
/// Placeholders left in the stored copy (`{site_name}`, `{phone}`, `{year}`)
/// are filled in here, so an editor can write "Call {phone}" and get the
/// current number without touching a template.
pub async fn site_info(db: &PgPool) -> Res<SiteInfo> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT key, value FROM site_settings").fetch_all(db).await?;
    let get = |key: &str| {
        rows.iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };

    let phones = ["contact_phone_1", "contact_phone_2"]
        .iter()
        .map(|k| get(k))
        .filter(|v| !v.is_empty())
        .map(|display| Phone {
            tel: display
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '+')
                .collect(),
            display,
        })
        .collect::<Vec<_>>();
    let first_phone = phones.first();

    let name = get("site_name");
    let phone_display = first_phone.map(|p| p.display.clone()).unwrap_or_default();
    let phone_tel = first_phone.map(|p| p.tel.clone()).unwrap_or_default();
    // Copies, so the struct below can still take ownership of the originals.
    let fill_name = name.clone();
    let fill_phone = phone_display.clone();
    let fill_year = current_year().to_string();
    let fill = move |text: String| {
        text.replace("{site_name}", &fill_name)
            .replace("{phone}", &fill_phone)
            .replace("{year}", &fill_year)
    };

    Ok(SiteInfo {
        name,
        tagline: get("site_tagline"),
        header_note: get("site_header_note"),
        meta_description: get("site_meta_description"),
        footer_tagline: get("site_footer_tagline"),
        footer_legal: fill(get("site_footer_legal")),
        map_embed_url: get("site_map_embed_url"),
        map_title: get("site_map_title"),
        admissions_cta: get("site_admissions_cta"),
        research_note: get("site_research_note"),
        university_short: get("site_university_short"),
        contact_title: get("contact_title"),
        contact_lede: get("contact_lede"),
        contact_address: get("contact_address"),
        contact_email: get("contact_email"),
        phone_display,
        phone_tel,
        contact_phones: phones,
        page_placeholder_heading: get("page_placeholder_heading"),
        page_placeholder_body: fill(get("page_placeholder_body")),
        login_description: fill(get("login_description")),
        error_404_heading: get("error_404_heading"),
        error_404_body: get("error_404_body"),
        error_403_heading: get("error_403_heading"),
        error_403_body: get("error_403_body"),
        error_400_heading: get("error_400_heading"),
        error_400_body: get("error_400_body"),
        error_500_heading: get("error_500_heading"),
        error_500_body: fill(get("error_500_body")),
    })
}

/// The current calendar year, for the footer and any dated copy.
pub fn current_year() -> i32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    // Days since 1970-01-01 to a proleptic Gregorian year (cycles every 400 years).
    (1970 + (days * 400) / 146_097) as i32
}

/// The homepage copy, addressed by `home_sections.section_key`.
pub async fn home_copy(db: &PgPool) -> Res<HomeCopy> {
    let sql = format!(
        r#"SELECT section_key, heading, body, photo_path,
                  layout, grid_columns, image_align, text_align, photo_shape, photo_size
           FROM home_sections ORDER BY sort_order, section_key"#
    );
    let raw: Vec<(String, String, String, Option<String>, String, i32, String, String, String, String)> =
        sqlx::query_as(&sql).fetch_all(db).await?;
    let rows: Vec<HomeSection> = raw
        .into_iter()
        .map(
            |(section_key, heading, body, photo, l, c, ia, ta, ps, pz)| HomeSection {
                section_key,
                heading,
                body,
                photo,
                opts: DisplayOptions::from_row(&l, c, &ia, &ta, &ps, &pz),
            },
        )
        .collect();
    let find = |key: &str| {
        rows.iter()
            .find(|r| r.section_key == key)
            .map(|r| (r.heading.clone(), r.body.clone()))
            .unwrap_or_default()
    };
    let stats = rows
        .iter()
        .filter(|r| r.section_key.starts_with("stat_"))
        .map(|r| HomeStat {
            label: r.heading.clone(),
            value: r.body.clone(),
        })
        .collect();

    let (hero_heading, hero_body) = find("hero");
    let (latest_heading, _) = find("latest");
    let (programmes_heading, programmes_body) = find("programmes");
    let (admissions_heading, admissions_body) = find("admissions");
    let (rank_holders_heading, _) = find("rank_holders");
    let (units_heading, _) = find("units");
    let (history_heading, history_body) = find("history");
    let (vision_heading, vision_body) = find("vision");
    let (mission_heading, mission_body) = find("mission");
    let (milestones_heading, _) = find("milestones");
    let (facilities_heading, _) = find("facilities");
    let (contact_heading, _) = find("contact");

    Ok(HomeCopy {
        photos: rows
            .iter()
            .filter_map(|r| r.photo.clone().map(|p| (r.section_key.clone(), p)))
            .collect(),
        layouts: rows.iter().map(|r| (r.section_key.clone(), r.opts)).collect(),
        hero_heading,
        hero_body,
        stats,
        latest_heading,
        programmes_heading,
        programmes_body,
        admissions_heading,
        admissions_body,
        rank_holders_heading,
        units_heading,
        history_heading,
        history_body,
        vision_heading,
        vision_body,
        mission_heading,
        mission_body,
        milestones_heading,
        facilities_heading,
        contact_heading,
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
