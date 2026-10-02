-- Public site content managed from the admin dashboard.

CREATE TABLE uploads (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    path          TEXT        NOT NULL UNIQUE,  -- web path under /uploads, e.g. /uploads/notices/ab12.pdf
    original_name TEXT        NOT NULL,
    mime_type     TEXT        NOT NULL,
    size_bytes    BIGINT      NOT NULL CHECK (size_bytes >= 0),
    uploaded_by   BIGINT      REFERENCES users (id) ON DELETE SET NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE notices (
    id              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    title           TEXT        NOT NULL,
    category        TEXT        NOT NULL DEFAULT 'Notice',
    body            TEXT        NOT NULL DEFAULT '',
    attachment_path TEXT,
    audience        TEXT        NOT NULL DEFAULT 'public' CHECK (audience IN ('public', 'students', 'faculty')),
    is_pinned       BOOLEAN     NOT NULL DEFAULT false,
    status          TEXT        NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'published', 'archived')),
    published_at    TIMESTAMPTZ,
    created_by      BIGINT      REFERENCES users (id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX notices_feed_idx ON notices (status, audience, published_at DESC);

CREATE TABLE news (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    title        TEXT        NOT NULL,
    body         TEXT        NOT NULL DEFAULT '',
    image_path   TEXT,
    status       TEXT        NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'published', 'archived')),
    published_at TIMESTAMPTZ,
    created_by   BIGINT      REFERENCES users (id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX news_feed_idx ON news (status, published_at DESC);

CREATE TABLE events (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    title       TEXT        NOT NULL,
    description TEXT        NOT NULL DEFAULT '',
    location    TEXT,
    starts_at   TIMESTAMPTZ NOT NULL,
    ends_at     TIMESTAMPTZ,
    image_path  TEXT,
    audience    TEXT        NOT NULL DEFAULT 'public' CHECK (audience IN ('public', 'students', 'faculty')),
    status      TEXT        NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'published', 'archived')),
    created_by  BIGINT      REFERENCES users (id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (ends_at IS NULL OR ends_at >= starts_at)
);
CREATE INDEX events_upcoming_idx ON events (status, starts_at);

CREATE TABLE documents (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    title        TEXT        NOT NULL,
    category     TEXT        NOT NULL DEFAULT 'General',
    file_path    TEXT        NOT NULL,
    status       TEXT        NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'published', 'archived')),
    published_at TIMESTAMPTZ,
    created_by   BIGINT      REFERENCES users (id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX documents_feed_idx ON documents (status, category, published_at DESC);

CREATE TABLE rank_holders (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name          TEXT        NOT NULL,
    rank_position INT         NOT NULL CHECK (rank_position > 0),
    department_id BIGINT      REFERENCES departments (id) ON DELETE SET NULL,
    exam_year     INT         NOT NULL,
    photo_path    TEXT,
    sort_order    INT         NOT NULL DEFAULT 0,
    status        TEXT        NOT NULL DEFAULT 'published' CHECK (status IN ('draft', 'published', 'archived')),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- NSS, NCC, and other clubs and cells.
CREATE TABLE clubs (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    slug        TEXT        NOT NULL UNIQUE,
    short_name  TEXT        NOT NULL,
    full_name   TEXT        NOT NULL,
    summary     TEXT        NOT NULL DEFAULT '',
    values_text TEXT        NOT NULL DEFAULT '',
    image_path  TEXT,
    featured    BOOLEAN     NOT NULL DEFAULT false,
    sort_order  INT         NOT NULL DEFAULT 0,
    status      TEXT        NOT NULL DEFAULT 'published' CHECK (status IN ('draft', 'published', 'archived')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE facilities (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name        TEXT        NOT NULL,
    description TEXT        NOT NULL DEFAULT '',
    image_path  TEXT,
    sort_order  INT         NOT NULL DEFAULT 0,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE milestones (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    when_label  TEXT        NOT NULL,
    description TEXT        NOT NULL,
    sort_order  INT         NOT NULL DEFAULT 0,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Informational pages (about, admissions, IQAC, ...). Served by path.
CREATE TABLE pages (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    path       TEXT        NOT NULL UNIQUE CHECK (path LIKE '/%' AND path <> '/'),
    title      TEXT        NOT NULL,
    lede       TEXT        NOT NULL DEFAULT '',
    status     TEXT        NOT NULL DEFAULT 'published' CHECK (status IN ('draft', 'published', 'archived')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE page_sections (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    page_id    BIGINT      NOT NULL REFERENCES pages (id) ON DELETE CASCADE,
    heading    TEXT        NOT NULL,
    body       TEXT        NOT NULL DEFAULT '',  -- paragraphs separated by a blank line
    sort_order INT         NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX page_sections_page_idx ON page_sections (page_id, sort_order);

CREATE TABLE site_settings (
    key        TEXT PRIMARY KEY,
    value      TEXT        NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
