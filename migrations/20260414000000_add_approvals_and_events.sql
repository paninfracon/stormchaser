-- Add tables for Human-in-the-Loop Approvals and Event Waiting

CREATE TABLE IF NOT EXISTS event_correlations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    step_instance_id UUID NOT NULL REFERENCES step_instances(id) ON DELETE CASCADE,
    run_id UUID NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    correlation_key TEXT NOT NULL,
    correlation_value TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_event_correlations_lookup ON event_correlations(correlation_key, correlation_value);

CREATE TABLE IF NOT EXISTS approval_registry (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    step_instance_id UUID NOT NULL REFERENCES step_instances(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    status TEXT NOT NULL, -- 'approved' or 'rejected'
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_approval_registry_step ON approval_registry(step_instance_id);
