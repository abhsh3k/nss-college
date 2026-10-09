-- Departments that teach but run no programme of their own.
--
-- English, Malayalam and Mathematics own catalogue courses and offer them to
-- other departments' programmes through course offerings. A department with
-- no programmes has no students of its own, so nobody is enrolled directly
-- in them: every student reaches their courses through an offering, an
-- approval and a selection, exactly like any other cross-department course.

INSERT INTO departments (slug, name, summary, sort_order) VALUES
  ('english', 'English', 'Language courses offered to the students of other departments.', 5),
  ('malayalam', 'Malayalam', 'Language and literature courses offered to the students of other departments.', 6),
  ('mathematics', 'Mathematics', 'Mathematics courses offered to the students of other departments.', 7);
