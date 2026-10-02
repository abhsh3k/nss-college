-- Core identity and academic structure.

CREATE OR REPLACE FUNCTION set_updated_at() RETURNS trigger AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TABLE users (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    email         TEXT        NOT NULL,
    password_hash TEXT        NOT NULL,
    role          TEXT        NOT NULL CHECK (role IN ('student', 'faculty', 'staff', 'admin', 'alumni')),
    is_active     BOOLEAN     NOT NULL DEFAULT true,
    last_login_at TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX users_email_lower_key ON users (lower(email));

CREATE TABLE departments (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    slug       TEXT        NOT NULL UNIQUE,
    name       TEXT        NOT NULL,
    summary    TEXT        NOT NULL DEFAULT '',
    sort_order INT         NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE programmes (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    department_id BIGINT      NOT NULL REFERENCES departments (id) ON DELETE RESTRICT,
    slug          TEXT        NOT NULL UNIQUE,
    name          TEXT        NOT NULL,
    level         TEXT        NOT NULL,
    summary       TEXT        NOT NULL DEFAULT '',
    sort_order    INT         NOT NULL DEFAULT 0,
    status        TEXT        NOT NULL DEFAULT 'published' CHECK (status IN ('draft', 'published', 'archived')),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX programmes_department_idx ON programmes (department_id);

CREATE TABLE faculty (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id       BIGINT UNIQUE REFERENCES users (id) ON DELETE SET NULL,
    department_id BIGINT      REFERENCES departments (id) ON DELETE SET NULL,
    name          TEXT        NOT NULL,
    designation   TEXT        NOT NULL DEFAULT '',
    qualification TEXT        NOT NULL DEFAULT '',
    email         TEXT,
    photo_path    TEXT,
    bio           TEXT        NOT NULL DEFAULT '',
    sort_order    INT         NOT NULL DEFAULT 0,
    status        TEXT        NOT NULL DEFAULT 'published' CHECK (status IN ('draft', 'published', 'archived')),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX faculty_department_idx ON faculty (department_id);

CREATE TABLE students (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id      BIGINT UNIQUE REFERENCES users (id) ON DELETE SET NULL,
    admission_no TEXT        NOT NULL UNIQUE,
    name         TEXT        NOT NULL,
    programme_id BIGINT      NOT NULL REFERENCES programmes (id) ON DELETE RESTRICT,
    batch_year   INT         NOT NULL,
    semester     INT         NOT NULL DEFAULT 1 CHECK (semester BETWEEN 1 AND 12),
    phone        TEXT,
    photo_path   TEXT,
    is_active    BOOLEAN     NOT NULL DEFAULT true,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX students_programme_idx ON students (programme_id, semester);

CREATE TABLE courses (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    programme_id BIGINT      NOT NULL REFERENCES programmes (id) ON DELETE CASCADE,
    faculty_id   BIGINT      REFERENCES faculty (id) ON DELETE SET NULL,
    code         TEXT        NOT NULL,
    title        TEXT        NOT NULL,
    semester     INT         NOT NULL CHECK (semester BETWEEN 1 AND 12),
    credits      INT         NOT NULL DEFAULT 0,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (programme_id, code)
);
CREATE INDEX courses_programme_sem_idx ON courses (programme_id, semester);
