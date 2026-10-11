-- In-app notifications: an actionable feed per user for approvals, corrections,
-- handovers and other events. A notification carries a link back to the page
-- that owns the decision; opening that page always re-checks authorization, so
-- a notification never itself grants access.

CREATE TABLE notifications (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id     BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind        TEXT   NOT NULL DEFAULT 'general'
                CHECK (kind IN ('appointment', 'correction', 'import', 'handover', 'deadline', 'general')),
    title       TEXT   NOT NULL,
    body        TEXT   NOT NULL DEFAULT '',
    -- The relative path the item opens; re-authorised when it is followed.
    href        TEXT   NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    read_at     TIMESTAMPTZ
);

CREATE INDEX notifications_user_idx
    ON notifications (user_id, read_at, created_at DESC);
