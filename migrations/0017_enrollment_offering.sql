-- Which course offering produced each enrollment.
--
-- Until now an enrollment only knew its `course_id`, so an offering period's
-- seat count had to guess "students of this course in this semester" — which
-- counts every student ever enrolled in the underlying course, across every
-- offering of it. Making the link explicit lets the attendance roster, the
-- seat count and the capacity check all ask one question: who is enrolled in
-- *this* offering?
ALTER TABLE enrollments ADD COLUMN offering_id BIGINT REFERENCES course_offerings (id) ON DELETE SET NULL;
CREATE INDEX enrollments_offering_idx ON enrollments (offering_id) WHERE offering_id IS NOT NULL;

-- Back-fill from the confirmed (or locked) selection that created each
-- enrollment, matched through the offering's course. A student with two such
-- selections of the same course takes the most recently confirmed one — the
-- same "latest confirmed selection wins" rule migration 0015 used for the
-- semester column.
UPDATE enrollments e
   SET offering_id = (
       SELECT s.offering_id
         FROM student_course_selections s
         JOIN course_offerings o ON o.id = s.offering_id
        WHERE s.student_id = e.student_id
          AND o.course_id = e.course_id
          AND s.state IN ('confirmed', 'locked')
        ORDER BY s.confirmed_at DESC NULLS LAST, s.id DESC
        LIMIT 1)
 WHERE e.offering_id IS NULL
   AND EXISTS (
       SELECT 1
         FROM student_course_selections s
         JOIN course_offerings o ON o.id = s.offering_id
        WHERE s.student_id = e.student_id
          AND o.course_id = e.course_id
          AND s.state IN ('confirmed', 'locked'));
