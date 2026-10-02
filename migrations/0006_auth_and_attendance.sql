-- Phase 4a: account safety, enrollments, per-period attendance, substitutions, audit log.

ALTER TABLE users
    ADD COLUMN full_name           TEXT        NOT NULL DEFAULT '',
    ADD COLUMN must_change_password BOOLEAN    NOT NULL DEFAULT false,
    ADD COLUMN failed_logins       INT         NOT NULL DEFAULT 0,
    ADD COLUMN locked_until        TIMESTAMPTZ,
    ADD COLUMN password_changed_at TIMESTAMPTZ;

ALTER TABLE faculty  ADD COLUMN is_hod  BOOLEAN NOT NULL DEFAULT false;
-- E-grants students must keep 75% attendance every month (not only per semester).
ALTER TABLE students ADD COLUMN egrants BOOLEAN NOT NULL DEFAULT false;

-- The student sign-in page is now a real route (/login), not an informational page.
DELETE FROM pages WHERE path = '/hub/login';

CREATE TABLE enrollments (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    student_id BIGINT      NOT NULL REFERENCES students (id) ON DELETE CASCADE,
    course_id  BIGINT      NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    status     TEXT        NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'dropped')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (student_id, course_id)
);
CREATE INDEX enrollments_course_idx ON enrollments (course_id);

-- Attendance moves from "one row per student/course/day" to "one session per timetable
-- slot and date, with one record per student". The old table was never used.
DROP TABLE attendance;

CREATE TABLE attendance_sessions (
    id                 BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    timetable_entry_id BIGINT      NOT NULL REFERENCES timetable_entries (id) ON DELETE RESTRICT,
    course_id          BIGINT      NOT NULL REFERENCES courses (id) ON DELETE RESTRICT,
    on_date            DATE        NOT NULL,
    taught_by          BIGINT      REFERENCES faculty (id) ON DELETE SET NULL, -- actual teacher (may be a substitute)
    marked_by          BIGINT      REFERENCES users (id) ON DELETE SET NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (timetable_entry_id, on_date)
);
CREATE INDEX attendance_sessions_course_date_idx ON attendance_sessions (course_id, on_date);

CREATE TABLE attendance_records (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    session_id BIGINT      NOT NULL REFERENCES attendance_sessions (id) ON DELETE CASCADE,
    student_id BIGINT      NOT NULL REFERENCES students (id) ON DELETE CASCADE,
    status     TEXT        NOT NULL CHECK (status IN ('present', 'absent', 'leave')),
    updated_by BIGINT      REFERENCES users (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (session_id, student_id)
);
CREATE INDEX attendance_records_student_idx ON attendance_records (student_id);

CREATE TABLE substitutions (
    id                    BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    timetable_entry_id    BIGINT      NOT NULL REFERENCES timetable_entries (id) ON DELETE CASCADE,
    on_date               DATE        NOT NULL,
    substitute_faculty_id BIGINT      NOT NULL REFERENCES faculty (id) ON DELETE CASCADE,
    reason                TEXT        NOT NULL DEFAULT '',
    created_by            BIGINT      REFERENCES users (id) ON DELETE SET NULL,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (timetable_entry_id, on_date)
);

CREATE TABLE audit_log (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    actor_user_id BIGINT      REFERENCES users (id) ON DELETE SET NULL,
    action        TEXT        NOT NULL,
    entity        TEXT        NOT NULL,
    entity_id     BIGINT,
    details       JSONB,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX audit_log_entity_idx ON audit_log (entity, entity_id, created_at DESC);

DO $$
DECLARE
    t text;
BEGIN
    FOREACH t IN ARRAY ARRAY['enrollments', 'attendance_sessions', 'attendance_records', 'substitutions']
    LOOP
        EXECUTE format(
            'CREATE TRIGGER %I BEFORE UPDATE ON %I FOR EACH ROW EXECUTE FUNCTION set_updated_at()',
            t || '_set_updated_at', t
        );
    END LOOP;
END $$;

-- Attendance rules (editable later from the admin dashboard).
INSERT INTO site_settings (key, value) VALUES
    ('attendance_min_percent', '75'),
    ('attendance_egrants_monthly_min_percent', '75'),
    ('attendance_edit_window_days', '5')
ON CONFLICT (key) DO NOTHING;
