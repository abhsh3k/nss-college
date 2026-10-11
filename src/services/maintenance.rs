//! Scheduled housekeeping: expired sessions, staged import batches and the
//! sensitive text they hold, plus the recurring backup.
//!
//! Every step is idempotent and safe to run concurrently with normal traffic.
//! A failure in one step is logged and does not stop the others, so a missing
//! `pg_dump` cannot silently stop session cleanup.
//!
//! Two things are deliberately *not* deleted: committed result-import batches
//! (they are the record of what was filed) and anything referenced by academic
//! history. Only preview state is expired, and its raw payload — the uploaded
//! text — is dropped, because that is the part that carries student data.

use std::time::Duration;

use sqlx::PgPool;

use super::backup::{self, BackupConfig};

pub type Res<T> = Result<T, sqlx::Error>;

/// What one housekeeping pass did, for logging and for the `maintenance` CLI.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PurgeReport {
    pub sessions: u64,
    pub stale_student_batches: u64,
    pub expired_result_imports: u64,
    pub old_result_imports: u64,
}

/// Remove expired session rows. `tower-sessions` leaves them behind: an
/// eight-hour inactivity expiry stops a cookie being accepted but does not
/// delete the row.
///
/// The store keeps its table in its own `tower_sessions` schema, which is not
/// on the connection's `search_path`, so the table has to be named in full.
pub async fn purge_expired_sessions(db: &PgPool) -> Res<u64> {
    Ok(
        sqlx::query("DELETE FROM tower_sessions.session WHERE expiry_date < now()")
            .execute(db)
            .await?
            .rows_affected(),
    )
}

/// Run the existing student-import purge function, which a fresh import
/// otherwise only triggers as a side effect.
pub async fn purge_stale_student_batches(db: &PgPool) -> Res<u64> {
    Ok(sqlx::query("SELECT purge_stale_import_batches()")
        .execute(db)
        .await?
        .rows_affected())
}

/// Expire unconfirmed result imports past their window and drop the raw
/// payload they were staged from. The batch row stays for the audit trail.
pub async fn expire_result_imports(db: &PgPool) -> Res<u64> {
    Ok(sqlx::query(
        r#"UPDATE result_import_batches
              SET state = 'expired', payload = ''
            WHERE state = 'preview' AND expires_at < now()"#,
    )
    .execute(db)
    .await?
    .rows_affected())
}

/// Delete long-dead import batches (and, by cascade, their staging rows and any
/// generated temporary passwords).
pub async fn purge_old_result_imports(db: &PgPool) -> Res<u64> {
    Ok(sqlx::query(
        r#"DELETE FROM result_import_batches
            WHERE state IN ('expired', 'cancelled', 'committed')
              AND created_at < now() - interval '90 days'"#,
    )
    .execute(db)
    .await?
    .rows_affected())
}

/// One full housekeeping pass. Each step's failure is reported separately in
/// the log so one broken statement cannot hide the others.
pub async fn purge(db: &PgPool) -> PurgeReport {
    let mut report = PurgeReport::default();
    match purge_expired_sessions(db).await {
        Ok(n) => report.sessions = n,
        Err(e) => tracing::error!(error = ?e, "maintenance: expired-session purge failed"),
    }
    match purge_stale_student_batches(db).await {
        Ok(n) => report.stale_student_batches = n,
        Err(e) => tracing::error!(error = ?e, "maintenance: student import purge failed"),
    }
    match expire_result_imports(db).await {
        Ok(n) => report.expired_result_imports = n,
        Err(e) => tracing::error!(error = ?e, "maintenance: result-import expiry failed"),
    }
    match purge_old_result_imports(db).await {
        Ok(n) => report.old_result_imports = n,
        Err(e) => tracing::error!(error = ?e, "maintenance: result-import purge failed"),
    }
    report
}

/// Start the background housekeeping task.
///
/// Runs one pass immediately (so a container that restarts often still cleans
/// up), then hourly. The backup is checked on the same tick and only runs when
/// the last recorded backup is older than the configured interval, so the
/// schedule is driven by what actually happened rather than by uptime.
pub fn spawn(db: PgPool, backup_cfg: BackupConfig) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60 * 60));
        loop {
            tick.tick().await;
            let report = purge(&db).await;
            if report != PurgeReport::default() {
                tracing::info!(
                    sessions = report.sessions,
                    student_batches = report.stale_student_batches,
                    expired_imports = report.expired_result_imports,
                    old_imports = report.old_result_imports,
                    "maintenance: purge complete"
                );
            }
            if !backup_cfg.enabled {
                continue;
            }
            match backup::due_for_backup(&db, backup_cfg.interval_hours).await {
                Ok(true) => match backup::run(&db, &backup_cfg).await {
                    Ok(report) => tracing::info!(path = %report.path.display(), "maintenance: backup written"),
                    Err(e) => tracing::error!(error = %e, "maintenance: backup failed"),
                },
                Ok(false) => {}
                Err(e) => tracing::error!(error = ?e, "maintenance: could not tell whether a backup is due"),
            }
        }
    });
}
