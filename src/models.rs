//! Read models for the public site. Each struct maps one query in `services::content`.
//! Student Hub and admin models arrive in Layer 4.

use sqlx::FromRow;

#[derive(Debug, FromRow)]
pub struct Notice {
    pub title: String,
    pub category: String,
    pub href: String,
    pub is_new: bool,
}

#[derive(Debug, FromRow)]
pub struct NewsItem {
    pub id: i64,
    pub title: String,
    pub date: String,
    pub href: String,
    pub image: Option<String>,
    pub body: String,
}

#[derive(Debug, FromRow)]
pub struct Department {
    pub slug: String,
    pub name: String,
    pub summary: String,
}

#[derive(Debug, FromRow)]
pub struct Programme {
    pub slug: String,
    pub name: String,
    pub department: String,
    pub department_slug: String,
    pub level: String,
    pub summary: String,
    pub href: String,
}

#[derive(Debug, FromRow)]
pub struct RankHolder {
    pub name: String,
    pub rank: i32,
    pub department: Option<String>,
    pub year: i32,
    pub photo: Option<String>,
}

impl RankHolder {
    /// First letters of the first two words of the name, for the avatar fallback.
    pub fn initials(&self) -> String {
        self.name
            .split_whitespace()
            .take(2)
            .filter_map(|w| w.chars().next())
            .collect::<String>()
            .to_uppercase()
    }
}

#[derive(Debug, FromRow)]
pub struct Unit {
    pub name: String,
    pub full_name: String,
    pub text: String,
    pub values: String,
    pub href: String,
    pub image: Option<String>,
}

#[derive(Debug, FromRow)]
pub struct Facility {
    pub name: String,
    pub text: String,
}

#[derive(Debug, FromRow)]
pub struct Milestone {
    pub when: String,
    pub text: String,
}

#[derive(Debug, FromRow)]
pub struct Page {
    pub id: i64,
    pub title: String,
    pub lede: String,
}

#[derive(Debug, FromRow)]
pub struct PageSection {
    pub heading: String,
    pub body: String,
}

impl PageSection {
    pub fn paragraphs(&self) -> Vec<&str> {
        self.body
            .split("\n\n")
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect()
    }
}

#[derive(Debug)]
pub struct Phone {
    pub display: String,
    pub tel: String,
}

#[derive(Debug)]
pub struct ContactInfo {
    pub address: String,
    pub email: String,
    pub phones: Vec<Phone>,
}
