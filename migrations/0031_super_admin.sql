-- Root authority: the Super Admin account role.
--
-- The dedicated loopback-only listener already exists (see `main.rs`); this is
-- the account role that runs it. It is deliberately *not* reachable from the
-- people form: the IT admin can create staff, teachers and students, but only a
-- Super Admin can promote an account to `admin` or `super_admin` (see
-- `services::super_admin::set_role`). The bootstrap account comes from the CLI
-- (`create-user super_admin <email> "Name"`).
ALTER TABLE users DROP CONSTRAINT users_role_check;
ALTER TABLE users ADD CONSTRAINT users_role_check
    CHECK (role IN ('student', 'faculty', 'staff', 'admin', 'alumni', 'principal', 'super_admin'));

-- One Principal appointment at a time, and one request at a time, mirroring the
-- one-head-per-department rule. The account role (`users.role = 'principal'`) is
-- what grants the academic workspace; the assignment row is the record of the
-- Super Admin's appointment, its effective dates and its audit trail.
CREATE UNIQUE INDEX one_active_principal
    ON role_assignments (role)
    WHERE role = 'principal' AND status = 'active';
CREATE UNIQUE INDEX one_pending_principal
    ON role_assignments (role)
    WHERE role = 'principal' AND status = 'pending';
