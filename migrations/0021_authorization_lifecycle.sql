-- Authorization lifecycle foundation.
--
-- These tables are additive so current mock accounts and academic history remain
-- valid while handlers migrate from role/boolean flags to effective assignments.

ALTER TABLE users
    ADD COLUMN session_version BIGINT NOT NULL DEFAULT 0;

CREATE TABLE role_assignments (
    id              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id         BIGINT NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    role            TEXT NOT NULL CHECK (role IN (
        'super_admin', 'it_admin', 'principal', 'hod', 'acting_hod',
        'faculty', 'student', 'staff', 'alumni'
    )),
    department_id   BIGINT REFERENCES departments (id) ON DELETE RESTRICT,
    starts_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    ends_at         TIMESTAMPTZ,
    status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'active', 'revoked', 'expired')),
    approved_by     BIGINT REFERENCES users (id) ON DELETE SET NULL,
    approved_at     TIMESTAMPTZ,
    reason          TEXT NOT NULL DEFAULT '',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (ends_at IS NULL OR ends_at > starts_at),
    CHECK ((role IN ('hod', 'acting_hod') AND department_id IS NOT NULL)
        OR role NOT IN ('hod', 'acting_hod'))
);
CREATE INDEX role_assignments_user_idx ON role_assignments (user_id, status, starts_at);
CREATE INDEX role_assignments_scope_idx ON role_assignments (department_id, role, status);
CREATE UNIQUE INDEX one_active_department_head
    ON role_assignments (department_id)
    WHERE role IN ('hod', 'acting_hod') AND status = 'active' AND ends_at IS NULL;

CREATE TABLE delegations (
    id                  BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    granting_assignment_id BIGINT NOT NULL REFERENCES role_assignments (id) ON DELETE RESTRICT,
    delegate_user_id    BIGINT NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    permission          TEXT NOT NULL,
    starts_at           TIMESTAMPTZ NOT NULL,
    ends_at             TIMESTAMPTZ NOT NULL,
    status              TEXT NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active', 'revoked', 'expired', 'needs_review')),
    reviewed_by         BIGINT REFERENCES users (id) ON DELETE SET NULL,
    reviewed_at         TIMESTAMPTZ,
    reason              TEXT NOT NULL DEFAULT '',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (ends_at > starts_at)
);
CREATE INDEX delegations_delegate_idx ON delegations (delegate_user_id, status, starts_at, ends_at);

CREATE TABLE appointment_handover_tasks (
    id                  BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    assignment_id       BIGINT NOT NULL REFERENCES role_assignments (id) ON DELETE RESTRICT,
    successor_user_id   BIGINT REFERENCES users (id) ON DELETE SET NULL,
    entity              TEXT NOT NULL,
    entity_id           BIGINT,
    status              TEXT NOT NULL DEFAULT 'open'
                        CHECK (status IN ('open', 'reviewed', 'closed')),
    note                TEXT NOT NULL DEFAULT '',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    closed_at           TIMESTAMPTZ
);
CREATE INDEX appointment_handover_open_idx
    ON appointment_handover_tasks (assignment_id, status);

-- Initial compatibility backfill. Future appointment changes must use the
-- lifecycle tables rather than mutating these legacy role/boolean columns.
INSERT INTO role_assignments
    (user_id, role, department_id, status, starts_at, approved_at, reason)
SELECT u.id,
       CASE
           WHEN u.role = 'admin' THEN 'it_admin'
           WHEN u.role = 'faculty' AND f.is_hod AND f.department_id IS NOT NULL THEN 'hod'
           WHEN u.role = 'faculty' THEN 'faculty'
           WHEN u.role = 'student' THEN 'student'
           WHEN u.role = 'alumni' THEN 'alumni'
           ELSE 'staff'
       END,
       f.department_id,
       CASE WHEN u.is_active THEN 'active' ELSE 'revoked' END,
       u.created_at,
       u.created_at,
       '0021 compatibility backfill'
  FROM users u
  LEFT JOIN faculty f ON f.user_id = u.id
 WHERE NOT EXISTS (SELECT 1 FROM role_assignments ra WHERE ra.user_id = u.id);

-- Ordinary application roles may append audit rows but must not rewrite history.
CREATE OR REPLACE FUNCTION prevent_audit_mutation() RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'audit_log is append-only';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER audit_log_no_update
    BEFORE UPDATE OR DELETE ON audit_log
    FOR EACH ROW EXECUTE FUNCTION prevent_audit_mutation();

CREATE TRIGGER role_assignments_set_updated_at
    BEFORE UPDATE ON role_assignments
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER delegations_set_updated_at
    BEFORE UPDATE ON delegations
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
