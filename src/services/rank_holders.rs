//! Admin CRUD for the university rank holders.
//!
//! The `rank_holders` table is content like any other — a name, a rank, a
//! department, a year and a photo — but until now nothing on the admin side
//! touched it, so the list could only be changed with SQL. These functions back
//! a list screen where the IT admin adds a holder, uploads their photo, fixes a
//! rank, reorders the list and archives an old year.

use sqlx::{FromRow, PgPool};

use crate::layout::DisplayOptions;

type Res<T> = Result<T, sqlx::Error>;

// ---------- Reading ----------

/// One row of the admin rank holder list.
#[derive(Debug, FromRow)]
pub struct RankHolderRow {
    pub id: i64,
    pub name: String,
    pub rank: i32,
    pub year: i32,
    pub department: Option<String>,
    pub photo: Option<String>,
    pub status: String,
}

/// Every rank holder, newest year first, with an optional filter on publication
/// status. Ordered so the list reads the way the public page does.
pub async fn rank_holders(db: &PgPool, status: &str) -> Res<Vec<RankHolderRow>> {
    sqlx::query_as::<_, RankHolderRow>(
        r#"SELECT r.id, r.name, r.rank_position AS rank, r.exam_year AS year,
                  d.name AS department,
                  r.photo_path AS photo,
                  r.status
           FROM rank_holders r
           LEFT JOIN departments d ON d.id = r.department_id
           WHERE ($1 = '' OR r.status = $1)
           ORDER BY r.exam_year DESC, r.sort_order, r.rank_position, r.name"#,
    )
    .bind(status)
    .fetch_all(db)
    .await
}

/// One holder as the edit form wants it, plus the department id the form selects
/// on. The department is stored as an id but shown as a name.
#[derive(Debug, FromRow)]
pub struct RankHolderEdit {
    pub id: i64,
    pub name: String,
    pub rank: i32,
    pub year: i32,
    pub department_id: Option<i64>,
    pub photo: Option<String>,
    pub status: String,
}

/// The departments a holder can be placed in, for the form's select.
#[derive(Debug, FromRow)]
pub struct DepartmentOption {
    pub id: i64,
    pub name: String,
}

pub async fn rank_holder_for_edit(db: &PgPool, id: i64) -> Res<Option<RankHolderEdit>> {
    sqlx::query_as::<_, RankHolderEdit>(
        r#"SELECT id, name, rank_position AS rank, exam_year AS year,
                  department_id, photo_path AS photo, status
           FROM rank_holders WHERE id = $1"#,
    )
    .bind(id)
    .fetch_optional(db)
    .await
}

pub async fn departments(db: &PgPool) -> Res<Vec<DepartmentOption>> {
    sqlx::query_as::<_, DepartmentOption>("SELECT id, name FROM departments ORDER BY sort_order, name")
        .fetch_all(db)
        .await
}

/// The photo path a holder currently owns, so a rejected replacement upload can
/// be undone without deleting the file the row still points at.
///
/// Decoded as an `Option` because `photo_path` is nullable and most holders have
/// no photo yet.
pub async fn photo_for_rank_holder(db: &PgPool, id: i64) -> Res<Option<String>> {
    sqlx::query_scalar::<_, Option<String>>("SELECT photo_path FROM rank_holders WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await
        .map(|inner| inner.flatten())
}

// ---------- Writing ----------

/// The values a rank holder form posts. `department_id` is optional, so a
/// university-wide holder can sit with no department.
#[derive(Debug)]
pub struct RankHolderInput {
    pub name: String,
    pub rank: i32,
    pub year: i32,
    pub department_id: Option<i64>,
    pub photo: String,
    pub status: String,
}

pub async fn create_rank_holder(db: &PgPool, r: &RankHolderInput) -> Res<i64> {
    sqlx::query_scalar(
        r#"INSERT INTO rank_holders
             (name, rank_position, exam_year, department_id, photo_path, status, sort_order)
           VALUES ($1, $2, $3, $4, NULLIF($5, ''), $6,
                   COALESCE((SELECT max(sort_order) + 1 FROM rank_holders WHERE exam_year = $3), 1))
           RETURNING id"#,
    )
    .bind(&r.name)
    .bind(r.rank)
    .bind(r.year)
    .bind(r.department_id)
    .bind(&r.photo)
    .bind(crate::services::content_admin::clean_status(&r.status))
    .fetch_one(db)
    .await
}

pub async fn update_rank_holder(db: &PgPool, id: i64, r: &RankHolderInput) -> Res<()> {
    sqlx::query(
        r#"UPDATE rank_holders
           SET name = $2, rank_position = $3, exam_year = $4, department_id = $5,
               photo_path = NULLIF($6, ''), status = $7
           WHERE id = $1"#,
    )
    .bind(id)
    .bind(&r.name)
    .bind(r.rank)
    .bind(r.year)
    .bind(r.department_id)
    .bind(&r.photo)
    .bind(crate::services::content_admin::clean_status(&r.status))
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete_rank_holder(db: &PgPool, id: i64) -> Res<()> {
    sqlx::query("DELETE FROM rank_holders WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn set_rank_holder_status(db: &PgPool, id: i64, status: &str) -> Res<()> {
    sqlx::query("UPDATE rank_holders SET status = $2 WHERE id = $1")
        .bind(id)
        .bind(crate::services::content_admin::clean_status(status))
        .execute(db)
        .await?;
    Ok(())
}

/// Swap a holder with its neighbour within the same exam year, so the order can
/// be fixed without typing numbers.
pub async fn move_rank_holder(db: &PgPool, id: i64, up: bool) -> Res<()> {
    let current: Option<(i32, i32)> =
        sqlx::query_as("SELECT exam_year, sort_order FROM rank_holders WHERE id = $1")
            .bind(id)
            .fetch_optional(db)
            .await?;
    let Some((year, order)) = current else {
        return Ok(());
    };

    // Neighbours are looked for inside the same exam year, so reordering one
    // year cannot shuffle another.
    let neighbour: Option<i64> = if up {
        sqlx::query_scalar(
            "SELECT id FROM rank_holders WHERE exam_year = $1 AND sort_order < $2
             ORDER BY sort_order DESC, id DESC LIMIT 1",
        )
        .bind(year)
        .bind(order)
        .fetch_optional(db)
        .await?
    } else {
        sqlx::query_scalar(
            "SELECT id FROM rank_holders WHERE exam_year = $1 AND sort_order > $2
             ORDER BY sort_order, id LIMIT 1",
        )
        .bind(year)
        .bind(order)
        .fetch_optional(db)
        .await?
    };
    let Some(other) = neighbour else {
        return Ok(());
    };
    let other_order: i32 = sqlx::query_scalar("SELECT sort_order FROM rank_holders WHERE id = $1")
        .bind(other)
        .fetch_one(db)
        .await?;

    // Both rows take the other's order in one statement, so the pair is never
    // seen half-swapped. Note the swap: the row being moved takes its
    // neighbour's order, not its own.
    sqlx::query(
        r#"UPDATE rank_holders
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

// ---------- Display options for the list ----------

/// The six display choices for the rank holder list, read from the
/// `rank_holders` home section so the home page block and the full page agree.
pub async fn rank_holders_display(db: &PgPool) -> Res<DisplayOptions> {
    crate::layout::rank_holders_options(db).await
}

/// Write the rank holder list's display choices onto that same home section row.
pub async fn save_rank_holders_display(
    db: &PgPool,
    heading: &str,
    body: &str,
    photo: &str,
    opts: &DisplayOptions,
) -> Res<bool> {
    // This screen has no "online" or caption fields of its own — those live on
    // the home page block form — so the row keeps what it already holds.
    let (caption, published): (String, bool) =
        sqlx::query_as("SELECT photo_caption, published FROM home_sections WHERE section_key = $1")
            .bind(crate::layout::RANK_HOLDERS_KEY)
            .fetch_optional(db)
            .await?
            .unwrap_or((String::new(), true));
    crate::services::site_admin::update_home_section(
        db,
        crate::layout::RANK_HOLDERS_KEY,
        heading,
        body,
        photo,
        &caption,
        published,
        opts,
    )
    .await
}