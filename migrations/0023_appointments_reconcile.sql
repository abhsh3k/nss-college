-- Reconcile the compatibility role_assignments with the current faculty flags.
--
-- Migration 0021 backfilled role_assignments from faculty.is_hod once, but the
-- faculty flags are edited afterwards by the people tools and the demo seeder,
-- so the two drifted: an account could keep an active head assignment after
-- being demoted, or a real head could have none. The application now treats an
-- active head assignment as authoritative, so the table has to start correct.

-- 1. Revoke every active head assignment whose faculty is no longer a head of
--    that department.
UPDATE role_assignments ra
   SET status = 'revoked', ends_at = now(), updated_at = now()
 WHERE ra.role IN ('hod', 'acting_hod')
   AND ra.status = 'active'
   AND ra.ends_at IS NULL
   AND NOT EXISTS (
        SELECT 1 FROM faculty f
         WHERE f.user_id = ra.user_id
           AND f.is_hod
           AND f.status = 'published'
           AND f.department_id = ra.department_id);

-- 2. Give every current head an active assignment, unless the department already
--    has one. One head per department is enforced by `one_active_department_head`,
--    so DISTINCT ON keeps a duplicated head flag from aborting the migration.
INSERT INTO role_assignments
    (user_id, role, department_id, status, starts_at, approved_at, reason)
SELECT DISTINCT ON (f.department_id)
       f.user_id, 'hod', f.department_id, 'active', now(), now(),
       '0023 reconciliation'
  FROM faculty f
 WHERE f.is_hod
   AND f.status = 'published'
   AND f.department_id IS NOT NULL
   AND NOT EXISTS (
        SELECT 1 FROM role_assignments ra
         WHERE ra.department_id = f.department_id
           AND ra.role IN ('hod', 'acting_hod')
           AND ra.status = 'active'
           AND ra.ends_at IS NULL)
 ORDER BY f.department_id, f.user_id;
