//! HOD appointments and delegations: the effective authority a faculty account
//! holds, read from `role_assignments` and `delegations` rather than the legacy
//! `faculty.is_hod` / `faculty.can_manage` booleans.
//!
//! The booleans are still written (the people form and the seeder use them) and
//! still drive the sidebar, but the authority decision itself is made here:
//!
//! * an **active** `hod` / `acting_hod` assignment, inside its effective dates,
//!   is the head of its department;
//! * an **active** delegation, inside its dates and granted by a still-active
//!   head assignment, carries the same departmental scope;
//! * a user who has appointment rows but none currently active is *denied* — an
//!   expired appointment must not silently fall back to the legacy flag;
//! * only a user with no appointment rows at all falls back to the legacy flag,
//!   which is the compatibility path for accounts created before the model.
//!
//! `sync_hod_from_flags` keeps the two in step whenever a writer touches the
//! legacy flags, so the compatibility fallback is rarely needed.

use sqlx::{FromRow, PgPool};

type Res<T> = Result<T, sqlx::Error>;

/// Resolve one authority dimension. `active` is the department of a currently
/// effective appointment; `has_any` says whether the user has appointment rows
/// at all; `legacy` is the department the old boolean would have granted.
///
/// Pure, so the expiry rule can be unit-tested without a database.
pub fn resolve(active: Option<i64>, has_any: bool, legacy: Option<i64>) -> Option<i64> {
    match active {
        Some(d) => Some(d),
        // Had an appointment, none effective now: deny, do not fall back.
        None if has_any => None,
        None => legacy,
    }
}

/// The department this user heads right now, from an active in-window head
/// assignment.
pub async fn active_head(db: &PgPool, user_id: i64) -> Res<Option<i64>> {
    sqlx::query_scalar(
        r#"SELECT department_id FROM role_assignments
            WHERE user_id = $1 AND role IN ('hod', 'acting_hod')
              AND department_id IS NOT NULL
              AND status = 'active'
              AND starts_at <= now()
              AND (ends_at IS NULL OR ends_at > now())
            ORDER BY starts_at DESC LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
}

/// Whether the user has any head appointment row, active or not.
pub async fn has_head_row(db: &PgPool, user_id: i64) -> Res<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM role_assignments WHERE user_id = $1 AND role IN ('hod', 'acting_hod'))",
    )
    .bind(user_id)
    .fetch_one(db)
    .await
}

/// The department carried by an active, in-window delegation from a still-active
/// head assignment.
pub async fn active_delegation(db: &PgPool, user_id: i64) -> Res<Option<i64>> {
    sqlx::query_scalar(
        r#"SELECT ra.department_id
             FROM delegations d
             JOIN role_assignments ra ON ra.id = d.granting_assignment_id
            WHERE d.delegate_user_id = $1
              AND d.status = 'active'
              AND d.starts_at <= now() AND d.ends_at > now()
              AND ra.role IN ('hod', 'acting_hod')
              AND ra.status = 'active'
              AND ra.starts_at <= now() AND (ra.ends_at IS NULL OR ra.ends_at > now())
            LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
}

pub async fn has_delegation_row(db: &PgPool, user_id: i64) -> Res<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM delegations WHERE delegate_user_id = $1)",
    )
    .bind(user_id)
    .fetch_one(db)
    .await
}

/// `(department_id, is_hod, can_manage)` for the user's faculty row, if any.
pub async fn faculty_flags(db: &PgPool, user_id: i64) -> Res<Option<(Option<i64>, bool, bool)>> {
    sqlx::query_as(
        "SELECT department_id, is_hod, can_manage FROM faculty WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
}

/// The department a signed-in user may manage, or `None` to deny. Combines the
/// head and delegate dimensions through [`resolve`].
pub async fn effective_department(db: &PgPool, user_id: i64) -> Res<Option<i64>> {
    let (head, any_head, delegation, any_delegation, flags) = tokio::try_join!(
        active_head(db, user_id),
        has_head_row(db, user_id),
        active_delegation(db, user_id),
        has_delegation_row(db, user_id),
        faculty_flags(db, user_id),
    )?;

    let (legacy_head, legacy_delegation) = match flags {
        Some((dept, is_hod, can_manage)) => (
            if is_hod { dept } else { None },
            if can_manage { dept } else { None },
        ),
        None => (None, None),
    };

    let head_dept = resolve(head, any_head, legacy_head);
    let delegation_dept = resolve(delegation, any_delegation, legacy_delegation);
    Ok(head_dept.or(delegation_dept))
}

/// Reconcile the user's active head assignment with their faculty flags.
///
/// Called by every writer of `faculty.is_hod` so the appointment table does not
/// drift again. A faculty who is a head gets an active assignment for their
/// department unless that department already has one; a faculty who is not a
/// head has any active assignment revoked.
pub async fn sync_hod_from_flags(db: &PgPool, user_id: i64) -> Res<()> {
    let flags: Option<(Option<i64>, bool, String)> =
        sqlx::query_as("SELECT department_id, is_hod, status FROM faculty WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(db)
            .await?;

    let head_department = match flags {
        Some((Some(dept), true, ref status)) if status == "published" => Some(dept),
        _ => None,
    };

    match head_department {
        Some(dept) => {
            sqlx::query(
                r#"INSERT INTO role_assignments
                       (user_id, role, department_id, status, starts_at, approved_at, reason)
                   SELECT $1, 'hod', $2, 'active', now(), now(), 'faculty flag sync'
                    WHERE NOT EXISTS (
                          SELECT 1 FROM role_assignments
                           WHERE user_id = $1 AND department_id = $2
                             AND role IN ('hod', 'acting_hod')
                             AND status = 'active' AND ends_at IS NULL)
                      AND NOT EXISTS (
                          SELECT 1 FROM role_assignments
                           WHERE department_id = $2
                             AND role IN ('hod', 'acting_hod')
                             AND status = 'active' AND ends_at IS NULL)"#,
            )
            .bind(user_id)
            .bind(dept)
            .execute(db)
            .await?;
        }
        None => {
            sqlx::query(
                r#"UPDATE role_assignments
                      SET status = 'revoked', ends_at = now(), updated_at = now()
                    WHERE user_id = $1 AND role IN ('hod', 'acting_hod')
                      AND status = 'active' AND ends_at IS NULL"#,
            )
            .bind(user_id)
            .execute(db)
            .await?;
        }
    }
    Ok(())
}

// ---------- The management screen ----------

#[derive(Debug, FromRow)]
pub struct HeadRow {
    pub assignment_id: i64,
    pub user_id: i64,
    pub name: String,
    pub department_id: i64,
    pub department: String,
    pub role: String,
    pub started_label: String,
    pub ends_label: String,
}

/// The active heads across the college, for the appointments screen.
pub async fn list_heads(db: &PgPool) -> Res<Vec<HeadRow>> {
    sqlx::query_as::<_, HeadRow>(
        r#"SELECT ra.id AS assignment_id, ra.user_id, u.full_name AS name,
                  ra.department_id, d.name AS department, ra.role,
                  to_char(ra.starts_at, 'YYYY-MM-DD') AS started_label,
                  CASE WHEN ra.ends_at IS NULL THEN 'No end date'
                       ELSE to_char(ra.ends_at, 'YYYY-MM-DD') END AS ends_label
             FROM role_assignments ra
             JOIN users u ON u.id = ra.user_id
             JOIN departments d ON d.id = ra.department_id
            WHERE ra.role IN ('hod', 'acting_hod')
              AND ra.status = 'active'
              AND ra.starts_at <= now()
              AND (ra.ends_at IS NULL OR ra.ends_at > now())
            ORDER BY d.sort_order, d.name, u.full_name"#,
    )
    .fetch_all(db)
    .await
}

#[derive(Debug, FromRow)]
pub struct DelegationRow {
    pub id: i64,
    pub name: String,
    pub department: String,
    pub permission: String,
    pub ends_label: String,
}

pub async fn list_delegations(db: &PgPool) -> Res<Vec<DelegationRow>> {
    sqlx::query_as::<_, DelegationRow>(
        r#"SELECT d.id, u.full_name AS name,
                  COALESCE(dept.name, '—') AS department, d.permission,
                  to_char(d.ends_at, 'YYYY-MM-DD') AS ends_label
             FROM delegations d
             JOIN users u ON u.id = d.delegate_user_id
             LEFT JOIN role_assignments ra ON ra.id = d.granting_assignment_id
             LEFT JOIN departments dept ON dept.id = ra.department_id
            WHERE d.status = 'active' AND d.ends_at > now()
            ORDER BY d.ends_at, u.full_name"#,
    )
    .fetch_all(db)
    .await
}

/// The faculty an IT admin could appoint, with the department they sit in.
#[derive(Debug, FromRow)]
pub struct FacultyOption {
    pub user_id: i64,
    pub name: String,
    pub department_id: Option<i64>,
    pub department: String,
}

pub async fn faculty_options(db: &PgPool) -> Res<Vec<FacultyOption>> {
    sqlx::query_as::<_, FacultyOption>(
        r#"SELECT f.user_id, f.name, f.department_id,
                  COALESCE(d.name, 'No department') AS department
             FROM faculty f
             LEFT JOIN departments d ON d.id = f.department_id
            WHERE f.status = 'published'
            ORDER BY d.sort_order, d.name, f.name"#,
    )
    .fetch_all(db)
    .await
}

/// Appoint `user_id` as the head of `department_id`. `ends_at` is an optional
/// `YYYY-MM-DD`; `NULL` means open-ended. Fails on the one-active-head-per-
/// department unique index if the department already has a head.
pub async fn grant_head(
    db: &PgPool,
    user_id: i64,
    department_id: i64,
    acting: bool,
    ends_at: Option<&str>,
    actor: i64,
    reason: &str,
) -> Res<i64> {
    let mut tx = db.begin().await?;
    let role = if acting { "acting_hod" } else { "hod" };
    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO role_assignments
               (user_id, role, department_id, status, starts_at, ends_at, approved_by, approved_at, reason)
           VALUES ($1, $2, $3, 'active', now(), NULLIF($4, '')::timestamptz, $5, now(), $6)
           RETURNING id"#,
    )
    .bind(user_id)
    .bind(role)
    .bind(department_id)
    .bind(ends_at.unwrap_or(""))
    .bind(actor)
    .bind(reason)
    .fetch_one(&mut *tx)
    .await?;
    // Keep the legacy flag in step so the sidebar matches the appointment.
    sqlx::query("UPDATE faculty SET is_hod = true WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(id)
}

/// Request a head appointment, left `pending` until the Principal approves it.
/// The account is created by the IT admin; this only records the academic
/// request, so a pending row grants nothing (`active_head` ignores it).
pub async fn request_head(
    db: &PgPool,
    user_id: i64,
    department_id: i64,
    acting: bool,
    ends_at: Option<&str>,
    actor: i64,
    reason: &str,
) -> Res<i64> {
    // `actor` is the IT admin who asked; it is recorded in the audit log by the
    // route, while `approved_by` is reserved for the Principal's decision.
    let _ = actor;
    let role = if acting { "acting_hod" } else { "hod" };
    sqlx::query_scalar(
        r#"INSERT INTO role_assignments
               (user_id, role, department_id, status, starts_at, ends_at, reason)
           VALUES ($1, $2, $3, 'pending', now(), NULLIF($4, '')::timestamptz, $5)
           RETURNING id"#,
    )
    .bind(user_id)
    .bind(role)
    .bind(department_id)
    .bind(ends_at.unwrap_or(""))
    .bind(reason)
    .fetch_one(db)
    .await
}

/// Approve a pending head appointment, making it active. Refuses when the
/// approver is the person being appointed (a Principal cannot approve their
/// own appointment) and when the department already has an active head.
pub async fn approve_head(db: &PgPool, assignment_id: i64, approver: i64) -> Res<Option<i64>> {
    let mut tx = db.begin().await?;
    let user_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT user_id FROM role_assignments
            WHERE id = $1 AND status = 'pending'"#,
    )
    .bind(assignment_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(user_id) = user_id else {
        return Ok(None);
    };
    if user_id == approver {
        return Err(sqlx::Error::Protocol(
            "a Principal cannot approve their own appointment".into(),
        ));
    }
    sqlx::query(
        r#"UPDATE role_assignments
              SET status = 'active', starts_at = now(), approved_by = $2,
                  approved_at = now(), updated_at = now()
            WHERE id = $1 AND status = 'pending'"#,
    )
    .bind(assignment_id)
    .bind(approver)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE faculty SET is_hod = true WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Some(user_id))
}

/// Reject a pending head appointment. It never becomes active.
pub async fn reject_head(db: &PgPool, assignment_id: i64, approver: i64) -> Res<bool> {
    let done = sqlx::query(
        r#"UPDATE role_assignments
              SET status = 'revoked', ends_at = now(), approved_by = $2,
                  approved_at = now(), updated_at = now()
            WHERE id = $1 AND status = 'pending'"#,
    )
    .bind(assignment_id)
    .bind(approver)
    .execute(db)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// One head appointment waiting for the Principal's decision.
#[derive(Debug, FromRow)]
pub struct PendingHeadRow {
    pub assignment_id: i64,
    pub name: String,
    pub department: String,
    pub role: String,
    pub requested_label: String,
}

/// How many head appointments are waiting for a decision. Used for the
/// pending-approval badge, so it stays a single indexed count rather than a
/// full list shipped to every page.
pub async fn count_pending_heads(db: &PgPool) -> Res<i64> {
    sqlx::query_scalar("SELECT count(*) FROM role_assignments WHERE role IN ('hod', 'acting_hod') AND status = 'pending'")
        .fetch_one(db)
        .await
}

/// The pending head appointments, oldest first.
pub async fn list_pending_heads(db: &PgPool) -> Res<Vec<PendingHeadRow>> {
    sqlx::query_as::<_, PendingHeadRow>(
        r#"SELECT ra.id AS assignment_id, u.full_name AS name, d.name AS department,
                  ra.role,
                  to_char(ra.starts_at, 'Dy DD Mon YYYY HH24:MI') AS requested_label
             FROM role_assignments ra
             JOIN users u ON u.id = ra.user_id
             JOIN departments d ON d.id = ra.department_id
            WHERE ra.role IN ('hod', 'acting_hod')
              AND ra.status = 'pending'
            ORDER BY ra.starts_at, ra.id"#,
    )
    .fetch_all(db)
    .await
}

/// End a head appointment and open a handover task for the successor.
pub async fn revoke_head(db: &PgPool, assignment_id: i64, actor: i64) -> Res<()> {
    let mut tx = db.begin().await?;
    let user_id: Option<i64> = sqlx::query_scalar(
        r#"UPDATE role_assignments
              SET status = 'revoked', ends_at = now(), updated_at = now()
            WHERE id = $1 AND status = 'active'
        RETURNING user_id"#,
    )
    .bind(assignment_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(user_id) = user_id {
        sqlx::query("UPDATE faculty SET is_hod = false WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        // The incoming head must review whatever was delegated and any pending
        // work; record that as an open handover task.
        sqlx::query(
            r#"INSERT INTO appointment_handover_tasks (assignment_id, entity, status, note)
               VALUES ($1, 'department', 'open', $2)"#,
        )
        .bind(assignment_id)
        .bind(format!("Handover requested by user {actor}"))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Delegate the department's management to `delegate_user_id` until `ends_at`,
/// granted from the department's current active head assignment.
pub async fn grant_delegation(
    db: &PgPool,
    delegate_user_id: i64,
    department_id: i64,
    permission: &str,
    ends_at: &str,
    actor: i64,
) -> Res<i64> {
    let mut tx = db.begin().await?;
    let granting: i64 = sqlx::query_scalar(
        r#"SELECT id FROM role_assignments
            WHERE department_id = $1 AND role IN ('hod', 'acting_hod')
              AND status = 'active' AND (ends_at IS NULL OR ends_at > now())
            ORDER BY starts_at DESC LIMIT 1"#,
    )
    .bind(department_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(sqlx::Error::RowNotFound)?;
    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO delegations
               (granting_assignment_id, delegate_user_id, permission, starts_at, ends_at, status, reason)
           VALUES ($1, $2, $3, now(), $4::timestamptz, 'active', $5)
           RETURNING id"#,
    )
    .bind(granting)
    .bind(delegate_user_id)
    .bind(permission)
    .bind(ends_at)
    .bind(format!("Granted by user {actor}"))
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("UPDATE faculty SET can_manage = true WHERE user_id = $1")
        .bind(delegate_user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn revoke_delegation(db: &PgPool, id: i64, actor: i64) -> Res<()> {
    let mut tx = db.begin().await?;
    let user_id: Option<i64> = sqlx::query_scalar(
        r#"UPDATE delegations
              SET status = 'revoked', reviewed_by = $2, reviewed_at = now(), updated_at = now()
            WHERE id = $1 AND status = 'active'
        RETURNING delegate_user_id"#,
    )
    .bind(id)
    .bind(actor)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(user_id) = user_id {
        // Only clear the flag if no other active delegation remains.
        sqlx::query(
            r#"UPDATE faculty SET can_manage = EXISTS (
                   SELECT 1 FROM delegations d
                    WHERE d.delegate_user_id = $1 AND d.status = 'active' AND d.ends_at > now())
                WHERE user_id = $1"#,
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::resolve;

    #[test]
    fn an_active_appointment_wins() {
        assert_eq!(resolve(Some(4), true, Some(9)), Some(4));
    }

    #[test]
    fn an_expired_appointment_denies_rather_than_falling_back() {
        // Has appointment rows, but none effective now: no legacy fallback.
        assert_eq!(resolve(None, true, Some(9)), None);
    }

    #[test]
    fn a_user_with_no_appointments_uses_the_legacy_flag() {
        assert_eq!(resolve(None, false, Some(9)), Some(9));
        assert_eq!(resolve(None, false, None), None);
    }
}
