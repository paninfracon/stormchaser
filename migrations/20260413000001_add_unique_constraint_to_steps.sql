-- Add unique constraint to prevent duplicate scheduling of the same step iteration within a run
CREATE UNIQUE INDEX idx_step_instances_run_step_iter ON step_instances (run_id, step_name, COALESCE(iteration_index, -1));
