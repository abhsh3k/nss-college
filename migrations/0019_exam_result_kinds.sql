-- Two kinds of exam result are pushed to students, under two different keys:
--
--   university — the university's own semester exam, pushed keyed by the PRN;
--   internal   — an exam the college itself conducts, pushed keyed by the
--                admission number and named ("Internal 1", "Unit test") so a
--                course can carry several of them.
--
--   coursework — every other row (internal, assignment, practical, external).
ALTER TABLE marks ADD COLUMN exam_kind TEXT NOT NULL DEFAULT 'coursework'
    CHECK (exam_kind IN ('coursework', 'university', 'internal'));

-- The internal exam's own name; empty for every other kind.
ALTER TABLE marks ADD COLUMN exam_name TEXT NOT NULL DEFAULT '';

-- The semester the result was earned in. The student's own semester moves on
-- when they are promoted, so past results are read from here rather than from
-- wherever the student has got to.
ALTER TABLE marks ADD COLUMN semester INT CHECK (semester BETWEEN 1 AND 12);

-- Everything pushed before this split was pushed keyed by PRN: it is the
-- university's semester result.
UPDATE marks SET exam_kind = 'university' WHERE assessment = 'exam';

-- Attribute every existing row to the semester it was earned in: the
-- enrollment for that course where one exists, else the student's semester.
UPDATE marks m
   SET semester = COALESCE(
       (SELECT COALESCE(e.semester, c.semester)
          FROM enrollments e JOIN courses c ON c.id = e.course_id
         WHERE e.student_id = m.student_id AND e.course_id = m.course_id),
       (SELECT semester FROM students WHERE id = m.student_id))
 WHERE m.semester IS NULL;

-- One row per student, course, assessment, kind and exam name: the internal
-- exam and the university exam of the same course sit side by side instead of
-- overwriting each other.
ALTER TABLE marks DROP CONSTRAINT marks_student_id_course_id_assessment_key;
ALTER TABLE marks ADD CONSTRAINT marks_one_per_assessment
    UNIQUE (student_id, course_id, assessment, exam_kind, exam_name);
