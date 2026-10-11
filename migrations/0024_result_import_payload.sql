-- A staged result import keeps the raw text it was built from, so the commit
-- step re-parses and re-validates the exact same rows instead of trusting a
-- second upload or a lossy per-row reconstruction.

ALTER TABLE result_import_batches
    ADD COLUMN payload TEXT NOT NULL DEFAULT '';
