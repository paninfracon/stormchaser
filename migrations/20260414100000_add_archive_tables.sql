-- Migration to add archive tables for completed workflows

-- Archived workflow runs
CREATE TABLE archived_workflow_runs (
    id UUID PRIMARY KEY,
    workflow_name TEXT NOT NULL,
    status run_status NOT NULL,
    version INTEGER NOT NULL,
    fencing_token BIGINT NOT NULL,
    initiating_user TEXT NOT NULL,
    repo_url TEXT NOT NULL,
    workflow_path TEXT NOT NULL,
    git_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    started_resolving_at TIMESTAMPTZ,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    error TEXT
);

CREATE INDEX idx_archived_workflow_runs_name ON archived_workflow_runs(workflow_name);
CREATE INDEX idx_archived_workflow_runs_status ON archived_workflow_runs(status);
CREATE INDEX idx_archived_workflow_runs_created_at ON archived_workflow_runs(created_at);

-- Archived run contexts
CREATE TABLE archived_run_contexts (
    run_id UUID PRIMARY KEY REFERENCES archived_workflow_runs(id) ON DELETE CASCADE,
    dsl_version TEXT NOT NULL,
    workflow_definition JSONB NOT NULL,
    inputs JSONB NOT NULL,
    sensitive_values TEXT[] NOT NULL,
    source_code TEXT NOT NULL
);

-- Archived run quotas
CREATE TABLE archived_run_quotas (
    run_id UUID PRIMARY KEY REFERENCES archived_workflow_runs(id) ON DELETE CASCADE,
    max_concurrency INTEGER NOT NULL,
    max_cpu TEXT NOT NULL,
    max_memory TEXT NOT NULL,
    max_storage TEXT NOT NULL,
    timeout TEXT NOT NULL,
    current_cpu_usage DOUBLE PRECISION NOT NULL,
    current_memory_usage TEXT NOT NULL
);

-- Archived audit logs
CREATE TABLE archived_audit_logs (
    id BIGINT PRIMARY KEY, -- Keep same ID
    run_id UUID NOT NULL REFERENCES archived_workflow_runs(id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    actor TEXT NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_archived_audit_logs_run_id ON archived_audit_logs(run_id);

-- Archived step instances
CREATE TABLE archived_step_instances (
    id UUID PRIMARY KEY,
    run_id UUID NOT NULL REFERENCES archived_workflow_runs(id) ON DELETE CASCADE,
    step_name TEXT NOT NULL,
    step_type TEXT NOT NULL,
    status step_status NOT NULL,
    iteration_index INTEGER,
    runner_id TEXT,
    affinity_context TEXT,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    exit_code INTEGER,
    error TEXT,
    spec JSONB,
    params JSONB
);

CREATE INDEX idx_archived_step_instances_run_id ON archived_step_instances(run_id);

-- Archived step outputs
CREATE TABLE archived_step_outputs (
    step_instance_id UUID NOT NULL REFERENCES archived_step_instances(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    value JSONB NOT NULL,
    is_sensitive BOOLEAN NOT NULL,
    PRIMARY KEY (step_instance_id, key)
);

-- Archived step test reports
CREATE TABLE archived_step_test_reports (
    id UUID PRIMARY KEY,
    run_id UUID NOT NULL REFERENCES archived_workflow_runs(id) ON DELETE CASCADE,
    step_instance_id UUID NOT NULL REFERENCES archived_step_instances(id) ON DELETE CASCADE,
    report_name TEXT NOT NULL,
    file_name TEXT NOT NULL,
    format TEXT NOT NULL,
    content TEXT NOT NULL,
    checksum TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_archived_step_test_reports_run_id ON archived_step_test_reports(run_id);

-- Archived event correlations
CREATE TABLE archived_event_correlations (
    id UUID PRIMARY KEY,
    step_instance_id UUID NOT NULL REFERENCES archived_step_instances(id) ON DELETE CASCADE,
    run_id UUID NOT NULL REFERENCES archived_workflow_runs(id) ON DELETE CASCADE,
    correlation_key TEXT NOT NULL,
    correlation_value TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

-- Archived approval registry
CREATE TABLE archived_approval_registry (
    id UUID PRIMARY KEY,
    step_instance_id UUID NOT NULL REFERENCES archived_step_instances(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    status TEXT NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

-- Archived artifact registry
CREATE TABLE archived_artifact_registry (
    id UUID PRIMARY KEY,
    run_id UUID NOT NULL REFERENCES archived_workflow_runs(id) ON DELETE CASCADE,
    step_instance_id UUID NOT NULL REFERENCES archived_step_instances(id) ON DELETE CASCADE,
    artifact_name TEXT NOT NULL,
    backend_id UUID NOT NULL REFERENCES storage_backends(id),
    remote_path TEXT NOT NULL,
    metadata JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_archived_artifact_registry_run_id ON archived_artifact_registry(run_id);

-- Archived run storage states
CREATE TABLE archived_run_storage_states (
    run_id UUID NOT NULL REFERENCES archived_workflow_runs(id) ON DELETE CASCADE,
    storage_name TEXT NOT NULL,
    last_hash TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (run_id, storage_name)
);
