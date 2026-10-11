-- The Principal / Academic Admin is a distinct account role: institution-wide
-- academic authority, no technical administration powers. (The appointment
-- table already accepts a 'principal' role row; this is the account role.)
ALTER TABLE users DROP CONSTRAINT users_role_check;
ALTER TABLE users ADD CONSTRAINT users_role_check
    CHECK (role IN ('student', 'faculty', 'staff', 'admin', 'alumni', 'principal'));
