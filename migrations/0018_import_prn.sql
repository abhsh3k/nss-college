-- PRN (permanent registration number) on staged import rows, so a file can be
-- laid out as admission_no, prn, name, email, phone. Blank is allowed: PRN is
-- optional on `students` too (NULL means "not yet issued").
ALTER TABLE import_rows ADD COLUMN prn TEXT NOT NULL DEFAULT '';
