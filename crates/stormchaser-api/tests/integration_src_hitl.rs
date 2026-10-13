use axum::response::IntoResponse;
use serde_json::json;
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::env::var;
use std::sync::Arc;
use stormchaser_model::auth::Claims;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use uuid::Uuid;

use axum::extract::{Path, State};
use axum::Json;
use reqwest::StatusCode;
use stormchaser_api::auth::AuthClaims;
use stormchaser_api::hitl::*;
use stormchaser_api::AppState;
use stormchaser_engine::hitl::{seal_approval_link, ApprovalLink};

/// The key these tests seal links with; handlers are given it directly.
const LINK_KEY: &str = "test-only-approval-link-key-aaaaaa";

fn link(run_id: RunId, step_id: StepInstanceId, action: &str, inputs: Value) -> ApprovalLink {
    ApprovalLink {
        run_id,
        step_id,
        action: action.into(),
        inputs,
        exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
        approvers: vec!["lead@paninfracon.net".into()],
    }
}

fn encrypt_token(link: &ApprovalLink) -> String {
    seal_approval_link(link, LINK_KEY).unwrap()
}

async fn mock_state() -> AppState {
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
    let nats = stormchaser_model::nats::connect(nats_url).await.unwrap();

    AppState {
        pool,
        nats,
        opa: Arc::new(OpaClient::new(None, None)),
        oidc_config: None,
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
        api_base_url: "http://localhost:3000".to_string(),
        mcp_allowed_hosts: Vec::new(),
    }
}

#[tokio::test]
async fn test_approve_step_link_invalid_token() {
    let state = mock_state().await;
    let response = use_approval_link(&state, "invalid-token", Some(LINK_KEY)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_approve_step_not_found() {
    let state = mock_state().await;
    let token = encrypt_token(&link(
        RunId::new_v4(),
        StepInstanceId::new_v4(),
        "approve",
        json!({}),
    ));
    let response = use_approval_link(&state, &token, Some(LINK_KEY)).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn approval_links_do_nothing_without_a_configured_key() {
    let state = mock_state().await;
    let token = encrypt_token(&link(
        RunId::new_v4(),
        StepInstanceId::new_v4(),
        "approve",
        json!({}),
    ));
    assert_eq!(
        use_approval_link(&state, &token, None).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        approval_link_page(&state, &token, None).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn an_expired_or_foreign_link_is_refused() {
    let state = mock_state().await;
    let mut expired = link(
        RunId::new_v4(),
        StepInstanceId::new_v4(),
        "approve",
        json!({}),
    );
    expired.exp = (chrono::Utc::now() - chrono::Duration::minutes(1)).timestamp();
    let token = encrypt_token(&expired);
    assert_eq!(
        use_approval_link(&state, &token, Some(LINK_KEY))
            .await
            .status(),
        StatusCode::GONE
    );

    // Sealed with some other key, such as the JWT secret links used to share
    let foreign = seal_approval_link(
        &link(
            RunId::new_v4(),
            StepInstanceId::new_v4(),
            "approve",
            json!({}),
        ),
        "another-key-another-key-another-key",
    )
    .unwrap();
    assert_eq!(
        use_approval_link(&state, &foreign, Some(LINK_KEY))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn test_approve_step_link_success() {
    let state = mock_state().await;
    let run_id = RunId::new_v4();
    let step_id = StepInstanceId::new_v4();

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

    let token = encrypt_token(&link(
        run_id,
        step_id,
        "approve",
        json!({"decision": "yes"}),
    ));

    // Opening the link only shows a confirmation page; the step is still waiting
    let page = approval_link_page(&state, &token, Some(LINK_KEY)).await;
    assert_eq!(page.status(), StatusCode::OK);
    let body = axum::body::to_bytes(page.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("approval-step") && body.contains("method=\"post\""));
    assert!(!body.contains(&token), "the page must not repeat the token");
    let status: String =
        sqlx::query_scalar("SELECT status::text FROM step_instances WHERE id = $1")
            .bind(step_id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(status, "waiting_for_event");
    let decisions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM approval_registry WHERE step_instance_id = $1")
            .bind(step_id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(decisions, 0);

    // Submitting it records the decision, against whoever the link was sent to
    let response = use_approval_link(&state, &token, Some(LINK_KEY)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let recorded: String =
        sqlx::query_scalar("SELECT user_id FROM approval_registry WHERE step_instance_id = $1")
            .bind(step_id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(recorded, "link:lead@paninfracon.net");
}

#[tokio::test]
async fn test_approve_step_success() {
    let state = mock_state().await;
    let run_id = RunId::new_v4();
    let step_id = StepInstanceId::new_v4();

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
    let run_id = RunId::new_v4();
    let step_id = StepInstanceId::new_v4();

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
    let run_id = RunId::new_v4();
    let step_id = StepInstanceId::new_v4();
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
