-- Assessment/result lifecycle and correction history.

ALTER TABLE marks
    ADD COLUMN workflow_state TEXT NOT NULL DEFAULT 'published'
        CHECK (workflow_state IN ('draft', 'reviewed', 'finalized', 'published', 'correction_pending', 'superseded')),
    ADD COLUMN finalized_at TIMESTAMPTZ,
    ADD COLUMN finalized_by BIGINT REFERENCES users (id) ON DELETE SET NULL,
    ADD COLUMN correction_reason TEXT NOT NULL DEFAULT '';

UPDATE marks
   SET workflow_state = CASE WHEN published THEN 'published' ELSE 'draft' END;

CREATE TABLE mark_history (
    id              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    mark_id         BIGINT REFERENCES marks (id) ON DELETE SET NULL,
    student_id      BIGINT NOT NULL REFERENCES students (id) ON DELETE RESTRICT,
    course_id       BIGINT NOT NULL REFERENCES courses (id) ON DELETE RESTRICT,
    assessment      TEXT NOT NULL,
    exam_kind       TEXT NOT NULL,
    exam_name       TEXT NOT NULL DEFAULT '',
    semester        INT,
    marks_obtained  NUMERIC(6, 2) NOT NULL,
    max_marks       NUMERIC(6, 2) NOT NULL,
    published       BOOLEAN NOT NULL,
    workflow_state  TEXT NOT NULL,
    changed_by      BIGINT REFERENCES users (id) ON DELETE SET NULL,
    change_reason   TEXT NOT NULL DEFAULT '',
    changed_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX mark_history_lookup_idx
    ON mark_history (student_id, course_id, semester, changed_at DESC);

CREATE TABLE result_import_batches (
    id              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    created_by      BIGINT REFERENCES users (id) ON DELETE SET NULL,
    programme_id    BIGINT REFERENCES programmes (id) ON DELETE RESTRICT,
    semester        INT NOT NULL CHECK (semester BETWEEN 1 AND 12),
    exam_kind       TEXT NOT NULL CHECK (exam_kind IN ('coursework', 'internal', 'university')),
    source_name     TEXT NOT NULL DEFAULT '',
    state           TEXT NOT NULL DEFAULT 'preview'
                    CHECK (state IN ('preview', 'confirmed', 'committed', 'expired', 'cancelled')),
    expires_at      TIMESTAMPTZ NOT NULL DEFAULT now() + interval '24 hours',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    confirmed_at    TIMESTAMPTZ,
    committed_at    TIMESTAMPTZ
);

CREATE TABLE result_import_rows (
    id              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    batch_id        BIGINT NOT NULL REFERENCES result_import_batches (id) ON DELETE CASCADE,
    line_no         INT NOT NULL,
    identity_key    TEXT NOT NULL,
    course_code     TEXT NOT NULL,
    course_id       BIGINT REFERENCES courses (id) ON DELETE RESTRICT,
    student_id      BIGINT REFERENCES students (id) ON DELETE RESTRICT,
    obtained        NUMERIC(6, 2),
    maximum         NUMERIC(6, 2),
    state           TEXT NOT NULL DEFAULT 'error'
                    CHECK (state IN ('valid', 'error', 'duplicate', 'conflict', 'committed')),
    error_message   TEXT NOT NULL DEFAULT '',
    mapping_note    TEXT NOT NULL DEFAULT ''
);
CREATE INDEX result_import_rows_batch_idx ON result_import_rows (batch_id, line_no);
