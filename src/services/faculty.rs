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
