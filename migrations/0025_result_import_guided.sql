-- The guided marks import (from the marks grid) fixes the assessment, exam kind
-- and exam name from the sheet the head has open, rather than reading them off
-- each row. Store them on the batch so the commit files the same way.

ALTER TABLE result_import_batches
    ADD COLUMN assessment TEXT NOT NULL DEFAULT '',
    ADD COLUMN exam_name TEXT NOT NULL DEFAULT '';
