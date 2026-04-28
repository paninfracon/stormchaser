use sqlx::PgConnection;
use uuid::Uuid;

pub async fn archive_and_delete_workflow_run(
    conn: &mut PgConnection,
    run_id: Uuid,
) -> Result<(), sqlx::Error> {
    // 1. Move to archived_workflow_runs
    sqlx::query(
        r#"
        INSERT INTO archived_workflow_runs (
            id, workflow_name, status, version, fencing_token, created_at, updated_at,
            started_at, finished_at, error, repo_url, workflow_path, git_ref, initiating_user
        )
        SELECT
            id, workflow_name, status, version, fencing_token, created_at, updated_at,
            started_at, finished_at, error, repo_url, workflow_path, git_ref, initiating_user
        FROM workflow_runs WHERE id = $1
        "#,
    )
    .bind(run_id)
    .execute(&mut *conn)
    .await?;

    // 2. Move contexts
    sqlx::query(
        r#"
        INSERT INTO archived_run_contexts (
            run_id, dsl_version, workflow_definition, inputs, secrets, sensitive_values, source_code
        )
        SELECT
            run_id, dsl_version, workflow_definition, inputs, secrets, sensitive_values, source_code
        FROM run_contexts WHERE run_id = $1
        "#,
    )
    .bind(run_id)
    .execute(&mut *conn)
    .await?;

    // 3. Move Step Status History (MUST BE BEFORE STEPS DUE TO ON DELETE CASCADE)
    sqlx::query(
        r#"
        INSERT INTO archived_step_status_history (
            id, step_instance_id, status, created_at
        )
        SELECT
            h.id, h.step_instance_id, h.status, h.created_at
        FROM step_status_history h
        JOIN step_instances si ON h.step_instance_id = si.id
        WHERE si.run_id = $1
        "#,
    )
    .bind(run_id)
    .execute(&mut *conn)
    .await?;

    // 4. Move Step Instances
    sqlx::query(
        r#"
        INSERT INTO archived_step_instances (
            id, run_id, step_name, step_type, status, iteration_index, runner_id,
            affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at
        )
        SELECT
            id, run_id, step_name, step_type, status, iteration_index, runner_id,
            affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at
        FROM step_instances WHERE run_id = $1
        "#,
    )
    .bind(run_id)
    .execute(&mut *conn)
    .await?;

    // 5. Move Step Outputs
    sqlx::query(
        r#"
        INSERT INTO archived_step_outputs (
            step_instance_id, key, value, is_sensitive
        )
        SELECT
            o.step_instance_id, o.key, o.value, o.is_sensitive
        FROM step_outputs o
        JOIN step_instances si ON o.step_instance_id = si.id
        WHERE si.run_id = $1
        "#,
    )
    .bind(run_id)
    .execute(&mut *conn)
    .await?;

    // 6. Move Test Reports
    sqlx::query(
        r#"
        INSERT INTO archived_step_test_reports (
            id, run_id, step_instance_id, report_name, file_name, format, content, checksum, created_at, backend_id, remote_path
        )
        SELECT
            r.id, si.run_id, r.step_instance_id, r.report_name, r.file_name, r.format, r.content, r.checksum, r.created_at, r.backend_id, r.remote_path
        FROM step_test_reports r
        JOIN step_instances si ON r.step_instance_id = si.id
        WHERE si.run_id = $1
        "#,
    )
    .bind(run_id)
    .execute(&mut *conn)
    .await?;

    // 6b. Move Test Summaries
    sqlx::query(
        r#"
        INSERT INTO archived_step_test_summaries (
            id, run_id, step_instance_id, report_name, total_tests, passed, failed, skipped, errors, duration_ms, created_at
        )
        SELECT
            s.id, si.run_id, s.step_instance_id, s.report_name, s.total_tests, s.passed, s.failed, s.skipped, s.errors, s.duration_ms, s.created_at
        FROM step_test_summaries s
        JOIN step_instances si ON s.step_instance_id = si.id
        WHERE si.run_id = $1
        "#,
    )
    .bind(run_id)
    .execute(&mut *conn)
    .await?;

    // 6c. Move Test Cases
    sqlx::query(
        r#"
        INSERT INTO archived_step_test_cases (
            id, run_id, step_instance_id, report_name, test_suite, test_case, status, duration_ms, message, created_at
        )
        SELECT
            c.id, si.run_id, c.step_instance_id, c.report_name, c.test_suite, c.test_case, c.status, c.duration_ms, c.message, c.created_at
        FROM step_test_cases c
        JOIN step_instances si ON c.step_instance_id = si.id
        WHERE si.run_id = $1
        "#,
    )
    .bind(run_id)
    .execute(&mut *conn)
    .await?;

    // 7. Move Artifacts
    sqlx::query(
        r#"
        INSERT INTO archived_artifact_registry (
            id, run_id, step_instance_id, artifact_name, backend_id, remote_path, metadata, created_at
        )
        SELECT
            id, run_id, step_instance_id, artifact_name, backend_id, remote_path, metadata, created_at
        FROM artifact_registry WHERE run_id = $1
        "#,
    )
    .bind(run_id)
    .execute(&mut *conn)
    .await?;

    // 8. Move Storage States
    sqlx::query(
        r#"
        INSERT INTO archived_run_storage_states (
            run_id, storage_name, last_hash, updated_at
        )
        SELECT
            run_id, storage_name, last_hash, updated_at
        FROM run_storage_states WHERE run_id = $1
        "#,
    )
    .bind(run_id)
    .execute(&mut *conn)
    .await?;

    // 8. Delete from main tables (cascades will handle sub-tables)
    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(&mut *conn)
        .await?;

    Ok(())
}
