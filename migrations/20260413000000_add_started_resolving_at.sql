-- Add started_resolving_at column to workflow_runs
ALTER TABLE workflow_runs ADD COLUMN IF NOT EXISTS started_resolving_at TIMESTAMPTZ;
