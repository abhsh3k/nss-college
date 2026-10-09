use askama::Template;
use axum::extract::State;

use crate::{
    layout::DisplayOptions,
    models::SiteInfo,
    routes::public::page_copy,
    services::faculty,
    state::AppState,
};

#[derive(Template)]
#[template(path = "public/about_staff.html")]
struct AboutStaffTemplate {
    site: SiteInfo,
    title: String,
    lede: String,
    groups: Vec<faculty::DepartmentGroup>,
    display: DisplayOptions,
}

pub async fn page(State(s): State<AppState>) -> Result<axum::response::Html<String>, crate::error::AppError> {
    let db = &s.db;

    let (site, groups, display) = tokio::try_join!(
        crate::site::get(db),
        faculty::faculty_groups(db),
        crate::layout::defaults(db),
    )?;

    let copy = page_copy(db, "/about/staff", "Teaching staff").await?;

    let groups: Vec<faculty::DepartmentGroup> = groups
        .into_iter()
        .map(|g| faculty::DepartmentGroup {
            slug: g.slug,
            name: g.name,
            cards: g
                .cards
                .into_iter()
                .map(|mut c| {
                    let initials = faculty::initials(&c);
                    c.initials = Some(initials);
                    c
                })
                .collect(),
        })
        .collect();

    let html = AboutStaffTemplate {
        site,
        title: copy.title,
        lede: copy.lede,
        groups,
        display,
    }
    .render()?;

    Ok(axum::response::Html(html))
}
