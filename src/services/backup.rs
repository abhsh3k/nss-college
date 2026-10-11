//! Automated, verifiable backups of the two things that cannot be regenerated:
//! the PostgreSQL database and the private upload directory.
//!
//! Design decisions worth knowing:
//!
//! * **Never under `static/` or `uploads/`.** Those trees are served (uploads
//!   behind authorization, static publicly), so a backup written into them
//!   would be downloadable. `run` refuses such a directory outright.
//! * **Dated directories, newest kept.** Each run writes `backup-<UTC stamp>/`
//!   containing `database.dump`, `uploads.tar.gz` and `manifest.json`.
//!   Retention keeps the newest `BACKUP_KEEP` sets.
//! * **A manifestation of failure is loud.** A failed dump returns an error,
//!   is logged, and does not write a half-set. Only the uploads archive is
//!   best-effort, and the manifest records whether it was produced.
//! * **The dump tool is explicit.** `pg_dump` on `PATH` is preferred; in a
//!   container deployment where only the database container has it, set
//!   `BACKUP_DOCKER_SERVICE` (e.g. `db`) and the dump is taken with
//!   `docker compose exec`.
//!
//! Restore is deliberately not automated: it overwrites live data. The README
//! documents the verified `pg_restore`/`tar` commands.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use sqlx::PgPool;

type Res<T> = Result<T, sqlx::Error>;

#[derive(Clone, Debug)]
pub struct BackupConfig {
    pub dir: PathBuf,
    pub keep: usize,
    pub interval_hours: u64,
    pub enabled: bool,
    pub database_url: String,
    pub upload_dir: String,
    /// When set (e.g. `db`), dump through `docker compose exec <service>`.
    pub docker_service: Option<String>,
}

impl BackupConfig {
    pub fn from_env() -> Self {
        let dir = std::env::var("BACKUP_DIR").unwrap_or_else(|_| "backups".into());
        Self {
            dir: PathBuf::from(dir),
            keep: env_number("BACKUP_KEEP", 7).max(1) as usize,
            interval_hours: env_number("BACKUP_INTERVAL_HOURS", 24).max(1),
            enabled: std::env::var("BACKUP_ENABLED")
                .map(|v| !matches!(v.to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"))
                .unwrap_or(true),
            database_url: std::env::var("DATABASE_URL").unwrap_or_default(),
            upload_dir: std::env::var("UPLOAD_DIR").unwrap_or_else(|_| "uploads".into()),
            docker_service: std::env::var("BACKUP_DOCKER_SERVICE").ok().filter(|s| !s.is_empty()),
        }
    }
}

fn env_number(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// What one backup run produced.
#[derive(Debug)]
pub struct BackupReport {
    pub path: PathBuf,
    pub database_bytes: u64,
    pub uploads_bytes: Option<u64>,
    pub removed_old: usize,
}

/// Whether a backup is due: the last recorded one is missing or older than the
/// interval. Read from the audit trail rather than memory, so a restart does
/// not re-run a backup that just happened.
pub async fn due_for_backup(db: &PgPool, interval_hours: u64) -> Res<bool> {
    let last: Option<f64> = sqlx::query_scalar(
        r#"SELECT EXTRACT(EPOCH FROM max(created_at))::float8
             FROM audit_log WHERE action = 'backup_completed'"#,
    )
    .fetch_one(db)
    .await?;
    Ok(match last {
        None => true,
        Some(secs) => chrono_free::now_secs() - secs as i64 >= (interval_hours as i64) * 3600,
    })
}

/// Take one backup and enforce retention.
pub async fn run(db: &PgPool, cfg: &BackupConfig) -> Result<BackupReport, String> {
    assert_safe_dir(&cfg.dir, &cfg.upload_dir)?;
    std::fs::create_dir_all(&cfg.dir).map_err(|e| format!("could not create {}: {e}", cfg.dir.display()))?;
    restrict_permissions(&cfg.dir);

    let stamp = chrono_free::stamp();
    let run_dir = cfg.dir.join(format!("backup-{stamp}"));
    std::fs::create_dir_all(&run_dir).map_err(|e| format!("could not create {}: {e}", run_dir.display()))?;
    restrict_permissions(&run_dir);

    let dump_path = run_dir.join("database.dump");
    let tool = dump_database(cfg, &dump_path).await?;
    let database_bytes = std::fs::metadata(&dump_path).map(|m| m.len()).unwrap_or(0);
    if database_bytes == 0 {
        // An empty file means the dump failed quietly; remove it and report.
        let _ = std::fs::remove_dir_all(&run_dir);
        return Err("the database dump was empty; nothing was kept".into());
    }

    // Uploads are best-effort: a dump of the database is still worth keeping,
    // and the manifest says exactly what is missing.
    let (uploads_bytes, uploads_note) = archive_uploads(cfg, &run_dir).await;

    let manifest = serde_json::json!({
        "created_at": chrono_free::iso(),
        "tool": tool,
        "database": {
            "file": "database.dump",
            "bytes": database_bytes,
            "sha256": sha256_of(&dump_path),
        },
        "uploads": match uploads_bytes {
            Some(bytes) => serde_json::json!({
                "file": "uploads.tar.gz",
                "bytes": bytes,
                "sha256": sha256_of(&run_dir.join("uploads.tar.gz")),
                "directory": cfg.upload_dir,
            }),
            None => serde_json::json!({ "skipped": uploads_note }),
        },
    });
    let manifest_path = run_dir.join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).map_err(|e| format!("could not encode the manifest: {e}"))?,
    )
    .map_err(|e| format!("could not write {}: {e}", manifest_path.display()))?;

    let removed_old = enforce_retention(&cfg.dir, cfg.keep).await;

    // Record it so `due_for_backup` and operators can both see when it last ran.
    super::users::audit(db, None, "backup_completed", "backup", None)
        .await
        .map_err(|e| format!("backup written to {} but the audit row failed: {e}", run_dir.display()))?;

    Ok(BackupReport {
        path: run_dir,
        database_bytes,
        uploads_bytes,
        removed_old,
    })
}

/// Refuse a backup directory that a web request could read.
fn assert_safe_dir(dir: &Path, upload_dir: &str) -> Result<(), String> {
    let dir = absolute(dir);
    for forbidden in [PathBuf::from("static"), absolute(Path::new(upload_dir))] {
        let forbidden = if forbidden.is_absolute() { forbidden } else { absolute(&forbidden) };
        if dir.starts_with(&forbidden) {
            return Err(format!(
                "BACKUP_DIR {} is inside a served directory ({}); choose a path outside static/ and the upload directory",
                dir.display(),
                forbidden.display()
            ));
        }
    }
    Ok(())
}

fn absolute(p: &Path) -> PathBuf {
    match std::fs::canonicalize(p) {
        Ok(p) => p,
        Err(_) => match (std::env::current_dir(), p.is_absolute()) {
            (Ok(cwd), false) => cwd.join(p),
            (_, _) => p.to_path_buf(),
        },
    }
}

fn restrict_permissions(p: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    let _ = p;
}

/// Dump the database with `pg_dump`, directly or through a container.
async fn dump_database(cfg: &BackupConfig, target: &Path) -> Result<String, String> {
    if which("pg_dump").is_some() {
        if cfg.database_url.is_empty() {
            return Err("DATABASE_URL is not set, so there is nothing to dump".into());
        }
        let out = tokio::process::Command::new("pg_dump")
            .arg("--format=custom")
            .arg("--no-owner")
            .arg("--no-privileges")
            .arg("--dbname")
            .arg(&cfg.database_url)
            .arg("--file")
            .arg(target)
            .output()
            .await
            .map_err(|e| format!("could not run pg_dump: {e}"))?;
        if !out.status.success() {
            let _ = std::fs::remove_file(target);
            return Err(format!(
                "pg_dump failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        return Ok("pg_dump".into());
    }

    if let Some(service) = &cfg.docker_service {
        let parts = parse_db_url(&cfg.database_url)
            .ok_or("DATABASE_URL could not be parsed for the container dump")?;
        let file = target.to_path_buf();
        // `Command::output()` would replace the stdout redirect with a pipe and
        // leave the file empty, so the child is spawned with the file as its
        // stdout and only stderr captured, then waited on.
        let child = tokio::process::Command::new("docker")
            .args(["compose", "exec", "-T", service, "pg_dump", "--format=custom", "--no-owner", "--no-privileges", "-U"])
            .arg(&parts.user)
            .arg("-d")
            .arg(&parts.database)
            .stdout(Stdio::from(
                std::fs::File::create(&file).map_err(|e| format!("could not create {}: {e}", file.display()))?,
            ))
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not run docker compose exec: {e}"))?;
        let out = child
            .wait_with_output()
            .await
            .map_err(|e| format!("could not wait for the container pg_dump: {e}"))?;
        if !out.status.success() {
            let _ = std::fs::remove_file(target);
            return Err(format!(
                "container pg_dump failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        return Ok(format!("docker compose exec {service} pg_dump"));
    }

    Err("no pg_dump found on PATH. Install postgresql-client, or set \
         BACKUP_DOCKER_SERVICE to the compose service running PostgreSQL."
        .into())
}

/// Archive the upload directory. `tar` is used (present on every deployment
/// target for this app) rather than a Rust archive crate, so the result is the
/// ordinary file an operator expects. Returns the byte size, or a note saying
/// why it was skipped.
async fn archive_uploads(cfg: &BackupConfig, run_dir: &Path) -> (Option<u64>, &'static str) {
    let src = PathBuf::from(&cfg.upload_dir);
    if !src.is_dir() {
        return (None, "the upload directory does not exist");
    }
    if which("tar").is_none() {
        return (None, "tar is not installed");
    }
    let archive = run_dir.join("uploads.tar.gz");
    let parent = src.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
    let name = src.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    let parent = if parent.as_os_str().is_empty() { PathBuf::from(".") } else { parent };
    let out = tokio::process::Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(&parent)
        .arg(&name)
        .output()
        .await;
    match out {
        Ok(o) if o.status.success() => (
            std::fs::metadata(&archive).ok().map(|m| m.len()),
            "",
        ),
        Ok(o) => {
            tracing::error!(error = %String::from_utf8_lossy(&o.stderr).trim(), "backup: uploads archive failed");
            let _ = std::fs::remove_file(&archive);
            (None, "the uploads archive failed")
        }
        Err(e) => {
            tracing::error!(error = %e, "backup: could not run tar");
            (None, "tar could not be run")
        }
    }
}

/// Keep the newest `keep` backup sets; delete the rest. Returns how many were
/// removed. A failure to delete is logged but does not fail the run: the new
/// backup is already complete.
async fn enforce_retention(dir: &Path, keep: usize) -> usize {
    let mut sets: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with("backup-"))
                        .unwrap_or(false)
            })
            .collect(),
        Err(e) => {
            tracing::error!(error = ?e, "backup: could not list the backup directory");
            return 0;
        }
    };
    // The stamp sorts lexicographically, so the names are the timestamps.
    sets.sort();
    let mut removed = 0;
    let excess = sets.len().saturating_sub(keep);
    for old in sets.into_iter().take(excess) {
        match std::fs::remove_dir_all(&old) {
            Ok(()) => removed += 1,
            Err(e) => tracing::error!(error = ?e, path = %old.display(), "backup: could not remove an old set"),
        }
    }
    removed
}

/// A readable listing of the backup sets on disk, newest first, for the CLI.
pub fn list_sets(dir: &Path) -> Vec<PathBuf> {
    let mut sets: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.is_dir()
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.starts_with("backup-"))
                            .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    sets.sort();
    sets.reverse();
    sets
}

fn which(cmd: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|p| p.join(cmd))
        .find(|p| p.is_file())
}

/// The `user` and database name out of a `postgres://user:pass@host:port/db`
/// URL, for the container dump.
struct DbParts {
    user: String,
    database: String,
}

fn parse_db_url(url: &str) -> Option<DbParts> {
    let rest = url.split("://").nth(1)?;
    let (userinfo, host_and_path) = rest.rsplit_once('@')?;
    let user = userinfo.split(':').next()?.to_string();
    let path = host_and_path.split('/').nth(1)?;
    let database = path.split('?').next()?.to_string();
    if user.is_empty() || database.is_empty() {
        return None;
    }
    Some(DbParts { user, database })
}

fn sha256_of(path: &Path) -> Option<String> {
    which("sha256sum")?;
    let out = std::process::Command::new("sha256sum").arg(path).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).split_whitespace().next().map(str::to_string)
}

/// Date and time helpers. The project has no date crate, and the backup only
/// needs UTC seconds, so this keeps a dependency out of the build.
mod chrono_free {
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn now_secs() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// `YYYYMMDD-HHMMSS` in UTC. Lexicographic order is chronological order,
    /// which is what retention and `list_sets` rely on.
    pub fn stamp() -> String {
        let (y, mo, d, h, mi, s) = civil(now_secs());
        format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
    }

    pub fn iso() -> String {
        let (y, mo, d, h, mi, s) = civil(now_secs());
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
    }

    /// Days-since-epoch to a civil date (Howard Hinnant's algorithm), for UTC.
    fn civil(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
        let days = secs.div_euclid(86_400);
        let rem = secs.rem_euclid(86_400);
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let y = if m <= 2 { y + 1 } else { y };
        (
            y,
            m,
            d,
            (rem / 3600) as u32,
            ((rem % 3600) / 60) as u32,
            (rem % 60) as u32,
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn epoch_formats_correctly() {
            assert_eq!(stamp_for(0), "19700101-000000");
            assert_eq!(iso_for(0), "1970-01-01T00:00:00Z");
        }

        #[test]
        fn a_leap_day_is_right() {
            // 2024-02-29T12:34:56Z
            assert_eq!(iso_for(1_709_210_096), "2024-02-29T12:34:56Z");
            // 2026-10-10T00:00:00Z
            assert_eq!(iso_for(1_791_590_400), "2026-10-10T00:00:00Z");
        }

        fn stamp_for(secs: i64) -> String {
            let (y, mo, d, h, mi, s) = civil(secs);
            format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
        }

        fn iso_for(secs: i64) -> String {
            let (y, mo, d, h, mi, s) = civil(secs);
            format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
        }
    }
}
