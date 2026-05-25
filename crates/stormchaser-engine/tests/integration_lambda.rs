#[cfg(feature = "aws-lambda")]
use sqlx::PgPool;
#[cfg(feature = "aws-lambda")]
use std::env::var;
#[cfg(feature = "aws-lambda")]
use stormchaser_engine::handler::handle_lambda_invoke;
#[cfg(feature = "aws-lambda")]
use stormchaser_model::RunId;
#[cfg(feature = "aws-lambda")]
use stormchaser_model::StepInstanceId;

#[cfg(feature = "aws-lambda")]
async fn setup() -> (PgPool, async_nats::Client) {
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPool::connect(&db_url).await.unwrap();
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(&nats_url).await.unwrap();
    (pool, nats_client)
}

#[cfg(feature = "aws-lambda")]
#[tokio::test]
async fn test_lambda_handler_invoke() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("debug")
        .try_init();
    let (pool, nats_client) = setup().await;

    // Start wiremock
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // We expect a POST to /2015-03-31/functions/test-func/invocations
    Mock::given(method("POST"))
        .and(path("/2015-03-31/functions/test-func/invocations"))
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
    let spec = serde_json::json!({
        "function_name": "test-func",
        "region": "us-east-1",
        "payload": {"hello": "world"}
    });

    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, $2, $3, 'running', 1, 'http://git.local', 'workflow.storm', 'main')")
        .bind(run_id)
        .bind("test-workflow")
        .bind("test-user")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, step_type, status, spec, params) VALUES ($1, $2, 'test-step', 'LambdaInvoke', 'pending', '{}', '{}')")
        .bind(step_id)
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    let res = handle_lambda_invoke(run_id, step_id, spec, pool.clone(), nats_client).await;

    assert!(res.is_ok(), "Lambda invoke failed: {:?}", res.err());
}
