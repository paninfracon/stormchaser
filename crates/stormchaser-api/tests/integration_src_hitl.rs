use axum::response::IntoResponse;
use serde_json::json;
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::sync::Arc;
use stormchaser_model::auth::Claims;
use stormchaser_model::auth::OpaClient;
use uuid::Uuid;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use axum::extract::{Path, State};
use axum::Json;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use stormchaser_api::auth::AuthClaims;
use stormchaser_api::hitl::*;
use stormchaser_api::AppState;
use stormchaser_api::JWT_SECRET;

#[derive(serde::Deserialize, serde::Serialize)]
struct ApprovalLinkPayload {
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
    action: String,
    #[serde(default)]
    inputs: Value,
}

async fn mock_state() -> AppState {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            std::env::var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
        .unwrap();

    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats = async_nats::connect(nats_url).await.unwrap();

    AppState {
        pool,
        nats,
        opa: Arc::new(OpaClient::new(None, None)),
        oidc_config: None,
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
    }
}

fn encrypt_token(payload: &ApprovalLinkPayload) -> String {
    let mut hasher = Sha256::new();
    hasher.update(JWT_SECRET);
    let key_bytes = hasher.finalize();
    let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);

    let nonce_bytes = [0u8; 12]; // Fixed nonce for tests is fine
    let nonce = Nonce::from_slice(&nonce_bytes);

    let plaintext = serde_json::to_vec(payload).unwrap();
    let ciphertext = cipher.encrypt(nonce, plaintext.as_ref()).unwrap();

    let mut combined = nonce_bytes.to_vec();
    combined.extend_from_slice(&ciphertext);
    URL_SAFE_NO_PAD.encode(combined)
}

#[tokio::test]
async fn test_approve_step_link_invalid_token() {
    let state = mock_state().await;
    let response = approve_step_link(State(state), Path("invalid-token".into())).await;
    assert_eq!(response.into_response().status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_approve_step_not_found() {
    let state = mock_state().await;
    let payload = ApprovalLinkPayload {
        run_id: stormchaser_model::RunId::new_v4(),
        step_id: stormchaser_model::StepInstanceId::new_v4(),
        action: "approve".into(),
        inputs: json!({}),
    };
    let token = encrypt_token(&payload);
    let response = approve_step_link(State(state), Path(token)).await;
    assert_eq!(response.into_response().status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_approve_step_link_success() {
    let state = mock_state().await;
    let run_id = stormchaser_model::RunId::new_v4();
    let step_id = stormchaser_model::StepInstanceId::new_v4();

    // Setup DB data
    sqlx::query(
            "INSERT INTO workflow_runs (id, workflow_name, initiating_user, repo_url, workflow_path, git_ref, status, fencing_token) VALUES ($1, $2, $3, $4, $5, $6, $7::run_status, $8)"
        )
        .bind(run_id)
        .bind("test-workflow")
        .bind("test-user")
        .bind("http://github.com/test/repo")
        .bind("workflow.storm")
        .bind("HEAD")
        .bind("running")
        .bind(1i64)
        .execute(&state.pool)
        .await
        .unwrap();

    sqlx::query(
            "INSERT INTO step_instances (id, run_id, step_name, step_type, status, created_at) VALUES ($1, $2, $3, $4, $5::step_status, now())"
        )
        .bind(step_id)
        .bind(run_id)
        .bind("approval-step")
        .bind("approval")
        .bind("waiting_for_event")
        .execute(&state.pool)
        .await
        .unwrap();

    let payload = ApprovalLinkPayload {
        run_id,
        step_id,
        action: "approve".into(),
        inputs: json!({"decision": "yes"}),
    };
    let token = encrypt_token(&payload);

    let response = approve_step_link(State(state), Path(token)).await;
    assert_eq!(response.into_response().status(), StatusCode::OK);
}

#[tokio::test]
async fn test_approve_step_success() {
    let state = mock_state().await;
    let run_id = stormchaser_model::RunId::new_v4();
    let step_id = stormchaser_model::StepInstanceId::new_v4();

    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, repo_url, workflow_path, git_ref, status, fencing_token) VALUES ($1, 'wf', 'user', 'url', 'path', 'ref', 'running'::run_status, 1)").bind(run_id).execute(&state.pool).await.unwrap();
    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, step_type, status, created_at) VALUES ($1, $2, 'step', 'approval', 'waiting_for_event'::step_status, now())").bind(step_id).bind(run_id).execute(&state.pool).await.unwrap();

    let response = approve_step(
        State(state),
        AuthClaims(Claims {
            sub: "test-user-123".to_string(),
            email: Some("test-user-123@paninfracon.net".to_string()),
            exp: 0,
        }),
        axum::http::HeaderMap::new(),
        Path((run_id, step_id)),
        Json(json!({"approved": true})),
    )
    .await;
    assert_eq!(response.into_response().status(), StatusCode::OK);
}

#[tokio::test]
async fn test_reject_step_success() {
    let state = mock_state().await;
    let run_id = stormchaser_model::RunId::new_v4();
    let step_id = stormchaser_model::StepInstanceId::new_v4();

    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, repo_url, workflow_path, git_ref, status, fencing_token) VALUES ($1, 'wf', 'user', 'url', 'path', 'ref', 'running'::run_status, 1)").bind(run_id).execute(&state.pool).await.unwrap();
    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, step_type, status, created_at) VALUES ($1, $2, 'step', 'approval', 'waiting_for_event'::step_status, now())").bind(step_id).bind(run_id).execute(&state.pool).await.unwrap();

    let response = reject_step(
        State(state),
        AuthClaims(Claims {
            sub: "test-user-123".to_string(),
            email: Some("test-user-123@paninfracon.net".to_string()),
            exp: 0,
        }),
        axum::http::HeaderMap::new(),
        Path((run_id, step_id)),
    )
    .await;
    assert_eq!(response.into_response().status(), StatusCode::OK);
}

#[tokio::test]
async fn test_correlate_event_success() {
    let state = mock_state().await;
    let run_id = stormchaser_model::RunId::new_v4();
    let step_id = stormchaser_model::StepInstanceId::new_v4();
    let corr_id = Uuid::new_v4();

    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, repo_url, workflow_path, git_ref, status, fencing_token) VALUES ($1, 'wf', 'user', 'url', 'path', 'ref', 'running'::run_status, 1)").bind(run_id).execute(&state.pool).await.unwrap();
    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, step_type, status, created_at) VALUES ($1, $2, 'step', 'event', 'waiting_for_event'::step_status, now())").bind(step_id).bind(run_id).execute(&state.pool).await.unwrap();

    sqlx::query("INSERT INTO event_correlations (id, step_instance_id, run_id, correlation_key, correlation_value) VALUES ($1, $2, $3, $4, $5)")
            .bind(corr_id).bind(step_id).bind(run_id).bind("ext_id").bind("123").execute(&state.pool).await.unwrap();

    let response = correlate_event(
        State(state),
        Json(json!({"key": "ext_id", "value": "123", "data": "payload"})),
    )
    .await;
    assert_eq!(response.into_response().status(), StatusCode::OK);
}
