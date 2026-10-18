mod common;

use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::env::var;
use std::sync::Arc;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::RunId;

async fn setup_db() -> sqlx::PgPool {
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap()
}

#[tokio::test]
async fn test_headless_validation_dynamic_query_success() {
    let pool = setup_db().await;
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = stormchaser_model::nats::connect(nats_url).await.unwrap();
    let opa_client = Arc::new(OpaClient::new(None, None));

    let run_id = RunId::new_v4();
    let workflow_name = format!("test-dyn-query-{}", run_id);
    let dsl = std::fs::read_to_string("../../tests/dynamic-query.storm")
        .expect("Failed to read dynamic-query.storm");
    let dsl = dsl.replace("dynamic-query", &workflow_name);

    let payload = json!({
        "run_id": run_id,
        "dsl": dsl,
        "inputs": {"environment": "staging"},
        "initiating_user": "test-user"
    });

    common::run_direct(payload, &pool, opa_client.clone(), nats_client.clone())
        .await
        .unwrap();

    let status: String = sqlx::query_scalar("SELECT status::text FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_ne!(status, "failed", "Validation should have succeeded");
}

#[tokio::test]
async fn test_headless_validation_dynamic_query_failure() {
    let pool = setup_db().await;
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = stormchaser_model::nats::connect(nats_url).await.unwrap();
    let opa_client = Arc::new(OpaClient::new(None, None));

    let run_id = RunId::new_v4();
    let workflow_name = format!("test-dyn-query-fail-{}", run_id);
    let dsl = std::fs::read_to_string("../../tests/dynamic-query.storm")
        .expect("Failed to read dynamic-query.storm");
    let dsl = dsl.replace("dynamic-query", &workflow_name);

    let payload = json!({
        "run_id": run_id,
        "dsl": dsl,
        "inputs": {"environment": "invalid-env"},
        "initiating_user": "test-user"
    });

    let res = common::run_direct(payload, &pool, opa_client.clone(), nats_client.clone()).await;

    let err = res.unwrap_err();
    assert!(err.to_string().contains("Input validation failed"));
}

#[tokio::test]
async fn test_execute_query_sql_without_connection_is_rejected() {
    let pool = setup_db().await;
    let mut params = std::collections::HashMap::new();
    params.insert("query".to_string(), "SELECT 'test_val'".to_string());

    // A query that names no connection must never run on the system pool.
    let err = stormchaser_engine::query::execute_query("sql", &params, Some(&pool), None, None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("require a 'connection'"));
}

#[tokio::test]
async fn test_headless_validation_malformed_schema_compile_error() {
    let pool = setup_db().await;
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = stormchaser_model::nats::connect(nats_url).await.unwrap();
    let opa_client = Arc::new(OpaClient::new(None, None));

    let run_id = RunId::new_v4();
    let dsl = r#"
workflow "bad-schema" {
  inputs {
    environment = string(type("invalid_json_type"))
  }
}
    "#;

    let payload = json!({
        "run_id": run_id,
        "dsl": dsl,
        "inputs": {},
        "initiating_user": "test-user"
    });

    let res = common::run_direct(payload, &pool, opa_client.clone(), nats_client.clone()).await;

    let err = res.unwrap_err();
    assert!(err.to_string().contains("Failed to compile input schema"));
}

#[tokio::test]
async fn test_headless_validation_hcl_eval_error() {
    let pool = setup_db().await;
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = stormchaser_model::nats::connect(nats_url).await.unwrap();
    let opa_client = Arc::new(OpaClient::new(None, None));

    let run_id = RunId::new_v4();
    let dsl = r#"
workflow "bad-schema-eval" {
  inputs {
    environment = string(default("${undefined_var.does_not_exist}"))
  }
}
    "#;

    let payload = json!({
        "run_id": run_id,
        "dsl": dsl,
        "inputs": {},
        "initiating_user": "test-user"
    });

    let res = common::run_direct(payload, &pool, opa_client.clone(), nats_client.clone()).await;

    let err = res.unwrap_err();
    assert!(err
        .to_string()
        .contains("Failed to evaluate expressions in inputs schema"));
}

#[tokio::test]
async fn test_execute_query_connection_resolution() {
    let pool = setup_db().await;

    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });

    let conn_id = stormchaser_model::id::ConnectionId::new_v4();
    sqlx::query("INSERT INTO connections (id, name, connection_type, config) VALUES ($1, 'test-db-conn', 'postgres', $2)")
        .bind(conn_id)
        .bind(serde_json::json!({ "url": db_url }))
        .execute(&pool)
        .await
        .unwrap();

    let mut params = std::collections::HashMap::new();
    params.insert("connection".to_string(), "test-db-conn".to_string());
    params.insert("query".to_string(), "SELECT 'test_conn_val'".to_string());

    let res = stormchaser_engine::query::execute_query("sql", &params, Some(&pool), None, None)
        .await
        .unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0], serde_json::json!("test_conn_val"));

    // A query cannot change data: it runs in a read-only transaction.
    for write in [
        "CREATE TABLE query_write_probe (id int)",
        "UPDATE connections SET description = 'changed' WHERE name = 'test-db-conn' RETURNING name",
    ] {
        params.insert("query".to_string(), write.to_string());
        let err = stormchaser_engine::query::execute_query("sql", &params, Some(&pool), None, None)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("read-only transaction"),
            "{write}: {err}"
        );
    }

    // Nor can it lift the restriction for a later statement: only one statement runs, and
    // the transaction is rolled back.
    params.insert(
        "query".to_string(),
        "SET TRANSACTION READ WRITE".to_string(),
    );
    let _ = stormchaser_engine::query::execute_query("sql", &params, Some(&pool), None, None).await;
    let description: Option<String> =
        sqlx::query_scalar("SELECT description FROM connections WHERE id = $1")
            .bind(conn_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(description, None);

    // More rows than the cap is an error, not an unbounded read.
    params.insert(
        "query".to_string(),
        "SELECT g::text FROM generate_series(1, 10001) g".to_string(),
    );
    let err = stormchaser_engine::query::execute_query("sql", &params, Some(&pool), None, None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("more than 10000 rows"), "{err}");

    sqlx::query("DELETE FROM connections WHERE id = $1")
        .bind(conn_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_direct_run_that_does_not_parse_is_recorded_as_failed() {
    let pool = setup_db().await;
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = stormchaser_model::nats::connect(nats_url).await.unwrap();
    let run_id = RunId::new_v4();

    let err = common::run_direct(
        json!({ "run_id": run_id, "dsl": "workflow {", "inputs": {}, "initiating_user": "test-user" }),
        &pool,
        Arc::new(OpaClient::new(None, None)),
        nats_client,
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("parsing failed"), "{err}");

    common::delete_run(&pool, run_id).await;
}
