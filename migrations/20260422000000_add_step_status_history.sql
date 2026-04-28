-- Migration to add step status history tracking

CREATE TABLE step_status_history (
    id BIGSERIAL PRIMARY KEY,
    step_instance_id UUID NOT NULL REFERENCES step_instances(id) ON DELETE CASCADE,
    status step_status NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_step_status_history_step_instance_id ON step_status_history(step_instance_id);

-- Create archived version
CREATE TABLE archived_step_status_history (
    id BIGINT PRIMARY KEY,
    step_instance_id UUID NOT NULL,
    status step_status NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_archived_step_status_history_step_instance_id ON archived_step_status_history(step_instance_id);

-- Add to combined views
CREATE VIEW combined_step_status_history AS
SELECT id, step_instance_id, status, created_at
FROM step_status_history
UNION ALL
SELECT id, step_instance_id, status, created_at
FROM archived_step_status_history;
