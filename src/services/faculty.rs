//! Published teaching staff, grouped by department, with the HOD listed first.

use sqlx::PgPool;

type Res<T> = Result<T, sqlx::Error>;

/// One published faculty member, ready for a public card.
#[derive(Debug, Clone)]
pub struct FacultyCard {
    pub name: String,
    pub designation: String,
    pub qualification: String,
    pub biography: String,
    pub email: Option<String>,
    pub photo: Option<String>,
    pub is_hod: bool,
    pub department: String,
    pub department_slug: String,
    pub initials: Option<String>,
}

#[derive(sqlx::FromRow, Debug, Clone)]
struct FacultyRow {
    pub name: String,
    pub designation: String,
    pub qualification: String,
    pub bio: String,
    pub email: Option<String>,
    pub photo_path: Option<String>,
    pub is_hod: bool,
    pub department: String,
    pub department_slug: String,
}

impl FacultyCard {
    fn from_row(r: FacultyRow) -> Self {
        Self {
            name: r.name,
            designation: r.designation,
            qualification: r.qualification,
            biography: r.bio,
            email: r.email,
            photo: r.photo_path,
            is_hod: r.is_hod,
            department: r.department,
            department_slug: r.department_slug,
            initials: None,
        }
    }
}

/// One department, with the cards in the order the page should render them.
pub struct DepartmentGroup {
    pub slug: String,
    pub name: String,
    pub cards: Vec<FacultyCard>,
}

impl std::fmt::Debug for DepartmentGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DepartmentGroup")
            .field("slug", &self.slug)
            .field("name", &self.name)
            .field("cards", &self.cards)
            .finish()
    }
}

/// Every department that has at least one published faculty member, with those
/// members in display order: head of department first, then the rest by
/// `faculty.sort_order`, then by name so a missing sort order still reads
/// stably.
///
/// Only `status = 'published'` faculty are shown, matching the other public
/// people lists on the site (rank holders, etc.).
pub async fn faculty_groups(db: &PgPool) -> Res<Vec<DepartmentGroup>> {
    let rows: Vec<FacultyRow> =
        sqlx::query_as(
            r#"
            SELECT
                f.name,
                f.designation,
                f.qualification,
                f.bio,
                f.email,
                f.photo_path,
                f.is_hod,
                d.name AS department,
                d.slug AS department_slug
            FROM faculty f
            JOIN departments d ON d.id = f.department_id
            WHERE f.status = 'published'
            ORDER BY d.sort_order, d.name, f.is_hod DESC, f.sort_order, f.name
            "#,
        )
        .fetch_all(db)
        .await?;

    let mut groups: Vec<DepartmentGroup> = Vec::new();
    let mut last_department: Option<(String, String)> = None;

    for row in rows {
        let card = FacultyCard::from_row(row);
        let department = (card.department_slug.clone(), card.department.clone());
        if last_department.as_ref() != Some(&department) {
            groups.push(DepartmentGroup {
                slug: card.department_slug.clone(),
                name: card.department.clone(),
                cards: Vec::new(),
            });
            last_department = Some(department);
        }
        groups.last_mut().unwrap().cards.push(card);
    }

    Ok(groups)
}

// ---------- A teacher's own profile ----------

/// A teacher's own profile row, addressed by their account, for the
/// self-service profile page. It is a narrower view than [`FacultyRow`]: the
/// department and the HOD flags are the IT admin's to set, not the teacher's.
#[derive(Debug, sqlx::FromRow)]
pub struct TeacherProfile {
    pub name: String,
    pub designation: String,
    pub qualification: String,
    pub email: Option<String>,
    pub bio: String,
    pub photo_path: Option<String>,
}

/// The teacher's own `faculty` row, if their account has one.
pub async fn profile_for_user(db: &PgPool, user_id: i64) -> Res<Option<TeacherProfile>> {
    sqlx::query_as::<_, TeacherProfile>(
        r#"SELECT name, designation, qualification, email, bio, photo_path
             FROM faculty WHERE user_id = $1"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
}

/// The photo a teacher's own profile currently points at, so a rejected
/// replacement upload can be undone without dropping the file still in use.
pub async fn photo_for_user(db: &PgPool, user_id: i64) -> Res<Option<String>> {
    sqlx::query_scalar::<_, Option<String>>("SELECT photo_path FROM faculty WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(db)
        .await
        .map(|inner| inner.flatten())
}

/// Update the teacher's own profile. Returns false when the account has no
/// `faculty` row, so the caller can fall back to a `staff_profiles` row.
///
/// The designation, qualification, biography and photo are the teacher's own;
/// the department, the HOD flag and the account are deliberately not touched.
pub async fn save_profile(
    db: &PgPool,
    user_id: i64,
    name: &str,
    designation: &str,
    qualification: &str,
    email: &str,
    bio: &str,
    photo: &str,
) -> Res<bool> {
    let done = sqlx::query(
        r#"UPDATE faculty
              SET name = $2, designation = $3, qualification = $4,
                  email = NULLIF($5, ''), bio = $6, photo_path = NULLIF($7, '')
            WHERE user_id = $1"#,
    )
    .bind(user_id)
    .bind(name.trim())
    .bind(designation.trim())
    .bind(qualification.trim())
    .bind(email.trim())
    .bind(bio.trim())
    .bind(photo)
    .execute(db)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// A compact public-facing initials fallback for a faculty photo, using the
/// first two words of the name.
pub fn initials(card: &FacultyCard) -> String {
    card
        .name
        .split_whitespace()
        .take(2)
        .filter_map(|w| w.chars().next())
        .collect::<String>()
        .to_uppercase()
}
