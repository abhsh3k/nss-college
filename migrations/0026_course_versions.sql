-- Stable course identity and course versions.
--
-- `courses` is the mutable current row: a head of department edits the code,
-- title, credits, category or suggested semester in place. That is fine for
-- "what we teach this year" but it loses what the course *was* when a semester
-- ran, so an offering — and therefore the classes, attendance and results
-- attached to it — could no longer be attributed to the wording it was created
-- under.
--
-- This migration adds an append-only version history beside `courses`:
--
--   * every existing course becomes version 1 (roster, title, credits,
--     category, suggested semester);
--   * `courses` keeps being the current values, so no reader has to change;
--   * every offering records the version it was created from, so historic
--     offerings stay attributable;
--   * editing a catalogue course appends a version instead of overwriting the
--     previous one (`courses::update_catalogue_course`).
--
-- Nothing is dropped, so this is additive and reversible (drop the version
-- columns/table to roll back; the `courses` row is unchanged).

CREATE TABLE course_versions (
    id             BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    course_id      BIGINT NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    version_no     INT    NOT NULL CHECK (version_no > 0),
    code           TEXT   NOT NULL,
    title          TEXT   NOT NULL,
    credits        INT    NOT NULL DEFAULT 0,
    category       TEXT   NOT NULL DEFAULT '',
    semester       INT,
    effective_from DATE   NOT NULL DEFAULT CURRENT_DATE,
    is_current     BOOLEAN NOT NULL DEFAULT true,
    created_by     BIGINT REFERENCES users (id) ON DELETE SET NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (course_id, version_no)
);
-- Exactly one current version per course, ever.
CREATE UNIQUE INDEX course_versions_current_idx
    ON course_versions (course_id) WHERE is_current;
CREATE INDEX course_versions_course_idx ON course_versions (course_id, version_no DESC);

-- Backfill: one version per existing course, marked current.
INSERT INTO course_versions
       (course_id, version_no, code, title, credits, category, semester, is_current)
SELECT id, 1, code, title, credits, category, semester, true
  FROM courses;

-- ---------------------------------------------------------------------
-- Offerings remember the version they were built from.
-- ---------------------------------------------------------------------
ALTER TABLE course_offerings
    ADD COLUMN course_version_id BIGINT REFERENCES course_versions (id) ON DELETE RESTRICT;

UPDATE course_offerings o
   SET course_version_id = v.id
  FROM course_versions v
 WHERE v.course_id = o.course_id AND v.is_current;

-- Every offering now has a version: a new course always gets version 1 at
-- creation, so nothing legitimate can be left without one.
ALTER TABLE course_offerings ALTER COLUMN course_version_id SET NOT NULL;
CREATE INDEX course_offerings_version_idx ON course_offerings (course_version_id);

-- Verification:
--   SELECT count(*) FROM courses c WHERE NOT EXISTS
--       (SELECT 1 FROM course_versions v WHERE v.course_id = c.id AND v.is_current);
--   -- must be 0
--   SELECT count(*) FROM course_offerings WHERE course_version_id IS NULL;
--   -- must be 0
