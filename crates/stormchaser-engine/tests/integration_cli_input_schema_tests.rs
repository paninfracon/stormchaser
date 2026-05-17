use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::env::var;
use std::sync::Arc;
use stormchaser_engine::handler;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::RunId;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

async fn run_validation_test(dsl: &str, inputs: serde_json::Value) -> Result<(), anyhow::Error> {
    let pool = setup_db().await;
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();
    let opa_client = Arc::new(OpaClient::new(None, None));

    let run_id = RunId::new_v4();
    let payload = json!({
        "run_id": run_id,
        "dsl": dsl.replace("WORKFLOW_NAME_PLACEHOLDER", &format!("test-{}", run_id)),
        "inputs": inputs,
        "initiating_user": "test-user"
    });

    handler::handle_workflow_direct(
        payload,
        pool.clone(),
        opa_client.clone(),
        nats_client.clone(),
    )
    .await
}

#[tokio::test]
async fn test_different_data_types_success() {
    let dsl = r#"
workflow "WORKFLOW_NAME_PLACEHOLDER" {
  inputs {
    str_val  = string()
    int_val  = integer()
    bool_val = boolean()
    required = ["str_val", "int_val", "bool_val"]
  }
}
"#;
    let _res = run_validation_test(
        dsl,
        json!({
            "str_val": "hello",
            "int_val": 42,
            "bool_val": true
        }),
    )
    .await;
}

#[tokio::test]
async fn test_different_data_types_failure() {
    let dsl = r#"
workflow "WORKFLOW_NAME_PLACEHOLDER" {
  inputs {
    int_val  = integer()
    required = ["int_val"]
  }
}
"#;
    let res = run_validation_test(
        dsl,
        json!({
            "int_val": "not an int"
        }),
    )
    .await;
    let err = res.expect_err("Validation should fail for wrong type: {:?}");
    assert!(err.to_string().contains("validation errors"));
}

#[tokio::test]
async fn test_validation_constraints_success() {
    let dsl = r#"
workflow "WORKFLOW_NAME_PLACEHOLDER" {
  inputs {
    name = string(pattern("^[a-z]+$"))
    age  = integer(minimum(18), maximum(100))
    required = ["name", "age"]
  }
}
"#;
    let res = run_validation_test(
        dsl,
        json!({
            "name": "john",
            "age": 30
        }),
    )
    .await;
    res.expect("Validation should succeed with valid constraints: {:?}");
}

#[tokio::test]
async fn test_validation_constraints_failure() {
    let dsl = r#"
workflow "WORKFLOW_NAME_PLACEHOLDER" {
  inputs {
    name = string(pattern("^[a-z]+$"))
    age  = integer(minimum(18), maximum(100))
  }
}
"#;
    // Test pattern failure
    let res1 = run_validation_test(dsl, json!({"name": "JOHN123"})).await;
    let _err = res1.expect_err("Validation should fail pattern constraint");

    // Test minimum failure
    let res2 = run_validation_test(dsl, json!({"age": 10})).await;
    let _err = res2.expect_err("Validation should fail minimum constraint");
}

#[tokio::test]
async fn test_enums_success() {
    let dsl = r#"
workflow "WORKFLOW_NAME_PLACEHOLDER" {
  inputs {
    color = string(enum(["red", "green", "blue"]))
  }
}
"#;
    let res = run_validation_test(dsl, json!({"color": "green"})).await;
    res.expect("Validation should succeed with valid enum");
}

#[tokio::test]
async fn test_enums_failure() {
    let dsl = r#"
workflow "WORKFLOW_NAME_PLACEHOLDER" {
  inputs {
    color = string(enum(["red", "green", "blue"]))
  }
}
"#;
    let res = run_validation_test(dsl, json!({"color": "yellow"})).await;
    let _err = res.expect_err("Validation should fail with invalid enum: {:?}");
}

#[tokio::test]
async fn test_conditional_fields_success() {
    let dsl = r#"
workflow "WORKFLOW_NAME_PLACEHOLDER" {
  inputs {
    deploy_type = string(enum(["aws", "k8s"]))

    allOf = [
      {
        if = { properties = { deploy_type = { const = "aws" } } }
        then = { properties = { region = { type = "string" } }, required = ["region"] }
      }
    ]
  }
}
"#;
    let res = run_validation_test(
        dsl,
        json!({
            "deploy_type": "aws",
            "region": "us-east-1"
        }),
    )
    .await;
    res.expect("Validation should succeed with conditional field present: {:?}");
}

#[tokio::test]
async fn test_conditional_fields_failure() {
    let dsl = r#"
workflow "WORKFLOW_NAME_PLACEHOLDER" {
  inputs {
    deploy_type = string(enum(["aws", "k8s"]))

    allOf = [
      {
        if = { properties = { deploy_type = { const = "aws" } } }
        then = { properties = { region = { type = "string" } }, required = ["region"] }
      }
    ]
  }
}
"#;
    let res = run_validation_test(
        dsl,
        json!({
            "deploy_type": "aws"
            // missing required "region" when deploy_type is "aws"
        }),
    )
    .await;
    let _err = res.expect_err("Validation should fail when conditional field is missing");
}

#[tokio::test]
async fn test_queries_mock_provider() {
    let dsl = r#"
workflow "WORKFLOW_NAME_PLACEHOLDER" {
  query "env_options" {
    type = "mock"
    params = {
      items = "dev,staging,prod"
    }
  }

  inputs {
    environment = string(enum("${queries.env_options}"))
  }
}
"#;
    let res_success = run_validation_test(dsl, json!({"environment": "staging"})).await;
    res_success.expect("Mock provider query success failed: {:?}");

    let res_fail = run_validation_test(dsl, json!({"environment": "local"})).await;
    let _err = res_fail.expect_err("Mock provider query failure failed: {:?}");
}

#[tokio::test]
async fn test_queries_wiremock_api() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/options"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "NextToken": "string",
            "ResourceDescriptions": [
                { "Identifier": "alpha" },
                { "Identifier": "beta" }
            ],
            "TypeName": "AWS::Example::Resource"
        })))
        .mount(&mock_server)
        .await;

    let dsl = format!(
        r#"
workflow "WORKFLOW_NAME_PLACEHOLDER" {{
  query "api_options" {{
    type = "api"
    params = {{
      url = "{}/api/options"
      jq_filter = ".ResourceDescriptions[].Identifier"
    }}
  }}

  inputs {{
    feature_flag = string(enum("${{queries.api_options}}"))
  }}
}}
"#,
        mock_server.uri()
    );

    let res_success = run_validation_test(&dsl, json!({"feature_flag": "alpha"})).await;
    res_success.expect("API query success failed: {:?}");

    let res_fail = run_validation_test(&dsl, json!({"feature_flag": "gamma"})).await;
    let _err = res_fail.expect_err("API query failure failed: {:?}");
}

#[tokio::test]
async fn test_queries_db_integration() {
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
    sqlx::query("INSERT INTO connections (id, name, connection_type, config) VALUES ($1, 'test-db-conn-inputs', 'postgres', $2)")
        .bind(conn_id)
        .bind(serde_json::json!({ "url": db_url }))
        .execute(&pool)
        .await
        .unwrap();

    // Ensure our test table exists and has data
    sqlx::query("CREATE TABLE IF NOT EXISTS test_enum_options (value TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM test_enum_options")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO test_enum_options (value) VALUES ('db_val_1'), ('db_val_2')")
        .execute(&pool)
        .await
        .unwrap();

    let dsl = r#"
workflow "WORKFLOW_NAME_PLACEHOLDER" {
  query "db_options" {
    type = "sql"
    params = {
      connection = "test-db-conn-inputs"
      query = "SELECT value FROM test_enum_options"
    }
  }

  inputs {
    db_item = string(enum("${queries.db_options}"))
  }
}
"#;

    let res_success = run_validation_test(dsl, json!({"db_item": "db_val_1"})).await;

    // Cleanup before assert
    sqlx::query("DELETE FROM connections WHERE id = $1")
        .bind(conn_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE IF EXISTS test_enum_options")
        .execute(&pool)
        .await
        .unwrap();

    res_success.expect("DB query success failed: {:?}");
}
