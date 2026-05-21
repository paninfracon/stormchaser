use futures::StreamExt;
use sqlx::PgPool;
use std::env::var;
use std::sync::Arc;
use stormchaser_engine::handler::step::intrinsic::{
    jinja, rest_api, test_report_email, wasm, webhook,
};
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_tls::{TlsConfig, TlsReloader};

async fn setup() -> (PgPool, async_nats::Client, Arc<TlsReloader>) {
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
    let tls_config = TlsConfig::default();
    let tls_reloader = Arc::new(TlsReloader::new(tls_config).await.unwrap());
    (pool, nats_client, tls_reloader)
}

#[tokio::test]
async fn test_intrinsic_steps_dispatch() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("debug")
        .try_init();
    let (pool, nats_client, tls_reloader) = setup().await;
    let run_id = RunId::new_v4();
    let step_id = StepInstanceId::new_v4();
    let spec = serde_json::json!({});
    let jinja_spec = serde_json::json!({
        "template": "Hello {{ run.id }}"
    });

    // We'll subscribe to all step completion/failure events for this run to ensure emissions happen
    let mut subscriber = nats_client.subscribe("stormchaser.v1.>").await.unwrap();

    // Insert dummy run/step
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

    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, step_type, status, spec, params) VALUES ($1, $2, 'test-step', 'JinjaRender', 'pending', 'null', 'null')")
        .bind(step_id)
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    // Test Jinja
    let dispatched = jinja::try_dispatch(
        run_id,
        step_id,
        "JinjaRender",
        &jinja_spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await
    .unwrap();
    assert!(dispatched);

    // Verify NATS emission for the dispatched Jinja step
    let timeout = tokio::time::timeout(std::time::Duration::from_secs(5), subscriber.next()).await;
    let msg = timeout
        .expect("Timed out waiting for Jinja NATS emission")
        .expect("NATS stream closed");
    let response: serde_json::Value = serde_json::from_slice(&msg.payload).unwrap();
    assert_eq!(
        response["data"]["step_id"].as_str(),
        Some(step_id.to_string().as_str())
    );

    let dispatched = jinja::try_dispatch(
        run_id,
        step_id,
        "Other",
        &spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await
    .unwrap();
    assert!(!dispatched);

    // Test Webhook
    let dispatched = webhook::try_dispatch(
        run_id,
        step_id,
        "WebhookInvoke",
        &spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await
    .unwrap();
    assert!(dispatched);
    let dispatched = webhook::try_dispatch(
        run_id,
        step_id,
        "Other",
        &spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await
    .unwrap();
    assert!(!dispatched);

    // Test RestApi
    let dispatched = rest_api::try_dispatch(
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
    assert!(dispatched);
    let dispatched = rest_api::try_dispatch(
        run_id,
        step_id,
        1,
        "Other",
        &spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await
    .unwrap();
    assert!(!dispatched);

    // Test TestReportEmail
    let dispatched = test_report_email::try_dispatch(
        run_id,
        step_id,
        "TestReportEmail",
        &spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await
    .unwrap();
    assert!(dispatched);
    let dispatched = test_report_email::try_dispatch(
        run_id,
        step_id,
        "Other",
        &spec,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await
    .unwrap();
    assert!(!dispatched);

    // Wasm requires params
    let params = serde_json::json!({});
    // Test non-matching WASM
    let dispatched = wasm::try_dispatch(
        run_id,
        step_id,
        1,
        "Other",
        &spec,
        &params,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await
    .unwrap();
    assert!(!dispatched);

    // Test matching WASM inline
    let wasm_spec = serde_json::json!({
        "module": "test-module"
    });
    let dispatched = wasm::try_dispatch(
        run_id,
        step_id,
        1,
        "Wasm",
        &wasm_spec,
        &params,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await
    .unwrap();
    assert!(dispatched);
}
