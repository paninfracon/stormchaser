-- Migration to add combined views for active and archived data

-- 1. Combined Workflow Runs
CREATE VIEW combined_workflow_runs AS
SELECT id, workflow_name, status, version, fencing_token, initiating_user, repo_url, workflow_path, git_ref, created_at, updated_at, started_resolving_at, started_at, finished_at, error
FROM workflow_runs
UNION ALL
SELECT id, workflow_name, status, version, fencing_token, initiating_user, repo_url, workflow_path, git_ref, created_at, updated_at, started_resolving_at, started_at, finished_at, error
FROM archived_workflow_runs;

-- 2. Combined Run Contexts
CREATE VIEW combined_run_contexts AS
SELECT run_id, dsl_version, workflow_definition, inputs, secrets, sensitive_values, source_code
FROM run_contexts
UNION ALL
SELECT run_id, dsl_version, workflow_definition, inputs, secrets, sensitive_values, source_code
FROM archived_run_contexts;

-- 3. Combined Run Details (joining runs and contexts)
CREATE VIEW combined_run_details AS
SELECT
    wr.id, wr.workflow_name, wr.initiating_user, wr.repo_url, wr.workflow_path, wr.git_ref,
    wr.status, wr.version, wr.created_at, wr.updated_at, wr.started_resolving_at, wr.started_at, wr.finished_at, wr.error,
    rc.inputs, rc.secrets, rc.source_code, rc.dsl_version
FROM combined_workflow_runs wr
JOIN combined_run_contexts rc ON wr.id = rc.run_id;

-- 4. Combined Step Instances
CREATE VIEW combined_step_instances AS
SELECT id, run_id, step_name, step_type, status, iteration_index, runner_id, affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at
FROM step_instances
UNION ALL
SELECT id, run_id, step_name, step_type, status, iteration_index, runner_id, affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at
FROM archived_step_instances;

-- 5. Combined Step Outputs
CREATE VIEW combined_step_outputs AS
SELECT step_instance_id, key, value, is_sensitive
FROM step_outputs
UNION ALL
SELECT step_instance_id, key, value, is_sensitive
FROM archived_step_outputs;
