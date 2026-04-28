-- Migration to add individual test cases tracking

CREATE TYPE test_case_status AS ENUM ('passed', 'failed', 'skipped', 'error');

CREATE TABLE step_test_cases (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id UUID NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    step_instance_id UUID NOT NULL REFERENCES step_instances(id) ON DELETE CASCADE,
    report_name TEXT NOT NULL,
    test_suite TEXT,
    test_case TEXT NOT NULL,
    status test_case_status NOT NULL,
    duration_ms BIGINT,
    message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_step_test_cases_run_id ON step_test_cases(run_id);
CREATE INDEX idx_step_test_cases_step_instance_id ON step_test_cases(step_instance_id);

-- Archived table
CREATE TABLE archived_step_test_cases (
    id UUID PRIMARY KEY,
    run_id UUID NOT NULL,
    step_instance_id UUID NOT NULL,
    report_name TEXT NOT NULL,
    test_suite TEXT,
    test_case TEXT NOT NULL,
    status test_case_status NOT NULL,
    duration_ms BIGINT,
    message TEXT,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_archived_step_test_cases_run_id ON archived_step_test_cases(run_id);
