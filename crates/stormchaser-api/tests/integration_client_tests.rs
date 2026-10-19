//! The shared client against the API itself: every call reaches a route that exists,
//! and every response decodes as the type the client expects.

use chrono::Utc;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use std::sync::{Arc, Once};
use stormchaser_api::{app, AppState, Claims, JWT_SECRET};
use stormchaser_client::Client;
use stormchaser_model::api::*;
use stormchaser_model::connections::ConnectionType;
use stormchaser_model::{ConnectionId, RuleId, WebhookId};
use stormchaser_opa::OpaClient;

static INIT: Once = Once::new();

/// A client of the API served on a local port, with a token for a test user, or `None`
/// when there is no test stack to run against.
async fn client() -> Option<Client> {
    INIT.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("Failed to install default crypto provider");
    });
    std::env::set_var("CRON_ENGINE", "none");
    std::env::set_var("API_RATE_LIMIT_PER_SECOND", "1000");
    std::env::set_var("API_RATE_LIMIT_BURST_SIZE", "1000");

    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats = stormchaser_model::nats::connect(nats_url).await.ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&std::env::var("DATABASE_URL").ok()?)
        .await
        .ok()?;
    let router = app(AppState {
        pool,
        nats,
        opa: Arc::new(OpaClient::new(None, None)),
        oidc_config: None,
        jwks: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        log_backend: None,
        api_base_url: "http://localhost:3000".to_string(),
        mcp_allowed_hosts: Vec::new(),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.ok()?;
    let addr = listener.local_addr().ok()?;
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    let claims = Claims {
        sub: "client-tester".to_string(),
        email: Some("client-tester@paninfracon.net".to_string()),
        exp: (Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
        groups: Vec::new(),
    };
    let token = stormchaser_api::sign_local_token(&claims, JWT_SECRET).unwrap();
    Some(
        Client::builder(format!("http://{addr}"))
            .token(token)
            .build()
            .unwrap(),
    )
}

#[tokio::test]
async fn sessions_runs_and_schema() {
    let Some(client) = client().await else { return };

    let session = client.session().await.unwrap();
    assert_eq!(session.sub, "client-tester");

    // A direct run is created by the engine, which is not running here; an enqueued
    // run is created by the API
    let connection = client
        .create_connection(&CreateStorageBackendRequest {
            name: format!("client-test-runs-{}", uuid::Uuid::new_v4().simple()),
            description: None,
            connection_type: ConnectionType::Git,
            config: json!({ "repo_url": "https://github.com/paninfracon/stormchaser.git" }),
            aws_assume_role_arn: None,
            encrypted_credentials: None,
            is_default_sfs: false,
        })
        .await
        .unwrap();
    let run = client
        .enqueue(&EnqueueRequest {
            workflow_name: "client-test".to_string(),
            connection: connection.id.to_string(),
            workflow_path: "tests/hello-world.storm".to_string(),
            git_ref: "trunk".to_string(),
            inputs: json!({}),
            overrides: None,
            sops_file: None,
            sops_role_arn: None,
        })
        .await
        .unwrap();

    let detail = client.get_run(run.run_id).await.unwrap();
    assert_eq!(detail.detail.id, run.run_id);
    let mine = client
        .list_runs(&ListRunsQuery {
            initiating_user: Some("client-tester@paninfracon.net".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(mine.iter().any(|r| r.id == run.run_id));
    assert!(client.run_artifacts(run.run_id).await.unwrap().is_empty());
    assert!(client
        .run_test_reports(run.run_id)
        .await
        .unwrap()
        .is_empty());
    assert!(client
        .run_test_summaries(run.run_id)
        .await
        .unwrap()
        .is_empty());

    assert!(client.schema().await.unwrap().get("properties").is_some());

    client.delete_run(run.run_id).await.unwrap();
    client
        .delete_connection(ConnectionId::new(connection.id))
        .await
        .unwrap();
}

#[tokio::test]
async fn connections_webhooks_rules_and_cron_schedules() {
    let Some(client) = client().await else { return };
    let suffix = uuid::Uuid::new_v4().simple().to_string();

    let connection = client
        .create_connection(&CreateStorageBackendRequest {
            name: format!("client-test-git-{suffix}"),
            description: None,
            connection_type: ConnectionType::Git,
            config: json!({ "repo_url": "https://github.com/paninfracon/stormchaser.git" }),
            aws_assume_role_arn: None,
            encrypted_credentials: None,
            is_default_sfs: false,
        })
        .await
        .unwrap();
    let connection_id = ConnectionId::new(connection.id);
    assert_eq!(
        client.get_connection(connection_id).await.unwrap().id,
        connection_id
    );
    client
        .update_connection(
            connection_id,
            &UpdateStorageBackendRequest {
                name: None,
                description: Some("changed".to_string()),
                connection_type: None,
                config: None,
                aws_assume_role_arn: None,
                encrypted_credentials: None,
                is_default_sfs: None,
            },
        )
        .await
        .unwrap();
    assert!(client
        .list_connections()
        .await
        .unwrap()
        .iter()
        .any(|c| c.id == connection_id));

    let webhook = client
        .create_webhook(&CreateWebhookRequest {
            name: format!("client-test-hook-{suffix}"),
            description: None,
            source_type: "generic".to_string(),
            secret_token: Some("a-secret-token-of-some-length".to_string()),
        })
        .await
        .unwrap();
    let webhook_id = WebhookId::new(webhook.id);
    client
        .update_webhook(
            webhook_id,
            &UpdateWebhookRequest {
                name: None,
                description: Some("changed".to_string()),
                source_type: None,
                secret_token: None,
                is_active: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        client
            .get_webhook(webhook_id)
            .await
            .unwrap()
            .description
            .as_deref(),
        Some("changed")
    );
    assert!(client
        .list_webhooks()
        .await
        .unwrap()
        .iter()
        .any(|w| w.id == webhook_id));

    let rule = client
        .create_event_rule(&CreateEventRuleRequest {
            name: format!("client-test-rule-{suffix}"),
            description: None,
            webhook_id,
            event_type_pattern: "push".to_string(),
            condition_expr: None,
            workflow_name: "wf".to_string(),
            connection: connection_id.to_string(),
            workflow_path: "wf.storm".to_string(),
            git_ref: "main".to_string(),
            input_mappings: Default::default(),
        })
        .await
        .unwrap();
    let rule_id = RuleId::new(rule.id);
    assert!(client
        .list_event_rules()
        .await
        .unwrap()
        .iter()
        .any(|r| r.id == rule_id));

    let cron = client
        .create_cron_workflow(&CreateCronWorkflowRequest {
            name: format!("client-test-cron-{suffix}"),
            description: None,
            cronspec: "0 3 * * *".to_string(),
            workflow_name: "wf".to_string(),
            connection: connection_id.to_string(),
            workflow_path: "wf.storm".to_string(),
            git_ref: "main".to_string(),
            inputs: json!({}),
        })
        .await
        .unwrap();
    assert!(client
        .list_cron_workflows()
        .await
        .unwrap()
        .iter()
        .any(|c| c.id == cron.id));

    client.delete_cron_workflow(cron.id).await.unwrap();
    client.delete_event_rule(rule_id).await.unwrap();
    client.delete_webhook(webhook_id).await.unwrap();
    client.delete_connection(connection_id).await.unwrap();
}

#[tokio::test]
async fn an_error_status_reaches_the_caller() {
    let Some(client) = client().await else { return };
    let err = client
        .get_run(stormchaser_model::RunId::new_v4())
        .await
        .expect_err("no such run");
    assert_eq!(err.status(), Some(reqwest::StatusCode::NOT_FOUND));

    let anonymous = Client::builder(client.api_url()).build().unwrap();
    let err = anonymous.session().await.expect_err("no token");
    assert_eq!(err.status(), Some(reqwest::StatusCode::UNAUTHORIZED));
}
