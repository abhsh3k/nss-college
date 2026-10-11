-- Result correction workflow: a finalized mark is locked, and changing it
-- requires a reasoned correction that is proposed by one authorized manager
-- and approved by another. The proposed values live on the row while the
-- correction is pending; applying it snapshots the old values into
-- mark_history before overwriting the live row.

ALTER TABLE marks
    ADD COLUMN correction_proposed_obtained NUMERIC(6, 2),
    ADD COLUMN correction_proposed_max      NUMERIC(6, 2),
    ADD COLUMN correction_requested_by      BIGINT REFERENCES users (id) ON DELETE SET NULL,
    ADD COLUMN correction_requested_at      TIMESTAMPTZ,
    ADD COLUMN correction_decided_by        BIGINT REFERENCES users (id) ON DELETE SET NULL,
    ADD COLUMN correction_decided_at        TIMESTAMPTZ;

-- A pending correction must carry the value it proposes; a row at rest must
-- not. The constraint is what keeps `correction_pending` from being set
-- without a request behind it (and vice versa).
ALTER TABLE marks
    ADD CONSTRAINT marks_correction_pending_iff_proposed CHECK (
        (workflow_state = 'correction_pending')
        = (correction_proposed_obtained IS NOT NULL)
    );

CREATE INDEX marks_correction_pending_idx
    ON marks (correction_requested_at DESC)
    WHERE workflow_state = 'correction_pending';
