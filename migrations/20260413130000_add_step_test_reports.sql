-- Migration to add test reports persistence

CREATE TABLE step_test_reports (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id UUID NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    step_instance_id UUID NOT NULL REFERENCES step_instances(id) ON DELETE CASCADE,
    report_name TEXT NOT NULL,
    file_name TEXT NOT NULL,
    format TEXT NOT NULL,
    content TEXT NOT NULL,
    checksum TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_step_test_reports_run_id ON step_test_reports(run_id);
CREATE INDEX idx_step_test_reports_step_instance_id ON step_test_reports(step_instance_id);
