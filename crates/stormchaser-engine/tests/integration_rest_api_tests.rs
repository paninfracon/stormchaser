use futures::StreamExt;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_engine::handler;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::RunId;
use tokio::time::sleep;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use stormchaser_tls::TlsConfig;
use stormchaser_tls::TlsReloader;

#[tokio::test]
async fn test_rest_api_step_execution() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    // 1. Setup DB and NATS
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            std::env::var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();
    let opa_client = Arc::new(OpaClient::new(None, None));

    // 2. Start WireMock server
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/test"))
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
                        url = "{}/api/test"
                        method = "POST"
                        headers = {{
                            "Content-Type" = "application/json"
                        }}
                        body = "{{\"input_val\": \"{{{{ inputs.test_value }}}}\"}}"
                        extractors = [
                            {{
                                name = "my_token"
                                source = "body"
                                format = "json"
                                regex = "/data/token"
                            }},
                            {{
                                name = "my_id"
                                source = "body"
                                format = "json"
                                regex = "data.id"
                            }}
                        ]
                    }}
                }}
            }}
        }}
    "#,
        mock_server.uri()
    );

    // Subscribe to completion events before dispatching
    let mut completion_sub = nats_client
        .subscribe("stormchaser.v1.step.>")
        .await
        .unwrap();

    // 3. Initialize workflow
    let payload = json!({
        "run_id": run_id,
        "dsl": dsl,
        "inputs": {
            "test_value": "hello"
        },
        "initiating_user": "test"
    });
    handler::handle_workflow_direct(
        payload,
        pool.clone(),
        opa_client.clone(),
        nats_client.clone(),
    )
    .await
    .unwrap();

    // Start it
    handler::handle_workflow_start_pending(
        run_id,
        pool.clone(),
        nats_client.clone(),
        Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap()),
    )
    .await
    .unwrap();

    // 4. Wait for the intrinsic RestApi step to execute and publish its completion
    let step_completed_payload;

    // Timeout after 10 seconds
    let timeout = sleep(Duration::from_secs(10));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            msg = completion_sub.next() => {
                if let Some(msg) = msg {
                    if let Ok(ce) = serde_json::from_slice::<cloudevents::Event>(&msg.payload) {
                        if let Some(cloudevents::Data::Json(payload)) = ce.data() {
                            if payload["run_id"].as_str() == Some(&run_id.to_string()) {
                                println!("Received event: {:?}", payload);
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

    // 5. Verify the extracted outputs
    assert_eq!(step_completed_payload["exit_code"], 0);

    let outputs = &step_completed_payload["outputs"];
    assert_eq!(outputs["my_token"], "ABC123XYZ");
    assert_eq!(outputs["my_id"], 42);

    // Also verify the full response is available
    assert_eq!(outputs["response"]["status"], "success");
}
