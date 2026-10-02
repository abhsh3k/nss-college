-- Student Hub data: timetable, attendance, exams, marks.

CREATE TABLE timetable_entries (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    programme_id BIGINT      NOT NULL REFERENCES programmes (id) ON DELETE CASCADE,
    semester     INT         NOT NULL CHECK (semester BETWEEN 1 AND 12),
    course_id    BIGINT      NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    faculty_id   BIGINT      REFERENCES faculty (id) ON DELETE SET NULL,
    weekday      SMALLINT    NOT NULL CHECK (weekday BETWEEN 1 AND 7), -- 1 = Monday
    start_time   TIME        NOT NULL,
    end_time     TIME        NOT NULL,
    room         TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (end_time > start_time)
);
CREATE INDEX timetable_lookup_idx ON timetable_entries (programme_id, semester, weekday, start_time);

CREATE TABLE attendance (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    student_id  BIGINT      NOT NULL REFERENCES students (id) ON DELETE CASCADE,
    course_id   BIGINT      NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    on_date     DATE        NOT NULL,
    status      TEXT        NOT NULL CHECK (status IN ('present', 'absent', 'leave')),
    marked_by   BIGINT      REFERENCES users (id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (student_id, course_id, on_date)
);
CREATE INDEX attendance_student_idx ON attendance (student_id, on_date);

CREATE TABLE exams (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name         TEXT        NOT NULL,
    programme_id BIGINT      NOT NULL REFERENCES programmes (id) ON DELETE CASCADE,
    semester     INT         NOT NULL CHECK (semester BETWEEN 1 AND 12),
    course_id    BIGINT      NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    exam_date    DATE        NOT NULL,
    start_time   TIME,
    end_time     TIME,
    venue        TEXT,
    status       TEXT        NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'published', 'archived')),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX exams_lookup_idx ON exams (programme_id, semester, exam_date);

CREATE TABLE marks (
    id              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    student_id      BIGINT        NOT NULL REFERENCES students (id) ON DELETE CASCADE,
    course_id       BIGINT        NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    exam_id         BIGINT        REFERENCES exams (id) ON DELETE SET NULL,
    assessment      TEXT          NOT NULL CHECK (assessment IN ('internal', 'external', 'practical', 'assignment')),
    marks_obtained  NUMERIC(6, 2) NOT NULL CHECK (marks_obtained >= 0),
    max_marks       NUMERIC(6, 2) NOT NULL CHECK (max_marks > 0),
    published       BOOLEAN       NOT NULL DEFAULT false,
    created_at      TIMESTAMPTZ   NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ   NOT NULL DEFAULT now(),
    UNIQUE (student_id, course_id, assessment),
    CHECK (marks_obtained <= max_marks)
);
CREATE INDEX marks_student_idx ON marks (student_id);

CREATE TABLE study_materials (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    course_id   BIGINT      NOT NULL REFERENCES courses (id) ON DELETE CASCADE,
    title       TEXT        NOT NULL,
    file_path   TEXT        NOT NULL,
    uploaded_by BIGINT      REFERENCES users (id) ON DELETE SET NULL,
    status      TEXT        NOT NULL DEFAULT 'published' CHECK (status IN ('draft', 'published', 'archived')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX study_materials_course_idx ON study_materials (course_id);
