-- Correlation ids for the audit log.
--
-- Every audit row written while handling one HTTP request shares a
-- `correlation_id`, set by the middleware in `src/correlation.rs`. That is what
-- lets an operator tie together the several rows a single action can produce
-- (for example an appointment request that also notifies the Principal),
-- without trusting anything the client sent. The value is generated
-- server-side per request; it is not a secret and decodes to nothing.

ALTER TABLE audit_log ADD COLUMN correlation_id TEXT;

-- Operators search by correlation id when reconstructing an incident.
CREATE INDEX audit_log_correlation_idx ON audit_log (correlation_id)
    WHERE correlation_id IS NOT NULL;
