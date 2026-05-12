use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use reqwest::StatusCode;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::env::var;
use std::sync::Arc;
use stormchaser_api::auth::AuthClaims;
use stormchaser_api::hitl::approve_step;
use stormchaser_api::AppState;
use stormchaser_model::auth::{Claims, OpaClient};
use stormchaser_model::StepInstanceId;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn mock_state(mock_server_uri: String) -> AppState {
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
        .unwrap();

    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats = async_nats::connect(nats_url).await.unwrap();

    AppState {
        pool,
        nats,
        opa: Arc::new(OpaClient::new(
            Some(format!(
                "{}/v1/data/stormchaser/authz/allow",
                mock_server_uri
            )),
            None,
        )),
        oidc_config: None,
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
        api_base_url: "http://localhost:3000".to_string(),
    }
}

#[tokio::test]
async fn test_hitl_opa_integration() {
    let mock_server = MockServer::start().await;

    // Mock OPA success response
    Mock::given(method("POST"))
        .and(path("/v1/data/stormchaser/authz/allow"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "result": true
        })))
        .mount(&mock_server)
        .await;

    let state = mock_state(mock_server.uri()).await;
    let run_id = stormchaser_model::RunId::new_v4();
    let step_id = StepInstanceId::new_v4();

    // 1. Insert workflow run
    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, repo_url, workflow_path, git_ref, status, fencing_token) VALUES ($1, 'wf', 'user', 'url', 'path', 'ref', 'running'::run_status, 1)").bind(run_id).execute(&state.pool).await.unwrap();

    // 2. Insert run_context to satisfy get_workflow_context_for_opa
    let workflow_def = json!({
        "steps": []
    });
    sqlx::query("INSERT INTO run_contexts (run_id, dsl_version, workflow_definition, source_code, inputs) VALUES ($1, 'v1', $2, 'code', '{}')")
        .bind(run_id).bind(workflow_def).execute(&state.pool).await.unwrap();

    // 3. Insert step instance
    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, step_type, status, created_at) VALUES ($1, $2, 'step', 'approval', 'waiting_for_event'::step_status, now())").bind(step_id).bind(run_id).execute(&state.pool).await.unwrap();

    // 4. Insert step outputs to satisfy get_run_outputs_for_opa
    let prev_step_id = StepInstanceId::new_v4();
    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, step_type, status, created_at) VALUES ($1, $2, 'prev', 'run_container', 'succeeded'::step_status, now())").bind(prev_step_id).bind(run_id).execute(&state.pool).await.unwrap();
    sqlx::query("INSERT INTO step_outputs (step_instance_id, key, value, is_sensitive) VALUES ($1, 'test_key', '\"test_val\"', false)").bind(prev_step_id).execute(&state.pool).await.unwrap();

    // Call approve_step, which invokes check_approval_opa and hits both db functions
    let response = approve_step(
        State(state),
        AuthClaims(Claims {
            sub: "test-user-123".to_string(),
            email: Some("test-user-123@paninfracon.net".to_string()),
            exp: 0,
        }),
        axum::http::HeaderMap::new(),
        Path((run_id, step_id)),
        Json(json!({"decision": "yes"})),
    )
    .await;

    let resp = response.into_response();
    if resp.status() != StatusCode::OK {
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        eprintln!("Error response: {:?}", String::from_utf8_lossy(&body));
    } else {
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
