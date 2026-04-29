use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use stormchaser_api::WorkflowRunDetail;
use stormchaser_model::workflow::RunStatus;
use uuid::Uuid;

#[tokio::test]
async fn test_workflow_run_detail_query() {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            std::env::var("STORMCHASER_DEV_PASSWORD").unwrap_or_else(|_| "stormchaser".to_string())
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    // 1. Setup test data
    let run_id = Uuid::new_v4();
    let workflow_name = format!("test-query-workflow-{}", run_id);

    // Clean up any previous runs with this ID (unlikely but safe)
    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query(
        r#"
        INSERT INTO workflow_runs (
            id, workflow_name, initiating_user, repo_url, workflow_path, git_ref, status, version, fencing_token
        ) VALUES ($1, $2, $3, $4, $5, $6, $7::run_status, $8, $9)
        "#,
    )
    .bind(run_id)
    .bind(&workflow_name)
    .bind("test-user")
    .bind("https://github.com/test/repo")
    .bind("workflow.storm")
    .bind("HEAD")
    .bind(RunStatus::Queued)
    .bind(1)
    .bind(123456)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"
        INSERT INTO run_contexts (
            run_id, dsl_version, workflow_definition, source_code, inputs, secrets, sensitive_values
        ) VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(run_id)
    .bind("1.0")
    .bind(json!({ "steps": [] }))
    .bind("workflow { }")
    .bind(json!({ "param1": "val1" }))
    .bind(json!({ "secret1": "masked" }))
    .bind(Vec::<String>::new())
    .execute(&pool)
    .await
    .unwrap();

    // 2. Execute the query using query_as
    // This is the EXACT query from get_workflow_run
    let detail: WorkflowRunDetail = sqlx::query_as(
        r#"
        WITH combined_runs AS (
            SELECT
                wr.id, wr.workflow_name, wr.initiating_user, wr.repo_url, wr.workflow_path, wr.git_ref,
                wr.status as "status", wr.version, wr.created_at, wr.updated_at, wr.started_resolving_at, wr.started_at, wr.finished_at, wr.error,
                rc.inputs, rc.secrets, rc.source_code, rc.dsl_version
            FROM workflow_runs wr
            JOIN run_contexts rc ON wr.id = rc.run_id
            UNION ALL
            SELECT
                wr.id, wr.workflow_name, wr.initiating_user, wr.repo_url, wr.workflow_path, wr.git_ref,
                wr.status as "status", wr.version, wr.created_at, wr.updated_at, wr.started_resolving_at, wr.started_at, wr.finished_at, wr.error,
                rc.inputs, rc.secrets, rc.source_code, rc.dsl_version
            FROM archived_workflow_runs wr
            JOIN archived_run_contexts rc ON wr.id = rc.run_id
        )
        SELECT * FROM combined_runs WHERE id = $1
        "#,
    )
    .bind(run_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch workflow run detail using query_as");

    // 3. Verify
    assert_eq!(detail.id, run_id);
    assert_eq!(detail.workflow_name, workflow_name);
    assert_eq!(detail.status, RunStatus::Queued);
    assert_eq!(detail.inputs["param1"], "val1");
    assert_eq!(detail.secrets["secret1"], "masked");
    assert_eq!(detail.source_code, "workflow { }");
    assert_eq!(detail.dsl_version, "1.0");

    // Cleanup
    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
}
