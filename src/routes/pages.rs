use askama::Template;
use axum::{routing::get, Router};

use crate::pages::{PageDef, Section, PAGES};

#[derive(Template)]
#[template(path = "public/page.html")]
pub struct PageTemplate {
    title: &'static str,
    lede: &'static str,
    sections: &'static [Section],
}

impl From<&'static PageDef> for PageTemplate {
    fn from(p: &'static PageDef) -> Self {
        Self {
            title: p.title,
            lede: p.lede,
            sections: p.sections,
        }
    }
}

/// Registers one GET route per entry in `PAGES`.
pub fn register(mut router: Router) -> Router {
    for page in PAGES.iter() {
        router = router.route(page.path, get(move || async move { PageTemplate::from(page) }));
    }
    router
}
