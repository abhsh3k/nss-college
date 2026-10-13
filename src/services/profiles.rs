//! Self-service profiles for the Principal and office staff.
//!
//! A teacher's public card comes from their `faculty` row (see
//! [`crate::services::faculty`]), which the IT admin manages and the timetable
//! and courses reference. The Principal and office staff have no `faculty` row,
//! so their profile lives in `staff_profiles`, keyed by their account. This
//! module reads and writes that table and turns the rows into the same
//! [`FacultyCard`] the staff page already renders, so `/about/staff` shows
//! everyone through one template.

use sqlx::PgPool;

use crate::services::faculty::{DepartmentGroup, FacultyCard};

type Res<T> = Result<T, sqlx::Error>;

/// One stored profile, as the editor and the public list read it.
#[derive(Debug, sqlx::FromRow)]
pub struct ProfileRow {
    pub display_name: String,
    pub designation: String,
    pub qualification: String,
    pub email: Option<String>,
    pub bio: String,
    pub photo_path: Option<String>,
}

/// The profile a user has saved, if any.
pub async fn for_user(db: &PgPool, user_id: i64) -> Res<Option<ProfileRow>> {
    sqlx::query_as::<_, ProfileRow>(
        "SELECT display_name, designation, qualification, email, bio, photo_path
           FROM staff_profiles WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
}

/// The photo path a user's profile currently points at, so a rejected
/// replacement upload can be undone without dropping the file still in use.
pub async fn photo_for_user(db: &PgPool, user_id: i64) -> Res<Option<String>> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT photo_path FROM staff_profiles WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
    .map(|inner| inner.flatten())
}

/// Create or update a user's profile. `photo` is the whole stored path (empty
/// when the profile has no photo), already resolved by the caller.
pub async fn save(
    db: &PgPool,
    user_id: i64,
    display_name: &str,
    designation: &str,
    qualification: &str,
    email: &str,
    bio: &str,
    photo: &str,
) -> Res<()> {
    sqlx::query(
        "INSERT INTO staff_profiles
             (user_id, display_name, designation, qualification, email, bio, photo_path)
         VALUES ($1, $2, $3, $4, NULLIF($5, ''), $6, NULLIF($7, ''))
         ON CONFLICT (user_id) DO UPDATE SET
             display_name = EXCLUDED.display_name,
             designation = EXCLUDED.designation,
             qualification = EXCLUDED.qualification,
             email = EXCLUDED.email,
             bio = EXCLUDED.bio,
             photo_path = EXCLUDED.photo_path",
    )
    .bind(user_id)
    .bind(display_name.trim())
    .bind(designation.trim())
    .bind(qualification.trim())
    .bind(email.trim())
    .bind(bio.trim())
    .bind(photo)
    .execute(db)
    .await?;
    Ok(())
}

/// The published profiles of every active account in `role`, grouped under one
/// heading, in display order.
///
/// Only accounts that have actually saved a profile appear: the inner join on
/// `staff_profiles` is what makes the listing opt-in. `None` is returned when a
/// role has no published profile at all, so the page can leave the heading out.
pub async fn group_for_role(
    db: &PgPool,
    role: &str,
    slug: &str,
    name: &str,
) -> Res<Option<DepartmentGroup>> {
    let cards: Vec<FacultyCard> = sqlx::query_as::<_, ProfileCard>(
        "SELECT COALESCE(NULLIF(sp.display_name, ''), u.full_name) AS name,
                sp.designation AS designation,
                sp.qualification AS qualification,
                sp.bio AS biography,
                sp.email AS email,
                sp.photo_path AS photo
           FROM staff_profiles sp
           JOIN users u ON u.id = sp.user_id
          WHERE u.role = $1 AND u.is_active AND sp.published
          ORDER BY sp.sort_order, name",
    )
    .bind(role)
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|c| FacultyCard {
        name: c.name,
        designation: c.designation,
        qualification: c.qualification,
        biography: c.biography,
        email: c.email,
        photo: c.photo,
        is_hod: false,
        department: name.to_string(),
        department_slug: slug.to_string(),
        initials: None,
    })
    .collect();

    if cards.is_empty() {
        return Ok(None);
    }
    Ok(Some(DepartmentGroup {
        slug: slug.to_string(),
        name: name.to_string(),
        cards,
    }))
}

/// The shape one `staff_profiles` row is read into. Kept private: callers work
/// with [`FacultyCard`], the same type the faculty listing produces.
#[derive(sqlx::FromRow)]
struct ProfileCard {
    name: String,
    designation: String,
    qualification: String,
    biography: String,
    email: Option<String>,
    photo: Option<String>,
}
