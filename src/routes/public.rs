use askama::Template;
use axum::{
    extract::{Path, State},
    http::Uri,
};

use crate::{
    error::AppError,
    models::{
        ContactInfo, Department, Facility, Milestone, NewsItem, Notice, PageSection, Programme,
        RankHolder, Unit,
    },
    services::content as svc,
    state::AppState,
};

pub async fn health() -> &'static str {
    "ok"
}

// ---------- Home ----------

#[derive(Template)]
#[template(path = "public/home.html")]
pub struct HomeTemplate {
    notices: Vec<Notice>,
    news: Vec<NewsItem>,
    programmes: Vec<Programme>,
    rank_holders: Vec<RankHolder>,
    units: Vec<Unit>,
    facilities: Vec<Facility>,
    milestones: Vec<Milestone>,
    contact: ContactInfo,
}

pub async fn home(State(s): State<AppState>) -> Result<HomeTemplate, AppError> {
    let db = &s.db;
    let (notices, news, programmes, rank_holders, units, facilities, milestones, contact) =
        tokio::try_join!(
            svc::notices(db, 5),
            svc::latest_news(db, 3),
            svc::programmes(db),
            svc::rank_holders(db, 3),
            svc::featured_units(db),
            svc::facilities(db),
            svc::milestones(db),
            svc::contact_info(db),
        )?;
    Ok(HomeTemplate {
        notices,
        news,
        programmes,
        rank_holders,
        units,
        facilities,
        milestones,
        contact,
    })
}

// ---------- Programmes ----------

#[derive(Template)]
#[template(path = "public/programmes.html")]
pub struct ProgrammesTemplate {
    title: &'static str,
    lede: &'static str,
    programmes: Vec<Programme>,
}

pub async fn programmes(State(s): State<AppState>) -> Result<ProgrammesTemplate, AppError> {
    Ok(ProgrammesTemplate {
        title: "Degree programmes",
        lede: "All programmes are affiliated to Mahatma Gandhi University, Kottayam. Undergraduate programmes follow the four-year honours structure.",
        programmes: svc::programmes(&s.db).await?,
    })
}

#[derive(Template)]
#[template(path = "public/programme.html")]
pub struct ProgrammeTemplate {
    title: String,
    lede: String,
    programme: Programme,
    related: Vec<Programme>,
}

pub async fn programme(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> Result<ProgrammeTemplate, AppError> {
    let programme = svc::programme_by_slug(&s.db, &slug)
        .await?
        .ok_or(AppError::NotFound)?;
    let related = svc::programmes_in_department(&s.db, &programme.department_slug)
        .await?
        .into_iter()
        .filter(|p| p.slug != programme.slug)
        .collect();
    Ok(ProgrammeTemplate {
        title: programme.name.clone(),
        lede: programme.summary.clone(),
        programme,
        related,
    })
}

// ---------- Departments ----------

#[derive(Template)]
#[template(path = "public/departments.html")]
pub struct DepartmentsTemplate {
    title: &'static str,
    lede: &'static str,
    departments: Vec<Department>,
}

pub async fn departments(State(s): State<AppState>) -> Result<DepartmentsTemplate, AppError> {
    Ok(DepartmentsTemplate {
        title: "Departments",
        lede: "Four teaching departments, with research in electronics and computer applications.",
        departments: svc::departments(&s.db).await?,
    })
}

#[derive(Template)]
#[template(path = "public/department.html")]
pub struct DepartmentTemplate {
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
    let programmes = svc::programmes_in_department(&s.db, &dept.slug).await?;
    Ok(DepartmentTemplate {
        title: dept.name,
        lede: dept.summary,
        programmes,
    })
}

// ---------- News and notices ----------

#[derive(Template)]
#[template(path = "public/news_list.html")]
pub struct NewsListTemplate {
    title: &'static str,
    lede: &'static str,
    news: Vec<NewsItem>,
}

pub async fn news_list(State(s): State<AppState>) -> Result<NewsListTemplate, AppError> {
    Ok(NewsListTemplate {
        title: "News and events",
        lede: "Reports from campus: celebrations, achievements and programmes.",
        news: svc::latest_news(&s.db, 50).await?,
    })
}

#[derive(Template)]
#[template(path = "public/news_item.html")]
pub struct NewsItemTemplate {
    title: String,
    lede: String,
    item: NewsItem,
}

pub async fn news_item(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<NewsItemTemplate, AppError> {
    let id: i64 = id.parse().map_err(|_| AppError::NotFound)?;
    let item = svc::news_by_id(&s.db, id).await?.ok_or(AppError::NotFound)?;
    Ok(NewsItemTemplate {
        title: item.title.clone(),
        lede: item.date.clone(),
        item,
    })
}

#[derive(Template)]
#[template(path = "public/notices.html")]
pub struct NoticesTemplate {
    title: &'static str,
    lede: &'static str,
    notices: Vec<Notice>,
}

pub async fn notices(State(s): State<AppState>) -> Result<NoticesTemplate, AppError> {
    Ok(NoticesTemplate {
        title: "Notices",
        lede: "Official notices for students, parents and applicants. This list updates while you read.",
        notices: svc::notices(&s.db, 50).await?,
    })
}

// ---------- Rank holders ----------

#[derive(Template)]
#[template(path = "public/rank_holders.html")]
pub struct RankHoldersTemplate {
    title: &'static str,
    lede: &'static str,
    rank_holders: Vec<RankHolder>,
}

pub async fn rank_holders(State(s): State<AppState>) -> Result<RankHoldersTemplate, AppError> {
    Ok(RankHoldersTemplate {
        title: "University rank holders",
        lede: "Students who placed among the top ranks in Mahatma Gandhi University examinations.",
        rank_holders: svc::rank_holders(&s.db, 100).await?,
    })
}

// ---------- Contact ----------

#[derive(Template)]
#[template(path = "public/contact.html")]
pub struct ContactTemplate {
    title: &'static str,
    lede: &'static str,
    contact: ContactInfo,
}

pub async fn contact(State(s): State<AppState>) -> Result<ContactTemplate, AppError> {
    Ok(ContactTemplate {
        title: "Contact and location",
        lede: "The campus is on a hilltop near Kulapparachal, 2 km east of Rajakumari town, in Idukki district.",
        contact: svc::contact_info(&s.db).await?,
    })
}

// ---------- Informational pages and 404 ----------

#[derive(Template)]
#[template(path = "public/page.html")]
pub struct PageTemplate {
    title: String,
    lede: String,
    sections: Vec<PageSection>,
}

/// Fallback: any other path is looked up in the `pages` table, otherwise 404.
pub async fn page_or_404(State(s): State<AppState>, uri: Uri) -> Result<PageTemplate, AppError> {
    let path = uri.path().trim_end_matches('/');
    let page = svc::page_by_path(&s.db, path)
        .await?
        .ok_or(AppError::NotFound)?;
    let sections = svc::page_sections(&s.db, page.id).await?;
    Ok(PageTemplate {
        title: page.title,
        lede: page.lede,
        sections,
    })
}
