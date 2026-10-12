-- Timetable teacher integrity.
--
-- A period used to be creatable with `faculty_id` left NULL, which meant
-- "unassigned" only because a column happened to be empty: nothing recorded
-- that a teacher was still owed, and nothing surfaced the gap. The forms now
-- require a validated teacher, *or* an explicit "teacher to be assigned" mark,
-- so a period can never be silently incomplete — it is either taught by
-- somebody real, or it is queued as work to be resolved.
--
-- The table constraint makes that a database invariant rather than a route
-- convention: exactly one of the two states holds.

ALTER TABLE timetable_entries ADD COLUMN teacher_tbd BOOLEAN NOT NULL DEFAULT false;

-- Existing rows with no teacher are, by definition, the outstanding work the
-- new state describes. Marking them keeps the constraint below satisfiable
-- without fabricating a teacher for a period that never had one.
UPDATE timetable_entries SET teacher_tbd = true WHERE faculty_id IS NULL;

ALTER TABLE timetable_entries ADD CONSTRAINT timetable_entries_teacher_check
    CHECK (teacher_tbd = (faculty_id IS NULL));

-- The work queue reads the outstanding periods by department, so index the
-- flag rather than scanning the whole table on every page load.
CREATE INDEX timetable_entries_tbd_idx ON timetable_entries (teacher_tbd)
    WHERE teacher_tbd;
