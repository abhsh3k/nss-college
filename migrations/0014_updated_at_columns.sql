-- 0013 attached the shared set_updated_at trigger to every table it created,
-- but three of them were declared without the updated_at column the trigger
-- assigns, so any UPDATE on their rows fails inside PL/pgSQL (re-addressing a
-- cohort's batch year hits this at once). Give them the column every other
-- table has; the trigger then behaves the same everywhere.
ALTER TABLE academic_years ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE cohorts ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE course_offering_targets ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
