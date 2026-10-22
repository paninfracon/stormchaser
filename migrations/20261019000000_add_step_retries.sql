-- Step retries. A step with a `retry` policy that fails is put back to pending rather
-- than failed: `attempt` counts its runs, the first being 1, and `retry_at` is when it
-- may next be dispatched. Dispatch clears `retry_at` when it claims the step.
ALTER TABLE step_instances
    ADD COLUMN attempt INTEGER NOT NULL DEFAULT 1,
    ADD COLUMN retry_at TIMESTAMPTZ;

ALTER TABLE archived_step_instances
    ADD COLUMN attempt INTEGER NOT NULL DEFAULT 1;

-- The retry worker looks for steps whose wait is over
CREATE INDEX idx_step_instances_retry_at ON step_instances (retry_at)
    WHERE retry_at IS NOT NULL;
