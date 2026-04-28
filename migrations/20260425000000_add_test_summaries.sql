-- Migration to add test summaries and update test reports for S3 references

-- 1. Create step_test_summaries table
CREATE TABLE step_test_summaries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id UUID NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    step_instance_id UUID NOT NULL REFERENCES step_instances(id) ON DELETE CASCADE,
    report_name TEXT NOT NULL,
    total_tests INTEGER NOT NULL DEFAULT 0,
    passed INTEGER NOT NULL DEFAULT 0,
    failed INTEGER NOT NULL DEFAULT 0,
    skipped INTEGER NOT NULL DEFAULT 0,
    errors INTEGER NOT NULL DEFAULT 0,
    duration_ms BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_step_test_summaries_run_id ON step_test_summaries(run_id);
CREATE INDEX idx_step_test_summaries_step_instance_id ON step_test_summaries(step_instance_id);

-- Create archived_step_test_summaries table
CREATE TABLE archived_step_test_summaries (
    id UUID PRIMARY KEY,
    run_id UUID NOT NULL,
    step_instance_id UUID NOT NULL,
    report_name TEXT NOT NULL,
    total_tests INTEGER NOT NULL,
    passed INTEGER NOT NULL,
    failed INTEGER NOT NULL,
    skipped INTEGER NOT NULL,
    errors INTEGER NOT NULL,
    duration_ms BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_archived_step_test_summaries_run_id ON archived_step_test_summaries(run_id);

-- 2. Update step_test_reports to support S3 references and make content optional
ALTER TABLE step_test_reports ALTER COLUMN content DROP NOT NULL;
ALTER TABLE step_test_reports ADD COLUMN backend_id UUID REFERENCES storage_backends(id);
ALTER TABLE step_test_reports ADD COLUMN remote_path TEXT;

-- 3. Update archived_step_test_reports as well
ALTER TABLE archived_step_test_reports ALTER COLUMN content DROP NOT NULL;
ALTER TABLE archived_step_test_reports ADD COLUMN backend_id UUID;
ALTER TABLE archived_step_test_reports ADD COLUMN remote_path TEXT;
