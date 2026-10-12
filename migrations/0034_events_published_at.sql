-- Events were created (0003) without a `published_at` column, and 0030 added
-- only `unpublish_at` — but the CMS service layer (create_event, update_event,
-- event_for_edit) has always read and written `events.published_at`, so every
-- event create, edit and publish/archive failed with a missing-column error.
-- Add the column (and the feed index the other content tables carry) so events
-- behave like notices and news: a future `published_at` schedules a publish and
-- `COALESCE(published_at, now())` gates visibility.
--
-- Non-destructive: existing rows get NULL, which reads as "visible now".
ALTER TABLE events ADD COLUMN published_at TIMESTAMPTZ;

CREATE INDEX events_feed_idx ON events (status, published_at DESC);
