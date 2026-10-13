-- Self-service public profiles for the Principal and office staff.
--
-- A teacher's public card already comes from their `faculty` row, which the IT
-- admin manages and which the timetable and courses also reference. The
-- Principal and office staff have no such row, so this table gives them a place
-- to keep the profile that appears on `/about/staff`: a display name, a
-- designation, a qualification, a contact address, a short biography and a
-- photo. A row is created the first time they save their own profile, so a
-- member of staff who never fills it in is simply not listed.
--
-- Keyed by `user_id`, and removed with the account, so a deleted account cannot
-- leave a profile behind. `published` lets an administrator take a profile off
-- the public page without deleting it.

CREATE TABLE staff_profiles (
    user_id       BIGINT      PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    display_name  TEXT        NOT NULL DEFAULT '',
    designation   TEXT        NOT NULL DEFAULT '',
    qualification TEXT        NOT NULL DEFAULT '',
    email         TEXT,
    bio           TEXT        NOT NULL DEFAULT '',
    photo_path    TEXT,
    sort_order    INT         NOT NULL DEFAULT 0,
    published     BOOLEAN     NOT NULL DEFAULT true,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TRIGGER staff_profiles_set_updated_at
    BEFORE UPDATE ON staff_profiles
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
