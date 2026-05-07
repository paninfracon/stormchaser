use serde_json::Value;
use sqlx::{Executor, Postgres};
use stormchaser_model::{BackendId, RunId, StepInstanceId, TestSummary};
use uuid::Uuid;

use stormchaser_model::test_report;

#[allow(clippy::too_many_arguments)]
/// Upsert run storage state.
pub async fn upsert_run_storage_state<'a, E>(
    executor: E,
    run_id: Uuid,
    storage_name: &str,
    last_hash: &str,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
                    INSERT INTO run_storage_states (run_id, storage_name, last_hash)
                    VALUES ($1, $2, $3)
                    ON CONFLICT (run_id, storage_name) DO UPDATE SET last_hash = EXCLUDED.last_hash, updated_at = NOW()
                    "#,
    )
    .bind(run_id)
    .bind(storage_name)
    .bind(last_hash)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Get storage backend id by name.
pub async fn get_storage_backend_id_by_name<'a, E, O>(
    executor: E,
    name: &str,
) -> Result<Option<O>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin,
    (O,): for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>,
{
    sqlx::query_scalar::<_, O>("SELECT id FROM storage_backends WHERE name = $1")
        .bind(name)
        .fetch_optional(executor)
        .await
}

#[allow(clippy::too_many_arguments)]
/// Get default sfs backend id.
pub async fn get_default_sfs_backend_id<'a, E, O>(executor: E) -> Result<Option<O>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin,
    (O,): for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>,
{
    sqlx::query_scalar::<_, O>(
        "SELECT id FROM storage_backends WHERE is_default_sfs = TRUE LIMIT 1",
    )
    .fetch_optional(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Insert artifact registry.
pub async fn insert_artifact_registry<'a, E>(
    executor: E,
    run_id: RunId,
    step_instance_id: StepInstanceId,
    artifact_name: &str,
    backend_id: BackendId,
    remote_path: String,
    metadata: Value,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
                        INSERT INTO artifact_registry (run_id, step_instance_id, artifact_name, backend_id, remote_path, metadata)
                        VALUES ($1, $2, $3, $4, $5, $6)
                        "#,
    )
    .bind(run_id)
    .bind(step_instance_id)
    .bind(artifact_name)
    .bind(backend_id)
    .bind(remote_path)
    .bind(metadata)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Insert step test report.
pub async fn insert_step_test_report<'a, E>(
    executor: E,
    run_id: Uuid,
    step_instance_id: Uuid,
    report_name: &str,
    file_name: &str,
    format: &str,
    content: Option<&str>,
    checksum: &str,
    backend_id: Option<Uuid>,
    remote_path: Option<&str>,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
                    INSERT INTO step_test_reports (run_id, step_instance_id, report_name, file_name, format, content, checksum, backend_id, remote_path)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                    "#,
    )
    .bind(run_id)
    .bind(step_instance_id)
    .bind(report_name)
    .bind(file_name)
    .bind(format)
    .bind(content)
    .bind(checksum)
    .bind(backend_id)
    .bind(remote_path)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Insert step test summary.
pub async fn insert_step_test_summary<'a, E>(
    executor: E,
    run_id: Uuid,
    step_instance_id: Uuid,
    report_name: &str,
    summary: &TestSummary,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
                    INSERT INTO step_test_summaries (run_id, step_instance_id, report_name, total_tests, passed, failed, skipped, errors, duration_ms)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                    "#,
    )
    .bind(run_id)
    .bind(step_instance_id)
    .bind(report_name)
    .bind(summary.total_tests)
    .bind(summary.passed)
    .bind(summary.failed)
    .bind(summary.skipped)
    .bind(summary.errors)
    .bind(summary.duration_ms)
    .execute(executor)
    .await
}

/// Insert step test case.
pub async fn insert_step_test_case<'a, E>(
    executor: E,
    run_id: Uuid,
    step_instance_id: Uuid,
    report_name: &str,
    test_case: &test_report::TestCase,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
                    INSERT INTO step_test_cases (run_id, step_instance_id, report_name, test_suite, test_case, status, duration_ms, message)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                    "#,
    )
    .bind(run_id)
    .bind(step_instance_id)
    .bind(report_name)
    .bind(&test_case.test_suite)
    .bind(&test_case.test_case)
    .bind(&test_case.status)
    .bind(test_case.duration_ms)
    .bind(&test_case.message)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Get storage backend by name.
pub async fn get_storage_backend_by_name<'a, E, O>(
    executor: E,
    name: &str,
) -> Result<Option<O>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>,
{
    sqlx::query_as::<_, O>("SELECT * FROM storage_backends WHERE name = $1")
        .bind(name)
        .fetch_optional(executor)
        .await
}

#[allow(clippy::too_many_arguments)]
/// Get default sfs backend.
pub async fn get_default_sfs_backend<'a, E, O>(executor: E) -> Result<Option<O>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>,
{
    sqlx::query_as::<_, O>("SELECT * FROM storage_backends WHERE is_default_sfs = TRUE LIMIT 1")
        .fetch_optional(executor)
        .await
}

#[allow(clippy::too_many_arguments)]
/// Get run storage last hash.
pub async fn get_run_storage_last_hash<'a, E, O>(
    executor: E,
    run_id: Uuid,
    storage_name: &str,
) -> Result<Option<O>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>,
{
    sqlx::query_as::<_, O>(
        "SELECT last_hash FROM run_storage_states WHERE run_id = $1 AND storage_name = $2",
    )
    .bind(run_id)
    .bind(storage_name)
    .fetch_optional(executor)
    .await
}

/// Get storage backend by id.
pub async fn get_storage_backend_by_id<'a, E, O>(
    executor: E,
    id: Uuid,
) -> Result<Option<O>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>,
{
    sqlx::query_as::<_, O>("SELECT * FROM storage_backends WHERE id = $1")
        .bind(id)
        .fetch_optional(executor)
        .await
}

pub async fn get_artifact_by_name<'a, E>(
    executor: E,
    run_id: Uuid,
    artifact_name: &str,
) -> Result<Option<(Uuid, String)>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    let record = sqlx::query(
        r#"
        SELECT backend_id, remote_path
        FROM artifact_registry
        WHERE run_id = $1 AND artifact_name = $2
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(run_id)
    .bind(artifact_name)
    .fetch_optional(executor)
    .await?;

    Ok(record.map(|r| {
        use sqlx::Row;
        (r.get("backend_id"), r.get("remote_path"))
    }))
}
