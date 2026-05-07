use sqlx::PgPool;
use stormchaser_model::RunId;

use stormchaser_model::storage;

/// Retrieves a list of artifacts associated with a given workflow run.
pub async fn list_run_artifacts(
    pool: &PgPool,
    run_id: RunId,
) -> Result<Vec<storage::ArtifactRegistry>, sqlx::Error> {
    sqlx::query_as(
        r#"
            WITH combined_artifacts AS (
                SELECT * FROM artifact_registry
                UNION ALL
                SELECT * FROM archived_artifact_registry
            )
            SELECT * FROM combined_artifacts WHERE run_id = $1 ORDER BY created_at ASC
            "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
}
