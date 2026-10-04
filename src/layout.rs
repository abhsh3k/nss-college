//! How a section of the public site is presented.
//!
//! One set of six choices describes every block of content: whether it reads as
//! cards or as plain paragraphs, how many cards sit in a row, how the photo and
//! the text are aligned, and the shape and size of the photo. The values live in
//! the database beside the copy (see `migrations/0011_section_display.sql`), so
//! the IT admin restyles a block from the settings screen.
//!
//! Three rules keep this type honest:
//!
//! * Every value that reaches the database goes through one of the `clean_*`
//!   functions, so a hand-crafted form cannot store a string the renderer has no
//!     class for and end up with unstyled content.
//! * The options are small `Copy` enums rather than strings, which is what lets
//!   an askama `{% let %}` bind them: the template assigns one to a shared
//!   variable before including the shared form fragment.
//! * The Tailwind class strings live in the templates, written out literally,
//!   because the CSS build scans `templates/` for class names. A class assembled
//!   in Rust would be dropped by that scan and never reach `static/css`. So the
//!   predicates here answer a question the template asks — "is the photo round?"
//! — and the template picks the class.

use sqlx::PgPool;

type Res<T> = Result<T, sqlx::Error>;

/// Card grid or a column of paragraphs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Layout {
    /// Bordered cards over a grid.
    #[default]
    Cards,
    /// Plain text, one block after another.
    Paragraphs,
}

/// Where the photo sits relative to the text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// How the photo is cropped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Shape {
    #[default]
    Circle,
    Rounded,
    Square,
}

/// How large the photo is drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Size {
    Small,
    #[default]
    Medium,
    Large,
}

/// The six display choices for one section.
///
/// The `columns` count is a plain number rather than an enum because the
/// templates ask "is this exactly 3?" and a number reads better than a variant
/// for that.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DisplayOptions {
    pub layout: Layout,
    pub columns: i32,
    pub image_align: Align,
    pub text_align: Align,
    pub photo_shape: Shape,
    pub photo_size: Size,
}

impl DisplayOptions {
    /// The starting point for a brand new section when no defaults row exists.
    ///
    /// Prose, left aligned, with a medium photo. Most sections on this site are
    /// sentences rather than lists of items, so paragraphs is the safe default and
    /// cards are something an admin opts into per section.
    pub fn starter() -> DisplayOptions {
        DisplayOptions {
            layout: Layout::Paragraphs,
            columns: 3,
            image_align: Align::Left,
            text_align: Align::Left,
            photo_shape: Shape::Circle,
            photo_size: Size::Medium,
        }
    }

    // The predicates below are what the templates actually ask. Each returns a
    // bool rather than the value so an include can compare against a class.

    pub fn is_cards(&self) -> bool {
        self.layout == Layout::Cards
    }

    pub fn is_paragraphs(&self) -> bool {
        self.layout == Layout::Paragraphs
    }

    pub fn photo_is_left(&self) -> bool {
        self.image_align == Align::Left
    }

    pub fn photo_is_centre(&self) -> bool {
        self.image_align == Align::Center
    }

    pub fn photo_is_right(&self) -> bool {
        self.image_align == Align::Right
    }

    pub fn text_is_centre(&self) -> bool {
        self.text_align == Align::Center
    }

    pub fn text_is_right(&self) -> bool {
        self.text_align == Align::Right
    }

    pub fn shape_is_circle(&self) -> bool {
        self.photo_shape == Shape::Circle
    }

    pub fn shape_is_rounded(&self) -> bool {
        self.photo_shape == Shape::Rounded
    }

    pub fn shape_is_square(&self) -> bool {
        self.photo_shape == Shape::Square
    }

    pub fn size_is_small(&self) -> bool {
        self.photo_size == Size::Small
    }

    pub fn size_is_medium(&self) -> bool {
        self.photo_size == Size::Medium
    }

    pub fn size_is_large(&self) -> bool {
        self.photo_size == Size::Large
    }

    /// The photo's edge length in pixels, for the `width`/`height` attributes so
    /// the browser reserves the right box before the file arrives.
    pub fn photo_px(&self) -> u32 {
        match self.photo_size {
            Size::Small => 64,
            Size::Medium => 96,
            Size::Large => 160,
        }
    }

    /// Read the six columns a query returned, in the order they are selected.
    pub fn from_row(
        layout: &str,
        columns: i32,
        image_align: &str,
        text_align: &str,
        photo_shape: &str,
        photo_size: &str,
    ) -> DisplayOptions {
        DisplayOptions {
            layout: clean_layout(layout),
            // A stored count outside the offered range would leave a template
            // branch unmatched, so it is clamped rather than trusted.
            columns: columns.clamp(1, 4),
            image_align: clean_align(image_align),
            text_align: clean_align(text_align),
            photo_shape: clean_shape(photo_shape),
            photo_size: clean_size(photo_size),
        }
    }

    /// The six values ready to bind to a query.
    pub fn to_row(&self) -> (&'static str, i32, &'static str, &'static str, &'static str, &'static str) {
        (layout_str(self.layout), self.columns, align_str(self.image_align), align_str(self.text_align), shape_str(self.photo_shape), size_str(self.photo_size))
    }
}

fn layout_str(layout: Layout) -> &'static str {
    match layout {
        Layout::Cards => "cards",
        Layout::Paragraphs => "paragraphs",
    }
}

fn align_str(align: Align) -> &'static str {
    match align {
        Align::Left => "left",
        Align::Center => "center",
        Align::Right => "right",
    }
}

fn shape_str(shape: Shape) -> &'static str {
    match shape {
        Shape::Circle => "circle",
        Shape::Rounded => "rounded",
        Shape::Square => "square",
    }
}

fn size_str(size: Size) -> &'static str {
    match size {
        Size::Small => "sm",
        Size::Medium => "md",
        Size::Large => "lg",
    }
}

// ---------- Cleaning a posted value ----------

/// Accepts a layout from a form, falling back to cards.
pub fn clean_layout(value: &str) -> Layout {
    match value.trim() {
        "paragraphs" => Layout::Paragraphs,
        _ => Layout::Cards,
    }
}

/// Accepts a column count, falling back to three and never below one.
pub fn clean_columns(value: &str) -> i32 {
    value
        .trim()
        .parse::<i32>()
        .ok()
        .filter(|n| (1..=4).contains(n))
        .unwrap_or(3)
}

/// Accepts an alignment from a form. `centre` is taken as a spelling of
/// `center`, because the site copy is British English and a hand-written value
/// should not silently fall back to left.
pub fn clean_align(value: &str) -> Align {
    match value.trim() {
        "center" | "centre" => Align::Center,
        "right" => Align::Right,
        _ => Align::Left,
    }
}

/// Accepts a photo shape from a form, falling back to a circle.
pub fn clean_shape(value: &str) -> Shape {
    match value.trim() {
        "rounded" => Shape::Rounded,
        "square" => Shape::Square,
        _ => Shape::Circle,
    }
}

/// Accepts a photo size from a form, falling back to medium.
pub fn clean_size(value: &str) -> Size {
    match value.trim() {
        "sm" => Size::Small,
        "lg" => Size::Large,
        _ => Size::Medium,
    }
}

/// Read the six choices out of a parsed form body.
pub fn from_form(body: &crate::uploads::ParsedForm) -> DisplayOptions {
    DisplayOptions {
        layout: clean_layout(body.field("layout")),
        columns: clean_columns(body.field("grid_columns")),
        image_align: clean_align(body.field("image_align")),
        text_align: clean_align(body.field("text_align")),
        photo_shape: clean_shape(body.field("photo_shape")),
        photo_size: clean_size(body.field("photo_size")),
    }
}

// ---------- The site's defaults ----------

/// The home page block that lists the university rank holders. Its display
/// choices drive both the block on the home page and the full
/// `/academics/rank-holders` page, so an admin sets the look in one place.
pub const RANK_HOLDERS_KEY: &str = "rank_holders";

/// The six display columns, in the order [`DisplayOptions::from_row`] wants.
pub const COLUMNS: &str =
    "layout, grid_columns, image_align, text_align, photo_shape, photo_size";

/// Read one `layout, columns, ...` row into [`DisplayOptions`].
async fn read_opts(db: &PgPool, sql: &str) -> Res<Option<DisplayOptions>> {
    let row: Option<(String, i32, String, String, String, String)> =
        sqlx::query_as(sql).fetch_optional(db).await?;
    Ok(row.map(|(l, c, ia, ta, ps, pz)| {
        DisplayOptions::from_row(&l, c, &ia, &ta, &ps, &pz)
    }))
}

/// The site's default choices, the ones a new section starts from.
///
/// `section_display_defaults` holds exactly one row, but it is read optionally
/// anyway: a database that somehow lost the row falls back to
/// [`DisplayOptions::starter`] rather than leaving every new section with
/// nothing to copy.
pub async fn defaults(db: &PgPool) -> Res<DisplayOptions> {
    Ok(read_opts(
        db,
        &format!("SELECT {COLUMNS} FROM section_display_defaults WHERE id = 1"),
    )
    .await?
    .unwrap_or_else(DisplayOptions::starter))
}

/// Write the site's default choices.
pub async fn save_defaults(db: &PgPool, opts: &DisplayOptions) -> Res<()> {
    let (layout, columns, image_align, text_align, photo_shape, photo_size) = opts.to_row();
    sqlx::query(
        r#"INSERT INTO section_display_defaults
             (id, layout, grid_columns, image_align, text_align, photo_shape, photo_size)
           VALUES (1, $1, $2, $3, $4, $5, $6)
           ON CONFLICT (id) DO UPDATE SET
               layout = EXCLUDED.layout,
               grid_columns = EXCLUDED.grid_columns,
               image_align = EXCLUDED.image_align,
               text_align = EXCLUDED.text_align,
               photo_shape = EXCLUDED.photo_shape,
               photo_size = EXCLUDED.photo_size"#,
    )
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

/// The rank holders list's display choices, falling back to the site defaults
/// when the home section has no row of its own.
pub async fn rank_holders_options(db: &PgPool) -> Res<DisplayOptions> {
    let row: Option<(String, i32, String, String, String, String)> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM home_sections WHERE section_key = $1"
    ))
    .bind(RANK_HOLDERS_KEY)
    .fetch_optional(db)
    .await?;

    if let Some((l, c, ia, ta, ps, pz)) = row {
        return Ok(DisplayOptions::from_row(&l, c, &ia, &ta, &ps, &pz));
    }
    defaults(db).await
}