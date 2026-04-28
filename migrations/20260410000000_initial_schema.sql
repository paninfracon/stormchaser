-- Initial schema for Stormchaser Orchestration Engine

-- Enums for Run and Step status
CREATE TYPE run_status AS ENUM (
    'queued',
    'running',
    'succeeded',
    'failed',
    'aborted'
);

CREATE TYPE step_status AS ENUM (
    'pending',
    'running',
    'succeeded',
    'failed',
    'failed_ignored',
    'skipped',
    'waiting_for_event'
);

-- Workflow runs table
CREATE TABLE workflow_runs (
    id UUID PRIMARY KEY,
    workflow_name TEXT NOT NULL,
    status run_status NOT NULL DEFAULT 'queued',
    version INTEGER NOT NULL DEFAULT 1,
    fencing_token BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    error TEXT
);

-- Index for workflow name and status
CREATE INDEX idx_workflow_runs_name ON workflow_runs(workflow_name);
CREATE INDEX idx_workflow_runs_status ON workflow_runs(status);

-- Workflow run context table (DSL definition and inputs)
CREATE TABLE run_contexts (
    run_id UUID PRIMARY KEY REFERENCES workflow_runs(id) ON DELETE CASCADE,
    dsl_version TEXT NOT NULL,
    workflow_definition JSONB NOT NULL,
    inputs JSONB NOT NULL DEFAULT '{}',
    sensitive_values TEXT[] NOT NULL DEFAULT '{}'
);

-- Workflow run quotas and usage
CREATE TABLE run_quotas (
    run_id UUID PRIMARY KEY REFERENCES workflow_runs(id) ON DELETE CASCADE,
    max_concurrency INTEGER NOT NULL,
    max_cpu TEXT NOT NULL,
    max_memory TEXT NOT NULL,
    max_storage TEXT NOT NULL,
    timeout TEXT NOT NULL,
    current_cpu_usage DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    current_memory_usage TEXT NOT NULL DEFAULT '0'
);

-- Audit log for workflow events
CREATE TABLE audit_logs (
    id BIGSERIAL PRIMARY KEY,
    run_id UUID NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    actor TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_audit_logs_run_id ON audit_logs(run_id);

-- Step instances table
CREATE TABLE step_instances (
    id UUID PRIMARY KEY,
    run_id UUID NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    step_name TEXT NOT NULL,
    step_type TEXT NOT NULL,
    status step_status NOT NULL DEFAULT 'pending',
    iteration_index INTEGER,
    runner_id TEXT,
    affinity_context TEXT,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    exit_code INTEGER,
    error TEXT
);

CREATE INDEX idx_step_instances_run_id ON step_instances(run_id);
CREATE INDEX idx_step_instances_status ON step_instances(status);

-- Step outputs table
CREATE TABLE step_outputs (
    step_instance_id UUID NOT NULL REFERENCES step_instances(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    value JSONB NOT NULL,
    is_sensitive BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (step_instance_id, key)
);

CREATE INDEX idx_step_outputs_step_instance_id ON step_outputs(step_instance_id);
