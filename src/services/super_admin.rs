//! Root authority: the account roles a Super Admin controls, and the emergency
//! powers the brief reserves for it — recovering a locked-out account and
//! intervening in an already-finalized academic record.
//!
//! The IT admin deliberately cannot reach any of this. It cannot promote an
//! account to `admin` or `super_admin` (the people form only ever writes
//! student, faculty and staff), and the last active Super Admin account can be
//! neither demoted nor deactivated, so the college cannot lock itself out of
//! role management. The bootstrap account is created from the CLI:
//!
//! ```text
//! cargo run -- create-user super_admin admin@college.local "Site Owner"
//! ```
//!
//! Every write here is called from a `SuperAdminOnly` route, which records an
//! audit row (with a structured reason where one was given).

use sqlx::{FromRow, PgPool};

type Res<T> = Result<T, sqlx::Error>;

/// The account roles this screen manages. `principal` is deliberately absent:
/// academic authority is granted by the Principal appointment below, not by
/// editing an account role.
pub const TECHNICAL_ROLES: &[&str] = &["super_admin", "admin"];

pub fn role_label(role: &str) -> &'static str {
    match role {
        "super_admin" => "Super Admin",
        "admin" => "IT administrator",
        "principal" => "Principal",
        "staff" => "Office staff",
        "faculty" => "Teacher",
        "student" => "Student",
        "alumni" => "Alumni",
        _ => "Unknown",
    }
}

// ---------- Accounts ----------

#[derive(Debug, FromRow)]
pub struct AccountRow {
    pub id: i64,
    pub full_name: String,
    pub email: String,
    pub role: String,
    pub role_label: String,
    pub is_active: bool,
    /// Locked out by failed sign-ins, not merely inactive.
    pub locked: bool,
    pub created_label: String,
}

/// Every account holding, or waiting on, a college-wide role: the Super Admins,
/// the IT admins and the current Principal. Departmental roles are managed on
/// the appointments screen instead.
pub async fn list_accounts(db: &PgPool) -> Res<Vec<AccountRow>> {
    sqlx::query_as::<_, AccountRow>(
        r#"SELECT u.id, u.full_name, u.email, u.role,
                  CASE u.role
                      WHEN 'super_admin' THEN 'Super Admin'
                      WHEN 'admin'       THEN 'IT administrator'
                      WHEN 'principal'   THEN 'Principal'
                      ELSE u.role
                  END AS role_label,
                  u.is_active,
                  (u.locked_until IS NOT NULL AND u.locked_until > now()) AS locked,
                  to_char(u.created_at, 'YYYY-MM-DD') AS created_label
             FROM users u
            WHERE u.role IN ('super_admin', 'admin', 'principal')
            ORDER BY CASE u.role
                         WHEN 'super_admin' THEN 0
                         WHEN 'admin'       THEN 1
                         ELSE 2
                     END, u.full_name"#,
    )
    .fetch_all(db)
    .await
}

/// How many active accounts hold root authority.
pub async fn active_super_admins(db: &PgPool) -> Res<i64> {
    sqlx::query_scalar("SELECT count(*) FROM users WHERE role = 'super_admin' AND is_active")
        .fetch_one(db)
        .await
}

/// Whether this account may be deactivated.
///
/// Deactivating the last active Super Admin would leave nobody able to manage
/// roles or recover an account, so it is refused; the caller shows the reason.
pub async fn deactivation_allowed(db: &PgPool, user_id: i64) -> Res<bool> {
    let role: Option<String> = sqlx::query_scalar("SELECT role FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(db)
        .await?;
    match role.as_deref() {
        // Not a super admin: nothing to protect.
        Some(r) if r != "super_admin" => Ok(true),
        None => Ok(true),
        Some(_) => {
            let remaining: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM users WHERE role = 'super_admin' AND is_active AND id <> $1",
            )
            .bind(user_id)
            .fetch_one(db)
            .await?;
            Ok(remaining > 0)
        }
    }
}

/// What a role change did, so the route can explain a refusal instead of
/// guessing from an error string.
#[derive(Debug, PartialEq, Eq)]
pub enum RoleChange {
    Done,
    NotFound,
    /// A role this screen does not manage (a departmental one, or a typo).
    UnknownRole,
    /// The account is the last active Super Admin.
    LastSuperAdmin,
    /// The account's authority comes from the Principal appointment, which must
    /// be ended on its own record instead.
    StillPrincipal,
}

/// Give an account one of the technical roles ([`TECHNICAL_ROLES`]). Bumping
/// `session_version` revokes any session the account already holds, so a role
/// change takes effect at the next request, not at the next sign-in.
pub async fn set_role(db: &PgPool, user_id: i64, role: &str) -> Res<RoleChange> {
    if !TECHNICAL_ROLES.contains(&role) {
        return Ok(RoleChange::UnknownRole);
    }
    let mut tx = db.begin().await?;
    let current: Option<String> = sqlx::query_scalar("SELECT role FROM users WHERE id = $1 FOR UPDATE")
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(current) = current else {
        return Ok(RoleChange::NotFound);
    };
    if current == role {
        return Ok(RoleChange::Done);
    }
    if current == "principal" {
        return Ok(RoleChange::StillPrincipal);
    }
    if current == "super_admin" {
        // Demoting the last one would leave no account able to manage roles.
        let remaining: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM users WHERE role = 'super_admin' AND is_active AND id <> $1",
        )
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        if remaining == 0 {
            return Ok(RoleChange::LastSuperAdmin);
        }
    }
    sqlx::query("UPDATE users SET role = $2, session_version = session_version + 1 WHERE id = $1")
        .bind(user_id)
        .bind(role)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(RoleChange::Done)
}

/// Emergency recovery: clear a failed-sign-in lockout so the account can sign
/// in again. The password is untouched. Returns false when the account was not
/// locked, so the caller does not claim to have done something it did not.
pub async fn unlock_account(db: &PgPool, user_id: i64) -> Res<bool> {
    let done = sqlx::query(
        r#"UPDATE users SET locked_until = NULL, failed_logins = 0
            WHERE id = $1 AND (locked_until IS NOT NULL OR failed_logins > 0)"#,
    )
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(done.rows_affected() > 0)
}

// ---------- The Principal appointment ----------

#[derive(Debug, FromRow)]
pub struct PrincipalRow {
    pub assignment_id: i64,
    pub user_id: i64,
    pub name: String,
    pub started_label: String,
    pub ends_label: String,
}

/// The Principal in office right now, if there is one.
pub async fn current_principal(db: &PgPool) -> Res<Option<PrincipalRow>> {
    sqlx::query_as::<_, PrincipalRow>(
        r#"SELECT ra.id AS assignment_id, ra.user_id, u.full_name AS name,
                  to_char(ra.starts_at, 'YYYY-MM-DD') AS started_label,
                  CASE WHEN ra.ends_at IS NULL THEN 'No end date'
                       ELSE to_char(ra.ends_at, 'YYYY-MM-DD') END AS ends_label
             FROM role_assignments ra
             JOIN users u ON u.id = ra.user_id
            WHERE ra.role = 'principal'
              AND ra.status = 'active'
              AND ra.starts_at <= now()
              AND (ra.ends_at IS NULL OR ra.ends_at > now())
            ORDER BY ra.starts_at DESC
            LIMIT 1"#,
    )
    .fetch_optional(db)
    .await
}

#[derive(Debug, FromRow)]
pub struct PrincipalCandidate {
    pub user_id: i64,
    pub name: String,
    pub role: String,
}

/// The active accounts a Principal appointment could be given to. A Super Admin
/// account is not offered: root and academic authority are kept separate.
pub async fn principal_candidates(db: &PgPool) -> Res<Vec<PrincipalCandidate>> {
    sqlx::query_as::<_, PrincipalCandidate>(
        r#"SELECT id AS user_id, full_name AS name, role
             FROM users
            WHERE is_active AND role IN ('staff', 'faculty', 'admin')
            ORDER BY full_name"#,
    )
    .fetch_all(db)
    .await
}

/// Appoint a Principal: give the account the `principal` role and record the
/// appointment, with its effective dates, as an active `role_assignments` row
/// in the same transaction.
///
/// A Super Admin cannot appoint themselves — root authority and academic
/// authority are separate offices — and an account cannot be appointed twice
/// while an appointment is live (the unique index refuses it).
pub async fn appoint_principal(
    db: &PgPool,
    user_id: i64,
    ends_at: Option<&str>,
    actor_id: i64,
    reason: &str,
) -> Res<i64> {
    if user_id == actor_id {
        return Err(sqlx::Error::Protocol(
            "a Super Admin cannot appoint themselves as Principal".into(),
        ));
    }
    let mut tx = db.begin().await?;
    let promoted = sqlx::query(
        "UPDATE users SET role = 'principal', session_version = session_version + 1
          WHERE id = $1 AND is_active AND role <> 'super_admin'",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    if promoted.rows_affected() == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO role_assignments
               (user_id, role, department_id, status, starts_at, ends_at, approved_by, approved_at, reason)
           VALUES ($1, 'principal', NULL, 'active', now(), NULLIF($2, '')::timestamptz, $3, now(), $4)
           RETURNING id"#,
    )
    .bind(user_id)
    .bind(ends_at.unwrap_or(""))
    .bind(actor_id)
    .bind(reason)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// End a Principal appointment: revoke the assignment and return the account to
/// the office-staff role, revoking its live sessions in the same write.
pub async fn revoke_principal(db: &PgPool, assignment_id: i64, actor_id: i64) -> Res<Option<i64>> {
    let _ = actor_id;
    let mut tx = db.begin().await?;
    let user_id: Option<i64> = sqlx::query_scalar(
        r#"UPDATE role_assignments
              SET status = 'revoked', ends_at = now(), updated_at = now()
            WHERE id = $1 AND role = 'principal' AND status = 'active'
        RETURNING user_id"#,
    )
    .bind(assignment_id)
    .bind(actor_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(user_id) = user_id {
        sqlx::query(
            "UPDATE users SET role = 'staff', session_version = session_version + 1
              WHERE id = $1 AND role = 'principal'",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(user_id)
}

// ---------- Emergency intervention in academic records ----------

#[derive(Debug, FromRow)]
pub struct FinalizedCourseRow {
    pub programme_id: i64,
    pub programme: String,
    pub semester: i32,
    pub course_id: i64,
    pub code: String,
    pub title: String,
    pub marks: i64,
}

/// Every course whose marks are currently locked, one row per programme,
/// semester and course. This is the list the emergency screen offers: if it is
/// not locked, the ordinary correction workflow applies and root authority is
/// not needed.
pub async fn list_finalized_courses(db: &PgPool) -> Res<Vec<FinalizedCourseRow>> {
    sqlx::query_as::<_, FinalizedCourseRow>(
        r#"SELECT st.programme_id, p.name AS programme, m.semester, m.course_id,
                  c.code, c.title, count(*) AS marks
             FROM marks m
             JOIN students st ON st.id = m.student_id
             JOIN programmes p ON p.id = st.programme_id
             JOIN courses c ON c.id = m.course_id
            WHERE m.workflow_state = 'finalized'
            GROUP BY st.programme_id, p.name, m.semester, m.course_id, c.code, c.title
            ORDER BY p.name, m.semester, c.code"#,
    )
    .fetch_all(db)
    .await
}

/// Reopen one finalized course, as the Super Admin's emergency intervention in
/// academic records.
///
/// Each locked row is first copied into `mark_history` as `superseded` with the
/// reason, so the value that was in force is preserved, and only then returned
/// to `reviewed` — where the ordinary edit, finalize and correction flow takes
/// over again. Rows already awaiting a correction decision are left alone.
/// Returns how many rows were reopened.
pub async fn reopen_finalized(
    db: &PgPool,
    programme_id: i64,
    semester: i32,
    course_id: i64,
    actor_id: i64,
    reason: &str,
) -> Res<u64> {
    let mut tx = db.begin().await?;
    sqlx::query(
        r#"INSERT INTO mark_history
               (mark_id, student_id, course_id, assessment, exam_kind, exam_name,
                semester, marks_obtained, max_marks, published, workflow_state,
                changed_by, change_reason)
           SELECT m.id, m.student_id, m.course_id, m.assessment, m.exam_kind, m.exam_name,
                  m.semester, m.marks_obtained, m.max_marks, m.published, 'superseded',
                  $4, $5
             FROM marks m
             JOIN students st ON st.id = m.student_id
            WHERE st.programme_id = $1
              AND m.semester = $2
              AND m.course_id = $3
              AND m.workflow_state = 'finalized'"#,
    )
    .bind(programme_id)
    .bind(semester)
    .bind(course_id)
    .bind(actor_id)
    .bind(reason)
    .execute(&mut *tx)
    .await?;

    let done = sqlx::query(
        r#"UPDATE marks m
              SET workflow_state = 'reviewed',
                  finalized_at   = NULL,
                  finalized_by   = NULL,
                  updated_at     = now()
             FROM students st
            WHERE st.id = m.student_id
              AND st.programme_id = $1
              AND m.semester = $2
              AND m.course_id = $3
              AND m.workflow_state = 'finalized'"#,
    )
    .bind(programme_id)
    .bind(semester)
    .bind(course_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(done.rows_affected())
}
