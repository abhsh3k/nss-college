-- Two things the result import learned:
--
-- * a student's name may travel beside their key in the file, as a label the
--   teacher can read back on the review page. It is never matched against the
--   database — the PRN/admission number is the identity — so it is kept purely
--   so the person filing the marks can see whose line they are looking at;
--
-- * a course-wise sheet (one line per student, one column per course) is
--   expanded into the same one-row-per-result batch, so the review and the
--   commit are shared, and each course's maximum is set on the review rather
--   than carried by every row of the file.
--
-- The row's `maximum` already exists, so nothing else is needed for the second:
-- the course-wise form leaves it at the sheet's default until it is edited.

ALTER TABLE result_import_rows
    ADD COLUMN student_name TEXT NOT NULL DEFAULT '';
