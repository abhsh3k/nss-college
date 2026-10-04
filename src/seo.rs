use axum::{extract::State, http::header, response::IntoResponse};

use crate::{error::AppError, services::content, state::AppState};

fn site_url() -> String {
    std::env::var("SITE_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".into())
}

pub async fn robots() -> impl IntoResponse {
    let body = format!(
        "User-agent: *\nAllow: /\nDisallow: /hub\nDisallow: /admin\nDisallow: /teacher\nDisallow: /login\nDisallow: /account\nDisallow: /fragments/\n\nSitemap: {}/sitemap.xml\n",
        site_url()
    );
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body)
}

pub async fn sitemap(State(s): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let base = site_url();
    // The list pages are now `pages` rows too, so they arrive with the query
    // rather than being listed here; dedupe anyway in case one is added twice.
    let mut paths: Vec<String> = vec!["/".into()];
    paths.extend(
        content::sitemap_paths(&s.db)
            .await?
            .into_iter()
            .filter(|p| !p.starts_with("/hub") && !p.starts_with("/admin")),
    );
    paths.sort();
    paths.dedup();

    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for p in paths {
        xml.push_str(&format!("  <url><loc>{base}{p}</loc></url>\n"));
    }
    xml.push_str("</urlset>\n");
    Ok(([(header::CONTENT_TYPE, "application/xml; charset=utf-8")], xml))
}
