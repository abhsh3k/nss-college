//! Read models for the public site. Each struct maps one query in `services::content`.
//! Student Hub and admin models arrive in Layer 4.

use std::collections::HashMap;

use sqlx::FromRow;

use crate::layout::DisplayOptions;

#[derive(Debug, FromRow)]
pub struct Notice {
    pub title: String,
    pub category: String,
    pub href: String,
    pub is_new: bool,
}

#[derive(Debug, FromRow)]
pub struct NewsItem {
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

/// One section of an informational page, with how it should be presented.
///
/// The six display columns are folded into one `DisplayOptions` rather than kept
/// as loose fields, so a template can pass `opts` straight to the shared
/// rendering partial.
#[derive(Debug)]
pub struct PageSection {
    pub heading: String,
    pub body: String,
    pub photo: Option<String>,
    /// A caption drawn under the photo, when the admin wrote one.
    pub caption: String,
    pub opts: DisplayOptions,
}

impl PageSection {
    /// The section's body split into paragraphs on blank lines, which is what
    /// both layouts render: as stacked paragraphs, or one per card.
    pub fn paragraphs(&self) -> Vec<&str> {
        self.body
            .split("\n\n")
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct Phone {
    pub display: String,
    pub tel: String,
}

/// One link in the public header menu.
///
/// The menu is drawn from `pages`: each row picks the group it sits in, the
/// words it is labelled with, and its position. Groups with no links are left
/// out of the header entirely, so taking the last page out of a menu removes
/// the menu.
#[derive(Debug, Clone)]
pub struct NavItem {
    pub label: String,
    pub href: String,
}

/// Every college fact the layouts and public pages render. Loaded from
/// `site_settings`, so no template carries an institution name, phone number
/// or legal line of its own.
#[derive(Debug, Default, Clone)]
pub struct SiteInfo {
    pub name: String,
    pub tagline: String,
    pub header_note: String,
    pub meta_description: String,
    pub footer_tagline: String,
    pub footer_legal: String,
    pub map_embed_url: String,
    pub map_title: String,
    pub admissions_cta: String,
    pub research_note: String,
    pub university_short: String,

    pub contact_title: String,
    pub contact_lede: String,
    pub contact_address: String,
    pub contact_email: String,
    pub contact_phones: Vec<Phone>,
    /// The first contact number, for the places that show only one.
    pub phone_display: String,
    pub phone_tel: String,

    pub page_placeholder_heading: String,
    pub page_placeholder_body: String,
    pub login_description: String,

    pub error_404_heading: String,
    pub error_404_body: String,
    pub error_403_heading: String,
    pub error_403_body: String,
    pub error_400_heading: String,
    pub error_400_body: String,
    pub error_500_heading: String,
    pub error_500_body: String,

    /// The dark strip above the header: IQAC, Placement, Gallery, RTI, Fees.
    pub utility_nav: Vec<NavItem>,
    /// Top-level links after the three menus (Alumni, News). "Contact" is a
    /// route of its own rather than a page row, so the header draws it after
    /// this group.
    pub top_nav: Vec<NavItem>,
    pub about_nav: Vec<NavItem>,
    pub academics_nav: Vec<NavItem>,
    pub student_life_nav: Vec<NavItem>,
}

/// One homepage block, from `home_sections`.
#[derive(Debug)]
pub struct HomeSection {
    pub section_key: String,
    pub heading: String,
    pub body: String,
    pub photo: Option<String>,
    /// A caption drawn under the photo, when the admin wrote one.
    pub caption: String,
    /// False when the admin has taken the block offline.
    pub published: bool,
    pub opts: DisplayOptions,
}

/// A label/value pair from the `stat_*` homepage sections.
#[derive(Debug)]
pub struct HomeStat {
    pub label: String,
    pub value: String,
}

/// The homepage copy, keyed by section so `home.html` never holds a literal.
#[derive(Debug, Default)]
pub struct HomeCopy {
    /// Each block's uploaded photo, keyed by `section_key`.
    pub photos: HashMap<String, String>,
    /// Each block's photo caption, keyed by `section_key`.
    pub captions: HashMap<String, String>,
    /// Whether the admin has put each block online, keyed by `section_key`.
    pub published: HashMap<String, bool>,
    /// Each block's display choices, keyed by `section_key`, so a home block can
    /// be a card grid or plain prose like any other section.
    pub layouts: HashMap<String, DisplayOptions>,
    pub hero_heading: String,
    pub hero_body: String,
    pub stats: Vec<HomeStat>,
    pub latest_heading: String,
    pub programmes_heading: String,
    pub programmes_body: String,
    pub admissions_heading: String,
    pub admissions_body: String,
    pub rank_holders_heading: String,
    pub units_heading: String,
    pub history_heading: String,
    pub history_body: String,
    pub vision_heading: String,
    pub vision_body: String,
    pub mission_heading: String,
    pub mission_body: String,
    pub milestones_heading: String,
    pub facilities_heading: String,
    pub contact_heading: String,
}

impl HomeCopy {
    /// The mission block is a bulleted list, one item per paragraph.
    pub fn mission_items(&self) -> Vec<&str> {
        self.mission_body
            .split("\n\n")
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect()
    }

    /// The photo uploaded for a home block, if the admin added one.
    ///
    /// `home.html` addresses blocks by their `section_key` through these, which
    /// is why the lookup lives here rather than in a dozen template fields.
    pub fn photo(&self, key: &str) -> Option<&str> {
        self.photos.get(key).map(String::as_str)
    }

    /// True when the block has a photo, so the template can skip the markup.
    pub fn has_photo(&self, key: &str) -> bool {
        self.photos.contains_key(key)
    }

    /// The caption written for a block's photo, or an empty string.
    pub fn caption(&self, key: &str) -> &str {
        self.captions.get(key).map(String::as_str).unwrap_or("")
    }

    /// True unless the admin has taken the block offline.
    ///
    /// A block missing from the table (a database that lost the seeded rows)
    /// counts as published, so the home page never blanks itself out by
    /// accident.
    pub fn is_published(&self, key: &str) -> bool {
        self.published.get(key).copied().unwrap_or(true)
    }

    /// The display choices for a home block.
    ///
    /// A block with no stored options falls back to prose, because most home
    /// copy is sentences rather than a list of items. The rank holders block is
    /// the exception the migration seeds as cards.
    pub fn opts(&self, key: &str) -> DisplayOptions {
        self.layouts
            .get(key)
            .copied()
            .unwrap_or(DisplayOptions {
                layout: crate::layout::Layout::Paragraphs,
                ..DisplayOptions::starter()
            })
    }

    /// A block's text split into paragraphs on blank lines.
    ///
    /// The shared body partial renders either prose or one card per paragraph,
    /// and an askama `{% let %}` cannot bind a `String` out of the template, so
    /// the block is addressed by key and read here instead.
    pub fn paragraphs_of(&self, key: &str) -> Vec<&str> {
        match key {
            "hero" => Some(&self.hero_body),
            "programmes" => Some(&self.programmes_body),
            "admissions" => Some(&self.admissions_body),
            "history" => Some(&self.history_body),
            "vision" => Some(&self.vision_body),
            "mission" => Some(&self.mission_body),
            _ => None,
        }
        .map(|body| {
            body.split("\n\n")
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_default()
    }
}
