//! The college's own facts (name, phone, address, footer, error copy), loaded
//! from `site_settings` and cached in memory.
//!
//! Templates get the struct through the `site` field. Public page handlers call
//! [`get`], which re-reads the row set so an edit shows up on the next page
//! view; the error pages and the signed-in shell have no database handle of
//! their own, so they read [`cached`], which is refreshed on a timer.

use std::sync::OnceLock;
use std::time::Duration;

use sqlx::PgPool;

use crate::models::SiteInfo;

/// How stale [`cached`] is allowed to get. Settings change rarely.
const REFRESH_EVERY: Duration = Duration::from_secs(300);

fn cell() -> &'static std::sync::RwLock<SiteInfo> {
    static CELL: OnceLock<std::sync::RwLock<SiteInfo>> = OnceLock::new();
    CELL.get_or_init(|| std::sync::RwLock::new(SiteInfo::default()))
}

/// The last successfully loaded settings, without touching the database.
pub fn cached() -> SiteInfo {
    cell().read().map(|s| s.clone()).unwrap_or_default()
}

fn store(info: SiteInfo) {
    if let Ok(mut slot) = cell().write() {
        *slot = info;
    }
}

/// Read the settings fresh, then update the cache for everyone else.
pub async fn get(db: &PgPool) -> Result<SiteInfo, sqlx::Error> {
    let info = super::services::content::site_info(db).await?;
    store(info.clone());
    Ok(info)
}

/// Fill the cache at boot and keep it warm, so error pages and the dashboard
/// shell have something to render even before the first public page view.
pub fn spawn_refresher(db: PgPool) {
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(REFRESH_EVERY);
        loop {
            timer.tick().await;
            if let Err(e) = get(&db).await {
                tracing::warn!(error = ?e, "could not refresh site settings");
            }
        }
    });
}