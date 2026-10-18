#[cfg(feature = "email")]
use sqlx::PgPool;
#[cfg(feature = "email")]
use std::env::var;
#[cfg(feature = "email")]
use std::sync::Arc;
#[cfg(feature = "email")]
use stormchaser_engine::handler::handle_email_send;
#[cfg(feature = "email")]
use stormchaser_model::{RunId, StepInstanceId};
#[cfg(feature = "email")]
use stormchaser_tls::{TlsConfig, TlsReloader};

#[cfg(feature = "email")]
async fn setup() -> (PgPool, async_nats::Client, Arc<TlsReloader>) {
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url)
        .await
        .expect("Failed to connect to DB");
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = stormchaser_model::nats::connect(&nats_url).await.unwrap();
    // Built-in steps report their results through the outbox, which the engine's
    // relay publishes
    let _relay =
        stormchaser_engine::workers::start_outbox_relay_worker(pool.clone(), nats_client.clone());
    let tls_config = TlsConfig::default();
    let tls_reloader = Arc::new(TlsReloader::new(tls_config).await.unwrap());
    (pool, nats_client, tls_reloader)
}

#[cfg(feature = "email")]
#[tokio::test]
async fn test_ses_handler_invoke() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("debug")
        .try_init();
    let (pool, nats_client, tls_reloader) = setup().await;

    // Start wiremock
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // We expect a POST for SES SendEmail
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": "success"})),
        )
        .mount(&mock_server)
        .await;

    // Force AWS SDK to use our mock server
    std::env::set_var("AWS_ENDPOINT_URL", mock_server.uri());
    std::env::set_var("AWS_ACCESS_KEY_ID", "test");
    std::env::set_var("AWS_SECRET_ACCESS_KEY", "test");

    let run_id = RunId::new_v4();
    let step_id = StepInstanceId::new_v4();

    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, $2, $3, 'running', 1, 'http://git.local', 'workflow.storm', 'main')")
        .bind(run_id)
        .bind("test-workflow")
        .bind("test-user")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO run_contexts (run_id, inputs, secrets, source_code, dsl_version, workflow_definition) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(run_id)
        .bind(serde_json::json!({}))
        .bind(serde_json::json!({}))
        .bind("")
        .bind("1.0")
        .bind(serde_json::json!({}))
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, step_type, status, spec, params) VALUES ($1, $2, 'test-step', 'Email', 'pending', '{}', '{}')")
        .bind(step_id)
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    let spec = serde_json::json!({
        "backend": "ses",
        "from": "test@example.com",
        "to": ["to@example.com"],
        "subject": "Test Subject",
        "body": "Test Body",
        "ses_region": "us-east-1"
    });

    let res = handle_email_send(
        run_id,
        step_id,
        spec.clone(),
        pool.clone(),
        tls_reloader.clone(),
    )
    .await;

    assert!(res.is_ok(), "SES invoke failed: {:?}", res.err());

    let spec_invalid = serde_json::json!({
        "backend": "ses",
        "from": "test@example.com",
        "to": ["to@example.com"],
        "subject": "Test Subject",
        "body": "Test Body",
        "ses_region": "us-east-1",
        "ses_role_arn": "arn:aws:iam::123456789012:role/invalid"
    });

    let step_id_2 = StepInstanceId::new_v4();
    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, step_type, status, spec, params) VALUES ($1, $2, 'test-step2', 'Email', 'pending', '{}', '{}')")
        .bind(step_id_2)
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    let mut failures = nats_client
        .subscribe("stormchaser.v1.*.step.failed")
        .await
        .unwrap();

    let res_fail = handle_email_send(
        run_id,
        step_id_2,
        spec_invalid,
        pool.clone(),
        tls_reloader.clone(),
    )
    .await;

    // With an invalid role ARN, the STS assume-role response (mocked as
    // {"status": "success"}) cannot be parsed as valid AWS credentials.
    // The handler reports the failure to the step-failed handler and returns Ok.
    assert!(
        res_fail.is_ok(),
        "Expected Ok even on failure, got: {:?}",
        res_fail
    );

    let reported = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        use futures::StreamExt;
        while let Some(msg) = failures.next().await {
            let event: serde_json::Value = serde_json::from_slice(&msg.payload).unwrap();
            if event["data"]["step_id"] == serde_json::json!(step_id_2) {
                return event["data"]["error"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
            }
        }
        String::new()
    })
    .await
    .expect("no failure was reported for the step");
    assert!(reported.contains("Failed to send email"), "{reported}");
}
