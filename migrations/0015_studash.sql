-- Student dashboard upgrade: PRN, per-semester enrollments, exam publishing.
--
-- The university's permanent registration number is a candidate key: the
-- admission number is the login, the PRN is how a candidate is identified
-- when semester exam results are pushed. Nullable until the university
-- assigns one; unique when present.
ALTER TABLE students ADD COLUMN prn TEXT;
CREATE UNIQUE INDEX students_prn_uidx ON students (prn) WHERE prn IS NOT NULL;

-- Which semester an enrollment belongs to, so Results can be read for any
-- semester. Catalogue courses keep courses.semester NULL (the offering
-- decides), so the offering's semester wins for them.
ALTER TABLE enrollments ADD COLUMN semester INT CHECK (semester BETWEEN 1 AND 12);

UPDATE enrollments e
   SET semester = COALESCE(
       c.semester,
       (SELECT o.semester
          FROM student_course_selections s
          JOIN course_offerings o ON o.id = s.offering_id
         WHERE s.student_id = e.student_id
           AND o.course_id = e.course_id
           AND s.state IN ('confirmed', 'locked')
         ORDER BY o.id DESC
         LIMIT 1),
       (SELECT st.semester FROM students st WHERE st.id = e.student_id))
  FROM courses c
 WHERE c.id = e.course_id;

-- Students see the exam timetable between the moment it was pushed and the
-- date of the last exam in that push.
ALTER TABLE exams ADD COLUMN published_at TIMESTAMPTZ;

-- 'exam' rows are the semester exam results pushed by the head of
-- department, kept apart from the coursework rows that already share
-- assessment = 'internal' (one row per assessment per student/course).
ALTER TABLE marks DROP CONSTRAINT marks_assessment_check;
ALTER TABLE marks ADD CONSTRAINT marks_assessment_check
    CHECK (assessment IN ('internal', 'external', 'practical', 'assignment', 'exam'));
