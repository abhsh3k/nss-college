use askama::Template;
use axum::{
    extract::{Path, State},
    http::Uri,
    response::{IntoResponse, Response},
};

use crate::{
    error::AppError,
    layout::{self, DisplayOptions},
    models::{Department, Facility, HomeCopy, Milestone, NewsItem, Notice, PageSection, Programme, RankHolder, SiteInfo, Unit},
    services::content as svc,
    site,
    state::AppState,
};

pub async fn health() -> &'static str {
    "ok"
}

/// A `pages` row used purely as the title and lede for a list page.
struct PageCopy {
    title: String,
    lede: String,
}

/// Fall back to the path segment, so a missing row still renders a sensible page.
async fn page_copy(
    db: &sqlx::PgPool,
    path: &str,
    fallback: &str,
) -> Result<PageCopy, sqlx::Error> {
    let page = svc::page_by_path(db, path).await?;
    let (title, lede) = match page {
        Some(p) => (p.title, p.lede),
        None => (fallback.to_string(), String::new()),
    };
    Ok(PageCopy { title, lede })
}

// ---------- Home ----------

#[derive(Template)]
#[template(path = "public/home.html")]
pub struct HomeTemplate {
    site: SiteInfo,
    copy: HomeCopy,
    notices: Vec<Notice>,
    news: Vec<NewsItem>,
    programmes: Vec<Programme>,
    holders: Vec<RankHolder>,
    /// How the rank holder block on the home page is presented. Read from the
    /// same row as the full page, so the two never disagree. Named `opts`
    /// because the shared partial that draws the block is also used by the
    /// full rank holders page.
    opts: DisplayOptions,
    units: Vec<Unit>,
    facilities: Vec<Facility>,
    milestones: Vec<Milestone>,
}

pub async fn home(State(s): State<AppState>) -> Result<HomeTemplate, AppError> {
    let db = &s.db;
    let (
        site,
        copy,
        notices,
        news,
        programmes,
        holders,
        opts,
        units,
        facilities,
        milestones,
    ) = tokio::try_join!(
        site::get(db),
        svc::home_copy(db),
        svc::notices(db, 5),
        svc::latest_news(db, 3),
        svc::programmes(db),
        svc::rank_holders(db, 3),
        layout::rank_holders_options(db),
        svc::featured_units(db),
        svc::facilities(db),
        svc::milestones(db),
    )?;
    Ok(HomeTemplate {
        site,
        copy,
        notices,
        news,
        programmes,
        holders,
        opts,
        units,
        facilities,
        milestones,
    })
}

// ---------- Programmes ----------

#[derive(Template)]
#[template(path = "public/programmes.html")]
pub struct ProgrammesTemplate {
    site: SiteInfo,
    title: String,
    lede: String,
    programmes: Vec<Programme>,
}

pub async fn programmes(State(s): State<AppState>) -> Result<ProgrammesTemplate, AppError> {
    let (site, copy, programmes) = tokio::try_join!(
        site::get(&s.db),
        page_copy(&s.db, "/academics", "Programmes"),
        svc::programmes(&s.db),
    )?;
    Ok(ProgrammesTemplate {
        site,
        title: copy.title,
        lede: copy.lede,
        programmes,
    })
}

#[derive(Template)]
#[template(path = "public/programme.html")]
pub struct ProgrammeTemplate {
    site: SiteInfo,
    title: String,
    lede: String,
    programme: Programme,
    related: Vec<Programme>,
}

pub async fn programme(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Response, AppError> {
    let Some(programme) = svc::programme_by_slug(&s.db, &slug).await? else {
        // `/academics/syllabus` and similar are informational pages, not programmes.
        let path = format!("/academics/{slug}");
        return Ok(page_for_path(&s, &path).await?.into_response());
    };
    let (site, related) = tokio::try_join!(
        site::get(&s.db),
        svc::programmes_in_department(&s.db, &programme.department_slug)
    )?;
    let related = related
        .into_iter()
        .filter(|p| p.slug != programme.slug)
        .collect();
    Ok(ProgrammeTemplate {
        site,
        title: programme.name.clone(),
        lede: programme.summary.clone(),
        programme,
        related,
    }
    .into_response())
}

// ---------- Departments ----------

#[derive(Template)]
#[template(path = "public/departments.html")]
pub struct DepartmentsTemplate {
    site: SiteInfo,
    title: String,
    lede: String,
    departments: Vec<Department>,
}

pub async fn departments(State(s): State<AppState>) -> Result<DepartmentsTemplate, AppError> {
    let (site, copy, departments) = tokio::try_join!(
        site::get(&s.db),
        page_copy(&s.db, "/departments", "Departments"),
        svc::departments(&s.db),
    )?;
    Ok(DepartmentsTemplate {
        site,
        title: copy.title,
        lede: copy.lede,
        departments,
    })
}

#[derive(Template)]
#[template(path = "public/department.html")]
pub struct DepartmentTemplate {
    site: SiteInfo,
    title: String,
    lede: String,
    programmes: Vec<Programme>,
}

pub async fn department(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> Result<DepartmentTemplate, AppError> {
    let dept = svc::department_by_slug(&s.db, &slug)
        .await?
        .ok_or(AppError::NotFound)?;
    let (site, programmes) = tokio::try_join!(
        site::get(&s.db),
        svc::programmes_in_department(&s.db, &dept.slug)
    )?;
    Ok(DepartmentTemplate {
        site,
        title: dept.name,
        lede: dept.summary,
        programmes,
    })
}

// ---------- News and notices ----------

#[derive(Template)]
#[template(path = "public/news_list.html")]
pub struct NewsListTemplate {
    site: SiteInfo,
    title: String,
    lede: String,
    news: Vec<NewsItem>,
}

pub async fn news_list(State(s): State<AppState>) -> Result<NewsListTemplate, AppError> {
    let (site, copy, news) = tokio::try_join!(
        site::get(&s.db),
        page_copy(&s.db, "/news", "News"),
        svc::latest_news(&s.db, 50),
    )?;
    Ok(NewsListTemplate {
        site,
        title: copy.title,
        lede: copy.lede,
        news,
    })
}

#[derive(Template)]
#[template(path = "public/news_item.html")]
pub struct NewsItemTemplate {
    site: SiteInfo,
    title: String,
    lede: String,
    item: NewsItem,
}

pub async fn news_item(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<NewsItemTemplate, AppError> {
    let id: i64 = id.parse().map_err(|_| AppError::NotFound)?;
    let (site, item) = tokio::try_join!(site::get(&s.db), svc::news_by_id(&s.db, id))?;
    let item = item.ok_or(AppError::NotFound)?;
    Ok(NewsItemTemplate {
        site,
        title: item.title.clone(),
        lede: item.date.clone(),
        item,
    })
}

#[derive(Template)]
#[template(path = "public/notices.html")]
pub struct NoticesTemplate {
    site: SiteInfo,
    title: String,
    lede: String,
    notices: Vec<Notice>,
}

pub async fn notices(State(s): State<AppState>) -> Result<NoticesTemplate, AppError> {
    let (site, copy, notices) = tokio::try_join!(
        site::get(&s.db),
        page_copy(&s.db, "/notices", "Notices"),
        svc::notices(&s.db, 50),
    )?;
    Ok(NoticesTemplate {
        site,
        title: copy.title,
        lede: copy.lede,
        notices,
    })
}

// ---------- Rank holders ----------

#[derive(Template)]
#[template(path = "public/rank_holders.html")]
pub struct RankHoldersTemplate {
    site: SiteInfo,
    title: String,
    lede: String,
    /// The people on the list. Named `holders` because the shared partial that
    /// draws them is also used by the home page, and both templates must hand it
    /// the same names.
    holders: Vec<RankHolder>,
    /// How the list is presented, chosen by the IT admin on the settings screen.
    opts: DisplayOptions,
}

pub async fn rank_holders(State(s): State<AppState>) -> Result<RankHoldersTemplate, AppError> {
    let (site, copy, holders, opts) = tokio::try_join!(
        site::get(&s.db),
        page_copy(&s.db, "/academics/rank-holders", "Rank holders"),
        svc::rank_holders(&s.db, 100),
        layout::rank_holders_options(&s.db),
    )?;
    Ok(RankHoldersTemplate {
        site,
        title: copy.title,
        lede: copy.lede,
        holders,
        opts,
    })
}

// ---------- Contact ----------

#[derive(Template)]
#[template(path = "public/contact.html")]
pub struct ContactTemplate {
    site: SiteInfo,
    title: String,
    lede: String,
}

pub async fn contact(State(s): State<AppState>) -> Result<ContactTemplate, AppError> {
    let site = site::get(&s.db).await?;
    Ok(ContactTemplate {
        title: site.contact_title.clone(),
        lede: site.contact_lede.clone(),
        site,
    })
}

// ---------- Informational pages and 404 ----------

#[derive(Template)]
#[template(path = "public/page.html")]
pub struct PageTemplate {
    site: SiteInfo,
    title: String,
    lede: String,
    sections: Vec<PageSection>,
    /// True when any section on the page is drawn as a card grid.
    ///
    /// A card grid chosen to be four across needs more room than the narrow
    /// column a page of prose reads well in, so the template widens the page for
    /// it. The `md:grid-cols-*` breakpoints inside the section partial keep the
    /// cards usable on a phone either way. Computed once here because an askama
    /// `{% if %}` cannot call a method taking `&self` on the whole list.
    wide: bool,
}

async fn page_for_path(s: &AppState, path: &str) -> Result<PageTemplate, AppError> {
    let (site, page) = tokio::try_join!(site::get(&s.db), svc::page_by_path(&s.db, path))?;
    let page = page.ok_or(AppError::NotFound)?;
    let sections = svc::page_sections(&s.db, page.id).await?;
    let wide = sections.iter().any(|s| s.opts.is_cards());
    Ok(PageTemplate {
        site,
        title: page.title,
        lede: page.lede,
        sections,
        wide,
    })
}

/// Fallback: any other path is looked up in the `pages` table, otherwise 404.
pub async fn page_or_404(State(s): State<AppState>, uri: Uri) -> Result<PageTemplate, AppError> {
    page_for_path(&s, uri.path().trim_end_matches('/')).await
}