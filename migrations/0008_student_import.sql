-- Staging tables for the file-based student import.
--
-- An uploaded CSV/Excel file is parsed into `import_rows` first. The admin
-- reviews and edits them there, then commits only the ticked rows. Nothing in
-- `students` or `users` is touched until that commit.

CREATE TABLE import_batches (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    source_name  TEXT        NOT NULL DEFAULT '',
    -- Defaults for rows that do not carry their own programme/semester/year.
    programme_id BIGINT      NOT NULL REFERENCES programmes (id) ON DELETE RESTRICT,
    semester     INT         NOT NULL DEFAULT 1 CHECK (semester BETWEEN 1 AND 12),
    batch_year   INT         NOT NULL CHECK (batch_year BETWEEN 2000 AND 2100),
    created_by   BIGINT      REFERENCES users (id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE import_rows (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    batch_id      BIGINT      NOT NULL REFERENCES import_batches (id) ON DELETE CASCADE,
    line_no       INT         NOT NULL,
    admission_no  TEXT        NOT NULL DEFAULT '',
    name          TEXT        NOT NULL DEFAULT '',
    email         TEXT        NOT NULL DEFAULT '',
    phone         TEXT        NOT NULL DEFAULT '',
    egrants       BOOLEAN     NOT NULL DEFAULT false,-- Free text, matched against programmes.name/slug when the row is saved
    -- and again at commit time, so a renamed programme is still resolved. The
    -- three overrides are kept as text so a value the admin has not corrected
    -- yet stays visible on the review page instead of being quietly dropped.
    programme_text TEXT        NOT NULL DEFAULT '',
    semester_text  TEXT        NOT NULL DEFAULT '',
    year_text      TEXT        NOT NULL DEFAULT '',
    include       BOOLEAN     NOT NULL DEFAULT true,
    status        TEXT        NOT NULL DEFAULT 'pending'
                  CHECK (status IN ('pending', 'created', 'skipped')),
    note          TEXT        NOT NULL DEFAULT '',
    user_id       BIGINT      REFERENCES users (id) ON DELETE SET NULL,
    -- One-time password, kept only until the batch is finished or discarded
    -- so the credentials can still be downloaded as CSV.
    temp_password TEXT        NOT NULL DEFAULT '',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX import_rows_batch_idx ON import_rows (batch_id, line_no);

-- Generated temporary passwords must not outlive the import that made them.
-- Any batch older than this is swept whenever a new file is uploaded.
CREATE FUNCTION purge_stale_import_batches() RETURNS void AS $$
    DELETE FROM import_batches WHERE created_at < now() - interval '7 days';
$$ LANGUAGE sql;
