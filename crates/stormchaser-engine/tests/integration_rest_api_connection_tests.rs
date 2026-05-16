#![allow(clippy::explicit_auto_deref)]
use chrono::Utc;
use futures::StreamExt;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::env::var;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::RunId;
use tokio::time::sleep;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use stormchaser_tls::TlsConfig;
use stormchaser_tls::TlsReloader;
use uuid::Uuid;

#[tokio::test]
async fn test_rest_api_with_httpapi_connection() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();
    let _opa_client = Arc::new(OpaClient::new(None, None));

    let mock_server = MockServer::start().await;

    // Create an HttpApi connection
    let connection_id = Uuid::new_v4();
    let backend_name = format!("test-httpapi-{}", connection_id);
    sqlx::query(
        r#"
        INSERT INTO connections (id, name, connection_type, config, encrypted_credentials, is_default_sfs)
        VALUES ($1, $2, 'http_api', $3, $4, FALSE)
        "#,
    )
    .bind(connection_id)
    .bind(&backend_name)
    .bind(json!({
        "base_url": mock_server.uri(),
        "headers": {
            "X-Custom-Header": "from-connection"
        }
    }))
    .bind("secret_token_123")
    .execute(&pool)
    .await
    .unwrap();

    Mock::given(method("POST"))
        .and(path("/api/secure-test"))
        .and(header("Authorization", "Bearer secret_token_123"))
        .and(header("X-Custom-Header", "from-connection"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "success",
            "data": {
                "token": "ABC123XYZ",
                "id": 42
            }
        })))
        .mount(&mock_server)
        .await;

    let run_id = RunId::new_v4();
    let dsl = format!(
        r#"
        stormchaser_dsl_version = "v1"
        workflow "rest-api-test" {{
            inputs {{
                test_value = string()
            }}
            steps {{
                step "fetch_data" "RestApi" {{
                    spec = {{
                        connection = "{}"
                        url = "/api/secure-test"
                        method = "POST"
                        headers = {{
                            "Content-Type" = "application/json"
                        }}
                        extractors = [
                            {{
                                name = "my_token"
                                format = "json"
                                json_pointer = "/data/token"
                            }}
                        ]
                    }}
                }}
            }}
        }}
    "#,
        backend_name
    );

    let mut completion_sub = nats_client
        .subscribe("stormchaser.v1.*.step.>")
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    let run = stormchaser_model::workflow::WorkflowRun {
        id: run_id,
        workflow_name: "rest-api-test".to_string(),
        initiating_user: "test".to_string(),
        repo_url: "direct://".to_string(),
        workflow_path: "inline.storm".to_string(),
        git_ref: "HEAD".to_string(),
        status: stormchaser_model::workflow::RunStatus::Running,
        version: 1,
        fencing_token: Utc::now().timestamp_nanos_opt().unwrap_or(0),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        started_resolving_at: Some(Utc::now()),
        started_at: Some(Utc::now()),
        finished_at: None,
        error: None,
    };
    stormchaser_engine::db::insert_full_workflow_run(
        &mut *tx,
        &run,
        "v1",
        serde_json::json!({}),
        Some(&dsl),
        json!({"test_value": "hello"}),
        10,
        "1",
        "4Gi",
        "10Gi",
        "1h",
    )
    .await
    .unwrap();

    let step_id = stormchaser_model::StepInstanceId::new_v4();
    let spec = json!({
        "connection": backend_name,
        "url": "/api/secure-test",
        "method": "POST",
        "headers": {
            "Content-Type": "application/json"
        },
        "extractors": [
            {
                "name": "my_token",
                "format": "json",
                "json_pointer": "/data/token"
            }
        ]
    });

    stormchaser_engine::db::insert_step_instance_with_spec(
        &mut *tx,
        step_id,
        run_id,
        "fetch_data",
        "RestApi",
        stormchaser_model::step::StepStatus::Pending,
        None,
        spec.clone(),
        json!({}),
        Utc::now(),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let tls_config = TlsConfig::default();
    let tls_reloader = Arc::new(TlsReloader::new(tls_config).await.unwrap());

    stormchaser_engine::handler::step::intrinsic::rest_api::try_dispatch(
        run_id,
        step_id,
        1,
        "RestApi",
        &spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await
    .unwrap();

    let step_completed_payload;

    let timeout = sleep(Duration::from_secs(10));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            msg = completion_sub.next() => {
                if let Some(msg) = msg {
                    if let Ok(ce) = serde_json::from_slice::<cloudevents::Event>(&msg.payload) {
                        if let Some(cloudevents::Data::Json(payload)) = ce.data() {
                            if payload["run_id"].as_str() == Some(&run_id.to_string()) {
                                if payload["event_type"] == "StepCompletedEvent" {
                                    step_completed_payload = payload.clone();
                                    break;
                                } else if payload["event_type"] == "StepFailedEvent" {
                                    panic!("Step failed: {:?}", payload);
                                }
                            }
                        }
                    }
                }
            }
            _ = &mut timeout => {
                panic!("Timed out waiting for step completion event");
            }
        }
    }

    assert_eq!(step_completed_payload["exit_code"], 0);

    let outputs = &step_completed_payload["outputs"];
    assert_eq!(outputs["my_token"], "ABC123XYZ");

    sqlx::query("DELETE FROM connections WHERE id = $1")
        .bind(connection_id)
        .execute(&pool)
        .await
        .unwrap();
}
