-- Course offerings and student course selection.
--
-- Until now a course always belonged to one programme (`courses.programme_id`
-- NOT NULL) and every student of a programme semester was bulk-enrolled in all
-- of that semester's courses. That stays true for a programme's own fixed
-- courses. This migration adds the other half, without duplicating anything:
--
--   * A department can also own *catalogue* courses (`courses.programme_id`
--     NULL, `department_id` set), e.g. Communicative English in English.
--   * A Course Offering is one catalogue course taught in one academic year
--     and semester, with a selection mode (FIXED, HOD_ASSIGNED,
--     INDIVIDUAL_CHOICE or COHORT_CHOICE) and a capacity.
--   * The offering department's HOD publishes the offering to programmes
--     and/or cohorts (targets). Each receiving department's HOD then approves
--     or rejects it for their own students (approvals).
--   * Students select from the offerings they are eligible for, review and
--     confirm (selections). A confirmed selection drives the existing
--     `enrollments` row, and timetable/attendance follow the enrollment.
--   * Confirmed choices are not edited silently: a student asks for a change,
--     the HOD approves it, and `course_change_requests` keeps who, what, when
--     and why. `cohort_specializations` records a cohort-wide decision (the
--     BCA 2026-2030 AI/ML-vs-Full-Stack case) for any programme.
--
-- Everything is driven by rows, not department names: a new programme,
-- department or course category needs data, never a code change.

-- ---------------------------------------------------------------- setup ----

CREATE TABLE academic_years (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    label      TEXT    NOT NULL UNIQUE,          -- "2026-2027"
    start_year INT     NOT NULL UNIQUE,
    end_year   INT     NOT NULL,
    is_current BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (end_year = start_year + 1)
);
-- Only one current year at a time.
CREATE UNIQUE INDEX academic_years_current_idx ON academic_years (is_current) WHERE is_current;

-- A cohort is a programme's batch: BCA + 2026 is "BCA 2026-2030". Rows are
-- created on demand the first time an HOD addresses that batch, so there is
-- nothing to maintain for programmes that never use cohorts.
CREATE TABLE cohorts (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    programme_id BIGINT NOT NULL REFERENCES programmes (id) ON DELETE CASCADE,
    batch_year   INT    NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (programme_id, batch_year)
);
CREATE INDEX cohorts_programme_idx ON cohorts (programme_id, batch_year);

-- ------------------------------------------------- department catalogue ----

-- Programme-bound courses stay exactly as they are. A catalogue course belongs
-- to a department instead and is made concrete by an offering.
ALTER TABLE courses ALTER COLUMN programme_id DROP NOT NULL;
ALTER TABLE courses ALTER COLUMN semester    DROP NOT NULL;
ALTER TABLE courses ADD COLUMN department_id BIGINT REFERENCES departments (id) ON DELETE CASCADE;
ALTER TABLE courses ADD COLUMN category      TEXT NOT NULL DEFAULT ''
    CHECK (category IN ('', 'MAJOR', 'MINOR', 'SPECIALIZATION', 'DSC', 'DSE', 'MDC', 'SEC', 'VAC', 'AEC'));
ALTER TABLE courses ADD COLUMN is_active     BOOLEAN NOT NULL DEFAULT true;

-- Every course must belong somewhere, and a course with no programme needs a
-- department. Existing rows already satisfy both.
ALTER TABLE courses ADD CONSTRAINT courses_owner_check
    CHECK (programme_id IS NOT NULL OR department_id IS NOT NULL);
ALTER TABLE courses ADD CONSTRAINT courses_programme_implies_semester_check
    CHECK (programme_id IS NULL OR semester IS NOT NULL);

CREATE INDEX courses_department_idx ON courses (department_id) WHERE department_id IS NOT NULL;
-- The old UNIQUE (programme_id, code) lets NULL programme_ids repeat, so pin
-- catalogue course codes per department explicitly.
CREATE UNIQUE INDEX courses_department_code_idx
    ON courses (department_id, code)
    WHERE department_id IS NOT NULL AND programme_id IS NULL;

-- Backfill: a programme's courses belong to that programme's department.
UPDATE courses c
   SET department_id = p.department_id
  FROM programmes p
 WHERE p.id = c.programme_id
   AND c.department_id IS NULL;

-- ------------------------------------------------------- course offerings --

CREATE TABLE course_offerings (
    id                    BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    course_id             BIGINT      NOT NULL REFERENCES courses (id) ON DELETE RESTRICT,
    offering_department_id BIGINT     NOT NULL REFERENCES departments (id) ON DELETE RESTRICT,
    academic_year_id      BIGINT      NOT NULL REFERENCES academic_years (id) ON DELETE RESTRICT,
    semester              INT         NOT NULL CHECK (semester BETWEEN 1 AND 12),
    -- Free-text-friendly course type so future MGU categories are data, not migrations.
    course_type           TEXT        NOT NULL DEFAULT 'Regular',
    selection_mode        TEXT        NOT NULL
        CHECK (selection_mode IN ('FIXED', 'HOD_ASSIGNED', 'INDIVIDUAL_CHOICE', 'COHORT_CHOICE')),
    -- COHORT_CHOICE offerings with the same group are alternatives: the HOD
    -- finalises exactly one of them for a cohort (AI/ML vs Full Stack).
    choice_group          TEXT        NOT NULL DEFAULT '',
    capacity              INT         CHECK (capacity IS NULL OR capacity > 0),
    faculty_id            BIGINT      REFERENCES faculty (id) ON DELETE SET NULL, -- default teacher
    status                TEXT        NOT NULL DEFAULT 'draft'
        CHECK (status IN ('draft', 'published', 'archived')),
    published_at          TIMESTAMPTZ,
    created_by            BIGINT      REFERENCES users (id) ON DELETE SET NULL,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- One offering of a course per year and semester.
    UNIQUE (course_id, academic_year_id, semester),
    CHECK (choice_group <> '' OR selection_mode <> 'COHORT_CHOICE')
);
CREATE INDEX course_offerings_scope_idx ON course_offerings (offering_department_id, academic_year_id, semester);
CREATE INDEX course_offerings_lookup_idx ON course_offerings (status, academic_year_id, semester);

-- Who the offering is open to: whole programmes (cohort_id NULL) or specific
-- cohorts (programme_id + cohort_id set together).
CREATE TABLE course_offering_targets (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    offering_id  BIGINT NOT NULL REFERENCES course_offerings (id) ON DELETE CASCADE,
    programme_id BIGINT NOT NULL REFERENCES programmes (id) ON DELETE CASCADE,
    cohort_id    BIGINT REFERENCES cohorts (id) ON DELETE CASCADE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE NULLS NOT DISTINCT (offering_id, programme_id, cohort_id)
);
CREATE INDEX course_offering_targets_programme_idx ON course_offering_targets (programme_id);
CREATE INDEX course_offering_targets_cohort_idx ON course_offering_targets (cohort_id) WHERE cohort_id IS NOT NULL;

-- The receiving department's decision. A row appears as 'pending' when the
-- offering is published, so the receiving HOD sees it without any polling.
CREATE TABLE course_offering_approvals (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    offering_id   BIGINT NOT NULL REFERENCES course_offerings (id) ON DELETE CASCADE,
    department_id BIGINT NOT NULL REFERENCES departments (id) ON DELETE CASCADE,
    status        TEXT   NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'rejected')),
    note          TEXT   NOT NULL DEFAULT '',
    decided_by    BIGINT REFERENCES users (id) ON DELETE SET NULL,
    decided_at    TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (offering_id, department_id)
);
CREATE INDEX course_offering_approvals_dept_idx ON course_offering_approvals (department_id, status);

-- --------------------------------------------------------- student choice --

CREATE TABLE student_course_selections (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    student_id    BIGINT NOT NULL REFERENCES students (id) ON DELETE CASCADE,
    offering_id   BIGINT NOT NULL REFERENCES course_offerings (id) ON DELETE CASCADE,
    state         TEXT   NOT NULL DEFAULT 'draft'
        CHECK (state IN ('draft', 'submitted', 'confirmed', 'locked', 'change_requested')),
    submitted_at  TIMESTAMPTZ,
    confirmed_at  TIMESTAMPTZ,
    locked_by     BIGINT REFERENCES users (id) ON DELETE SET NULL,
    locked_at     TIMESTAMPTZ,
    -- Who confirmed/assigned on the student's behalf (HOD or cohort decision).
    decided_by    BIGINT REFERENCES users (id) ON DELETE SET NULL,
    decided_note  TEXT   NOT NULL DEFAULT '',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (student_id, offering_id)
);
CREATE INDEX student_course_selections_student_idx ON student_course_selections (student_id, state);
CREATE INDEX student_course_selections_offering_idx ON student_course_selections (offering_id, state);

-- A student's request to move from one confirmed offering to another. The old
-- and new offerings, the reason and the decision are kept; nothing is
-- overwritten in place.
CREATE TABLE course_change_requests (
    id              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    student_id      BIGINT NOT NULL REFERENCES students (id) ON DELETE CASCADE,
    offering_id     BIGINT NOT NULL REFERENCES course_offerings (id) ON DELETE RESTRICT, -- current
    new_offering_id BIGINT NOT NULL REFERENCES course_offerings (id) ON DELETE RESTRICT, -- wanted
    reason          TEXT   NOT NULL DEFAULT '',
    status          TEXT   NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'rejected')),
    decision_note   TEXT   NOT NULL DEFAULT '',
    decided_by      BIGINT REFERENCES users (id) ON DELETE SET NULL,
    decided_at      TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (new_offering_id <> offering_id)
);
CREATE INDEX course_change_requests_student_idx ON course_change_requests (student_id);
CREATE INDEX course_change_requests_status_idx ON course_change_requests (status, created_at);

-- A cohort-wide finalisation: for one choice group, exactly one offering per
-- cohort. Works for BCA's specialisation today and any programme tomorrow.
CREATE TABLE cohort_specializations (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    cohort_id    BIGINT NOT NULL REFERENCES cohorts (id) ON DELETE CASCADE,
    choice_group TEXT   NOT NULL,
    offering_id  BIGINT NOT NULL REFERENCES course_offerings (id) ON DELETE CASCADE,
    note         TEXT   NOT NULL DEFAULT '',
    decided_by   BIGINT REFERENCES users (id) ON DELETE SET NULL,
    decided_at   TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (cohort_id, choice_group)
);
CREATE INDEX cohort_specializations_offering_idx ON cohort_specializations (offering_id);

-- --------------------------------------------- timetable for offerings ----

-- A period belongs either to a programme class (as before) or to a course
-- offering taught to a mixed group. Exactly one of the two is set.
ALTER TABLE timetable_entries ALTER COLUMN programme_id DROP NOT NULL;
ALTER TABLE timetable_entries
    ADD COLUMN course_offering_id BIGINT REFERENCES course_offerings (id) ON DELETE CASCADE;
ALTER TABLE timetable_entries ADD CONSTRAINT timetable_entries_owner_check
    CHECK ((programme_id IS NOT NULL)::int + (course_offering_id IS NOT NULL)::int = 1);
CREATE INDEX timetable_entries_offering_idx ON timetable_entries (course_offering_id)
    WHERE course_offering_id IS NOT NULL;

-- ------------------------------------------------------- bookkeeping  ----

INSERT INTO academic_years (label, start_year, end_year, is_current) VALUES
    ('2025-2026', 2025, 2026, false),
    ('2026-2027', 2026, 2027, true);

DO $$
DECLARE
    t text;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'academic_years', 'cohorts', 'course_offerings', 'course_offering_targets',
        'course_offering_approvals', 'student_course_selections',
        'course_change_requests', 'cohort_specializations'
    ]
    LOOP
        EXECUTE format(
            'CREATE TRIGGER %I BEFORE UPDATE ON %I FOR EACH ROW EXECUTE FUNCTION set_updated_at()',
            t || '_set_updated_at', t
        );
    END LOOP;
END $$;
