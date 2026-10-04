-- Head-of-department powers.
--
-- `faculty.is_hod` already existed but nothing acted on it. This adds the
-- second half of the picture: `can_manage`, for teachers the IT admin has
-- approved to act for the HOD, and a real department on `students` so an HOD
-- can place a student in a department rather than inferring it from the
-- programme.

-- A teacher the IT admin has approved to manage on the HOD's behalf.
-- `is_hod` is the head; `can_manage` is a delegate. Both grant the same
-- department-scoped powers, so an HOD can hand cover to a colleague.
ALTER TABLE faculty ADD COLUMN can_manage BOOLEAN NOT NULL DEFAULT false;

-- A student's department, set independently of their programme. Left NULL
-- where it is unknown; `services::people` falls back to the programme's
-- department so existing students keep a sensible value everywhere.
ALTER TABLE students
    ADD COLUMN department_id BIGINT REFERENCES departments (id) ON DELETE SET NULL;

CREATE INDEX students_department_idx ON students (department_id);

-- Backfill from the programme so no student is left without a department:
-- a student in a BCA programme belongs to that programme's department.
UPDATE students s
   SET department_id = p.department_id
  FROM programmes p
 WHERE p.id = s.programme_id
   AND s.department_id IS NULL
   AND p.department_id IS NOT NULL;