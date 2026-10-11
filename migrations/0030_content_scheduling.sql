-- Scheduled CMS publishing.
--
-- A row's public visibility is decided by one rule across every content type:
--   status IN ('published', 'scheduled')
--   AND COALESCE(published_at, now()) <= now()
--   AND (unpublish_at IS NULL OR unpublish_at > now())
--
-- `published_at` is already the publish time; a future value is how a publish is
-- scheduled. `unpublish_at` is new and how a publish is time-limited. The
-- 'scheduled' status marks a row the editor has explicitly lined up, though the
-- timestamp alone is what actually gates visibility.

ALTER TABLE notices   ADD COLUMN unpublish_at TIMESTAMPTZ;
ALTER TABLE news      ADD COLUMN unpublish_at TIMESTAMPTZ;
ALTER TABLE events    ADD COLUMN unpublish_at TIMESTAMPTZ;
ALTER TABLE documents ADD COLUMN unpublish_at TIMESTAMPTZ;
-- `pages` never carried a publish time; add both halves of the window here.
ALTER TABLE pages     ADD COLUMN published_at TIMESTAMPTZ;
ALTER TABLE pages     ADD COLUMN unpublish_at TIMESTAMPTZ;

ALTER TABLE notices   DROP CONSTRAINT notices_status_check,
    ADD CONSTRAINT notices_status_check   CHECK (status IN ('draft', 'published', 'scheduled', 'archived'));
ALTER TABLE news      DROP CONSTRAINT news_status_check,
    ADD CONSTRAINT news_status_check      CHECK (status IN ('draft', 'published', 'scheduled', 'archived'));
ALTER TABLE events    DROP CONSTRAINT events_status_check,
    ADD CONSTRAINT events_status_check    CHECK (status IN ('draft', 'published', 'scheduled', 'archived'));
ALTER TABLE documents DROP CONSTRAINT documents_status_check,
    ADD CONSTRAINT documents_status_check CHECK (status IN ('draft', 'published', 'scheduled', 'archived'));
ALTER TABLE pages     DROP CONSTRAINT pages_status_check,
    ADD CONSTRAINT pages_status_check     CHECK (status IN ('draft', 'published', 'scheduled', 'archived'));
