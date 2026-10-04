//! Admin CRUD for documents, informational pages and site settings.
//!
//! These tables back the three remaining greyed-out menu entries (Documents,
//! Pages, Settings). Split out from `content_admin` because they are `AdminOnly`
//! rather than open to office staff, and because the settings screen writes
//! key/value rows rather than whole records.

use sqlx::{FromRow, PgPool};

use crate::layout::DisplayOptions;

type Res<T> = Result<T, sqlx::Error>;

// ---------- Documents ----------

/// A downloadable file published on the public site.
#[derive(Debug, FromRow)]
pub struct DocumentRow {
    pub id: i64,
    pub title: String,
    pub category: String,
    pub file_path: String,
    pub file_name: String,
    pub status: String,
    pub published_label: String,
    pub size_label: String,
}

#[derive(Debug)]
pub struct DocumentInput {
    pub title: String,
    pub category: String,
    pub file_path: String,
    pub status: String,
}

#[derive(Debug, FromRow)]
pub struct DocumentEdit {
    pub id: i64,
    pub title: String,
    pub category: String,
    pub file_path: String,
    pub status: String,
}

/// The filename shown to a reader, which is the last path segment.
fn file_name_of(path: &str) -> String {
    path.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(path).to_string()
}

/// A human-readable size, so the list does not show a raw byte count.
fn size_label(bytes: Option<i64>) -> String {
    let Some(n) = bytes else {
        return "—".into();
    };
    let mb = n as f64 / 1024.0 / 1024.0;
    if mb >= 1.0 {
        format!("{mb:.1} MB")
    } else {
        format!("{:.0} KB", n as f64 / 1024.0)
    }
}

pub async fn documents(db: &PgPool, status: &str) -> Res<Vec<DocumentRow>> {
    let rows: Vec<(i64, String, String, String, String, String, Option<String>, Option<i64>)> =
        sqlx::query_as(
            r#"SELECT d.id, d.title, d.category, d.file_path, d.status,
                      COALESCE(to_char(d.published_at, 'DD Mon YYYY'), '—'),
                      u.original_name, u.size_bytes
               FROM documents d
               LEFT JOIN uploads u ON u.path = d.file_path
               WHERE ($1 = '' OR d.status = $1)
               ORDER BY d.created_at DESC"#,
        )
        .bind(status)
        .fetch_all(db)
        .await?;

    Ok(rows
        .into_iter()
        .map(|(id, title, category, file_path, status, published_label, original, size)| {
            DocumentRow {
                id,
                title,
                category,
                file_name: original.unwrap_or_else(|| file_name_of(&file_path)),
                file_path,
                status,
                published_label,
                size_label: size_label(size),
            }
        })
        .collect())
}

pub async fn document_for_edit(db: &PgPool, id: i64) -> Res<Option<DocumentEdit>> {
    sqlx::query_as::<_, DocumentEdit>(
        r#"SELECT id, title, category, file_path, status FROM documents WHERE id = $1"#,
    )
    .bind(id)
    .fetch_optional(db)
    .await
}

pub async fn create_document(db: &PgPool, d: &DocumentInput, author: Option<i64>) -> Res<i64> {
    sqlx::query_scalar(
        r#"INSERT INTO documents (title, category, file_path, status, published_at, created_by)
           VALUES ($1, $2, $3, $4,
                   CASE WHEN $4 = 'published' THEN now() ELSE NULL END, $5)
           RETURNING id"#,
    )
    .bind(&d.title)
    .bind(&d.category)
    .bind(&d.file_path)
    .bind(crate::services::content_admin::clean_status(&d.status))
    .bind(author)
    .fetch_one(db)
    .await
}

pub async fn update_document(db: &PgPool, id: i64, d: &DocumentInput) -> Res<()> {
    sqlx::query(
        r#"UPDATE documents
           SET title = $2, category = $3, file_path = $4, status = $5,
               published_at = CASE WHEN $5 = 'published' THEN COALESCE(published_at, now()) ELSE published_at END
           WHERE id = $1"#,
    )
    .bind(id)
    .bind(&d.title)
    .bind(&d.category)
    .bind(&d.file_path)
    .bind(crate::services::content_admin::clean_status(&d.status))
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete_document(db: &PgPool, id: i64) -> Res<()> {
    sqlx::query("DELETE FROM documents WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn set_document_status(db: &PgPool, id: i64, status: &str) -> Res<()> {
    sqlx::query(
        r#"UPDATE documents
           SET status = $2,
               published_at = CASE WHEN $2 = 'published' THEN COALESCE(published_at, now()) ELSE published_at END
           WHERE id = $1"#,
    )
    .bind(id)
    .bind(crate::services::content_admin::clean_status(status))
    .execute(db)
    .await?;
    Ok(())
}

pub async fn upload_path_for_document(db: &PgPool, id: i64) -> Res<Option<String>> {
    sqlx::query_scalar("SELECT file_path FROM documents WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await
}

// ---------- Pages ----------

/// An informational page, served by its `path`.
#[derive(Debug, FromRow)]
pub struct PageRow {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub lede: String,
    pub status: String,
    pub section_count: i64,
    /// True when the page is one the router serves from its own handler, so
    /// renaming it here would have no effect until that handler changes.
    pub is_list_page: bool,
}

#[derive(Debug)]
pub struct PageInput {
    pub path: String,
    pub title: String,
    pub lede: String,
    pub status: String,
}

#[derive(Debug, FromRow)]
pub struct PageEdit {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub lede: String,
    pub status: String,
}

/// Paths the router answers itself. Editing the title of one of these still
/// works, but the editor is warned that the path is fixed.
const LIST_PAGES: [&str; 5] = [
    "/academics",
    "/departments",
    "/news",
    "/notices",
    "/academics/rank-holders",
];

pub async fn pages(db: &PgPool, status: &str) -> Res<Vec<PageRow>> {
    let rows: Vec<(i64, String, String, String, String, i64)> = sqlx::query_as(
        r#"SELECT p.id, p.path, p.title, p.lede, p.status,
                  (SELECT count(*) FROM page_sections s WHERE s.page_id = p.id)
           FROM pages p
           WHERE ($1 = '' OR p.status = $1)
           ORDER BY p.path"#,
    )
    .bind(status)
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(id, path, title, lede, status, section_count)| PageRow {
            is_list_page: LIST_PAGES.contains(&path.as_str()),
            id,
            path,
            title,
            lede,
            status,
            section_count,
        })
        .collect())
}

pub async fn page_for_edit(db: &PgPool, id: i64) -> Res<Option<PageEdit>> {
    sqlx::query_as::<_, PageEdit>("SELECT id, path, title, lede, status FROM pages WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await
}

pub async fn create_page(db: &PgPool, p: &PageInput) -> Res<i64> {
    sqlx::query_scalar(
        r#"INSERT INTO pages (path, title, lede, status) VALUES ($1, $2, $3, $4) RETURNING id"#,
    )
    .bind(&p.path)
    .bind(&p.title)
    .bind(&p.lede)
    .bind(crate::services::content_admin::clean_status(&p.status))
    .fetch_one(db)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db) if db.code().as_deref() == Some("23505") => sqlx::Error::Protocol(
            "a page already uses that path".into(),
        ),
        other => other,
    })
}

pub async fn update_page(db: &PgPool, id: i64, p: &PageInput) -> Res<()> {
    sqlx::query("UPDATE pages SET path = $2, title = $3, lede = $4, status = $5 WHERE id = $1")
        .bind(id)
        .bind(&p.path)
        .bind(&p.title)
        .bind(&p.lede)
        .bind(crate::services::content_admin::clean_status(&p.status))
        .execute(db)
        .await?;
    Ok(())
}

pub async fn delete_page(db: &PgPool, id: i64) -> Res<()> {
    sqlx::query("DELETE FROM pages WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn set_page_status(db: &PgPool, id: i64, status: &str) -> Res<()> {
    sqlx::query("UPDATE pages SET status = $2 WHERE id = $1")
        .bind(id)
        .bind(crate::services::content_admin::clean_status(status))
        .execute(db)
        .await?;
    Ok(())
}

// ---------- Page sections ----------

/// A page section as the editor shows it: the copy, the uploaded photo, and the
/// six display choices.
#[derive(Debug)]
pub struct SectionRow {
    pub id: i64,
    pub heading: String,
    pub body: String,
    pub photo: Option<String>,
    pub opts: DisplayOptions,
}

pub async fn sections(db: &PgPool, page_id: i64) -> Res<Vec<SectionRow>> {
    let sql = "SELECT id, heading, body, photo_path,
                      layout, grid_columns, image_align, text_align, photo_shape, photo_size
               FROM page_sections WHERE page_id = $1 ORDER BY sort_order, id";
    let rows: Vec<(i64, String, String, Option<String>, String, i32, String, String, String, String)> =
        sqlx::query_as(sql).bind(page_id).fetch_all(db).await?;

    Ok(rows
        .into_iter()
        .map(|(id, heading, body, photo, l, c, ia, ta, ps, pz)| SectionRow {
            id,
            heading,
            body,
            photo,
            opts: DisplayOptions::from_row(&l, c, &ia, &ta, &ps, &pz),
        })
        .collect())
}

/// The photo path a section currently owns, so a rejected replacement upload can
/// be undone without deleting the file the row still points at.
///
/// The column is nullable and most rows are null, so it is decoded as an
/// `Option`: decoding a SQL NULL into a `String` would fail rather than yield an
/// empty value.
pub async fn section_photo(db: &PgPool, id: i64) -> Res<Option<String>> {
    sqlx::query_scalar::<_, Option<String>>("SELECT photo_path FROM page_sections WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await
        .map(|inner| inner.flatten())
}

pub async fn create_section(
    db: &PgPool,
    page_id: i64,
    heading: &str,
    body: &str,
    photo: &str,
    opts: &DisplayOptions,
) -> Res<i64> {
    let (layout, columns, image_align, text_align, photo_shape, photo_size) = opts.to_row();
    sqlx::query_scalar(
        r#"INSERT INTO page_sections
             (page_id, heading, body, sort_order, photo_path,
              layout, grid_columns, image_align, text_align, photo_shape, photo_size)
           VALUES ($1, $2, $3,
                   COALESCE((SELECT max(sort_order) + 1 FROM page_sections WHERE page_id = $1), 1),
                   NULLIF($4, ''),
                   $5, $6, $7, $8, $9, $10)
           RETURNING id"#,
    )
    .bind(page_id)
    .bind(heading)
    .bind(body)
    .bind(photo)
    .bind(layout)
    .bind(columns)
    .bind(image_align)
    .bind(text_align)
    .bind(photo_shape)
    .bind(photo_size)
    .fetch_one(db)
    .await
}

pub async fn update_section(
    db: &PgPool,
    id: i64,
    heading: &str,
    body: &str,
    photo: &str,
    opts: &DisplayOptions,
) -> Res<()> {
    let (layout, columns, image_align, text_align, photo_shape, photo_size) = opts.to_row();
    sqlx::query(
        r#"UPDATE page_sections
           SET heading = $2, body = $3, photo_path = NULLIF($4, ''),
               layout = $5, grid_columns = $6, image_align = $7,
               text_align = $8, photo_shape = $9, photo_size = $10
           WHERE id = $1"#,
    )
    .bind(id)
    .bind(heading)
    .bind(body)
    .bind(photo)
    .bind(layout)
    .bind(columns)
    .bind(image_align)
    .bind(text_align)
    .bind(photo_shape)
    .bind(photo_size)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete_section(db: &PgPool, id: i64) -> Res<()> {
    sqlx::query("DELETE FROM page_sections WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

/// Swap a section with its neighbour so the order can be fixed without numbers.
pub async fn move_section(db: &PgPool, id: i64, up: bool) -> Res<()> {
    let current: Option<(i64, i32)> =
        sqlx::query_as("SELECT page_id, sort_order FROM page_sections WHERE id = $1")
            .bind(id)
            .fetch_optional(db)
            .await?;
    let Some((page_id, order)) = current else {
        return Ok(());
    };
    let neighbour: Option<i64> = if up {
        sqlx::query_scalar(
            "SELECT id FROM page_sections WHERE page_id = $1 AND sort_order < $2
             ORDER BY sort_order DESC, id DESC LIMIT 1",
        )
        .bind(page_id)
        .bind(order)
        .fetch_optional(db)
        .await?
    } else {
        sqlx::query_scalar(
            "SELECT id FROM page_sections WHERE page_id = $1 AND sort_order > $2
             ORDER BY sort_order, id LIMIT 1",
        )
        .bind(page_id)
        .bind(order)
        .fetch_optional(db)
        .await?
    };
    let Some(other) = neighbour else {
        return Ok(());
    };
    let other_order: i32 = sqlx::query_scalar("SELECT sort_order FROM page_sections WHERE id = $1")
        .bind(other)
        .fetch_one(db)
        .await?;

    // Both rows take the other's order in one statement, so a unique index on
    // (page_id, sort_order) can never see a half-swapped state. Note the swap:
    // the section being moved takes its neighbour's order, not its own.
    sqlx::query(
        r#"UPDATE page_sections
           SET sort_order = CASE id
               WHEN $1 THEN $3
               WHEN $2 THEN $4
           END
           WHERE id IN ($1, $2)"#,
    )
    .bind(id)
    .bind(other)
    .bind(other_order)
    .bind(order)
    .execute(db)
    .await?;
    Ok(())
}

// ---------- Site settings ----------

/// One editable `site_settings` row, with a label for the form.
#[derive(Debug, FromRow)]
pub struct SettingRow {
    pub key: String,
    pub value: String,
    pub label: String,
    pub hint: String,
    pub multiline: bool,
}

/// The keys the settings screen offers, in the order they appear.
///
/// Only the presentation copy is listed here. Operational keys such as
/// `attendance_*` are deliberately left out: they belong to the attendance
/// screens, and offering them next to the college's phone number invites
/// someone to break marking by accident.
const SETTINGS: &[(&str, &str, &str, bool)] = &[
    ("site_name", "College name", "Shown in the header, the footer and every page title.", false),
    ("site_tagline", "Tagline", "The line under the college name in the header.", false),
    ("site_header_note", "Header note", "The small line at the very top of every page.", false),
    (
        "site_meta_description",
        "Meta description",
        "The summary search engines show. Used wherever a page has no description of its own.",
        true,
    ),
    ("site_footer_tagline", "Footer tagline", "The short sentence under the name in the footer.", true),
    (
        "site_footer_legal",
        "Footer legal line",
        "Leave {year} and {site_name} in place and they fill themselves in.",
        false,
    ),
    ("site_map_embed_url", "Map embed URL", "The src of the embedded map. Leave empty to hide the map.", true),
    ("site_map_title", "Map title", "The accessible name of the embedded map.", false),
    ("site_admissions_cta", "Admissions button", "The label on the call to action that leads to the admissions page.", false),
    ("site_research_note", "Research note", "Shown under the programme list. Leave empty to hide it.", true),
    ("site_university_short", "University (short)", "How the university is named beside a rank holder.", false),
    ("contact_title", "Contact page title", "", false),
    ("contact_lede", "Contact page introduction", "", true),
    ("contact_address", "Address", "", true),
    ("contact_phone_1", "Telephone (primary)", "Also used wherever a single number is shown.", false),
    ("contact_phone_2", "Telephone (secondary)", "Leave empty for a single number.", false),
    ("contact_email", "Email", "", false),
    (
        "page_placeholder_heading",
        "Placeholder page heading",
        "Shown when a page exists but has no content yet.",
        false,
    ),
    (
        "page_placeholder_body",
        "Placeholder page text",
        "{phone} is filled in from the primary number.",
        true,
    ),
    ("login_description", "Sign-in page description", "{site_name} is filled in for you.", true),
    ("error_404_heading", "404 heading", "", false),
    ("error_404_body", "404 message", "", true),
    ("error_403_heading", "403 heading", "", false),
    ("error_403_body", "403 message", "", true),
    ("error_400_heading", "400 heading", "", false),
    ("error_400_body", "400 message", "", true),
    ("error_500_heading", "500 heading", "", false),
    ("error_500_body", "500 message", "{phone} is filled in from the primary number.", true),
];

pub async fn settings(db: &PgPool) -> Res<Vec<SettingRow>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM site_settings WHERE key = ANY($1)",
    )
    .bind(SETTINGS.iter().map(|(k, _, _, _)| *k).collect::<Vec<_>>())
    .fetch_all(db)
    .await?;

    Ok(SETTINGS
        .iter()
        .map(|(key, label, hint, multiline)| {
            let value = rows
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            SettingRow {
                key: (*key).to_string(),
                value,
                label: (*label).to_string(),
                hint: (*hint).to_string(),
                multiline: *multiline,
            }
        })
        .collect())
}

/// Write one key. A key the settings screen does not offer is ignored, so a
/// crafted form cannot reach the attendance settings through this screen.
pub async fn set_setting(db: &PgPool, key: &str, value: &str) -> Res<bool> {
    if !SETTINGS.iter().any(|(k, _, _, _)| *k == key) {
        return Ok(false);
    }
    sqlx::query(
        r#"INSERT INTO site_settings (key, value) VALUES ($1, $2)
           ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value"#,
    )
    .bind(key)
    .bind(value)
    .execute(db)
    .await?;
    Ok(true)
}

// ---------- Homepage sections ----------

/// A block of homepage copy, addressed by its stable key, with its photo and the
/// six display choices.
#[derive(Debug)]
pub struct HomeSectionRow {
    pub section_key: String,
    pub heading: String,
    pub body: String,
    pub photo: Option<String>,
    pub opts: DisplayOptions,
}

pub async fn home_sections(db: &PgPool) -> Res<Vec<HomeSectionRow>> {
    let sql = "SELECT section_key, heading, body, photo_path,
                      layout, grid_columns, image_align, text_align, photo_shape, photo_size
               FROM home_sections ORDER BY sort_order, section_key";
    let rows: Vec<(String, String, String, Option<String>, String, i32, String, String, String, String)> =
        sqlx::query_as(sql).fetch_all(db).await?;

    Ok(rows
        .into_iter()
        .map(
            |(section_key, heading, body, photo, l, c, ia, ta, ps, pz)| HomeSectionRow {
                section_key,
                heading,
                body,
                photo,
                opts: DisplayOptions::from_row(&l, c, &ia, &ta, &ps, &pz),
            },
        )
        .collect())
}

/// The photo path a home block currently owns, for undoing a rejected upload.
///
/// Decoded as an `Option` because `photo_path` is nullable and a block with no
/// photo is the common case, not an error.
pub async fn home_section_photo(db: &PgPool, key: &str) -> Res<Option<String>> {
    sqlx::query_scalar::<_, Option<String>>("SELECT photo_path FROM home_sections WHERE section_key = $1")
        .bind(key)
        .fetch_optional(db)
        .await
        .map(|inner| inner.flatten())
}

/// Update one homepage block. An unknown key is refused so the form cannot
/// invent rows the template never reads.
pub async fn update_home_section(
    db: &PgPool,
    key: &str,
    heading: &str,
    body: &str,
    photo: &str,
    opts: &DisplayOptions,
) -> Res<bool> {
    let (layout, columns, image_align, text_align, photo_shape, photo_size) = opts.to_row();
    let updated = sqlx::query(
        r#"UPDATE home_sections
           SET heading = $2, body = $3, photo_path = NULLIF($4, ''),
               layout = $5, grid_columns = $6, image_align = $7,
               text_align = $8, photo_shape = $9, photo_size = $10
           WHERE section_key = $1"#,
    )
    .bind(key)
    .bind(heading)
    .bind(body)
    .bind(photo)
    .bind(layout)
    .bind(columns)
    .bind(image_align)
    .bind(text_align)
    .bind(photo_shape)
    .bind(photo_size)
    .execute(db)
    .await?;
    Ok(updated.rows_affected() > 0)
}