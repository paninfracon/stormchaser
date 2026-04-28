-- Add created_at to step_instances for concurrency ordering
ALTER TABLE step_instances ADD COLUMN created_at TIMESTAMPTZ NOT NULL DEFAULT NOW();
CREATE INDEX idx_step_instances_created_at ON step_instances(created_at);

-- Add created_at to archived_step_instances
ALTER TABLE archived_step_instances ADD COLUMN created_at TIMESTAMPTZ NOT NULL DEFAULT NOW();
