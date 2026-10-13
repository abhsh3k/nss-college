use askama::Template;
use axum::extract::State;

use crate::{
    layout::DisplayOptions,
    models::SiteInfo,
    routes::public::page_copy,
    services::{faculty, profiles},
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

    // The Principal and office staff keep their own profiles (they have no
    // `faculty` row); they are shown above and below the teaching departments
    // when they have filled one in.
    let (site, faculty_groups, principal, office_staff, display) = tokio::try_join!(
        crate::site::get(db),
        faculty::faculty_groups(db),
        profiles::group_for_role(db, "principal", "principal", "Principal"),
        profiles::group_for_role(db, "staff", "office-staff", "Office staff"),
        crate::layout::defaults(db),
    )?;

    let copy = page_copy(db, "/about/staff", "Teaching staff").await?;

    let mut ordered = Vec::new();
    if let Some(group) = principal {
        ordered.push(group);
    }
    ordered.extend(faculty_groups);
    if let Some(group) = office_staff {
        ordered.push(group);
    }

    let groups: Vec<faculty::DepartmentGroup> = ordered
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
