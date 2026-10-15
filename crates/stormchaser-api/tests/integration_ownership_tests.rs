//! Object-level authorization, end to end: the API router, the shipped policy
//! (`deploy/opa`) in a real OPA, and the database.

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::env::var;
use std::net::TcpListener;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_api::{app, AppState};
use stormchaser_model::auth::OpaClient;
use tower::ServiceExt;
use uuid::Uuid;

/// Pinned OPA image, the same one the engine's policy tests use.
const OPA_IMAGE: &str = "openpolicyagent/opa:0.68.0";

struct ContainerGuard {
    name: String,
}

impl Drop for ContainerGuard {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", &self.name])
            .status();
    }
}

/// Starts OPA with the shipped policy and API lock-down, and `settings` as its data.
async fn start_opa(settings: Value) -> (ContainerGuard, tempfile::TempDir, String) {
    start_opa_with(None, settings).await
}

/// Starts OPA as [`start_opa`] does, with `policy` in place of the shipped
/// `policy.rego` when given.
async fn start_opa_with(
    policy: Option<&str>,
    settings: Value,
) -> (ContainerGuard, tempfile::TempDir, String) {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let name = format!("opa-ownership-test-{}", Uuid::new_v4());
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let policy_dir = repo_root.join("deploy").join("opa");

    let settings_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        settings_dir.path().join("settings.json"),
        json!({ "stormchaser_settings": settings }).to_string(),
    )
    .unwrap();
    let policy_file = match policy {
        Some(policy) => {
            std::fs::write(settings_dir.path().join("policy.rego"), policy).unwrap();
            "/etc/opa-settings/policy.rego"
        }
        None => "/etc/opa/policy.rego",
    };
    // The OPA image runs as a non-root user
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(settings_dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();

    let status = Command::new("docker")
        .args(["run", "-d", "--name", &name])
        .args(["-p", &format!("127.0.0.1:{port}:8181")])
        .args(["-v", &format!("{}:/etc/opa:ro", policy_dir.display())])
        .args([
            "-v",
            &format!("{}:/etc/opa-settings:ro", settings_dir.path().display()),
        ])
        .arg(OPA_IMAGE)
        .args(["run", "--server", "--addr", "0.0.0.0:8181"])
        .arg("--authorization=basic")
        .args([
            policy_file,
            "/etc/opa/authz.rego",
            "/etc/opa-settings/settings.json",
        ])
        .status()
        .expect("Failed to start OPA container");
    assert!(status.success(), "Failed to start OPA container");
    let guard = ContainerGuard { name };

    let client = reqwest::Client::new();
    let mut ready = false;
    for _ in 0..40 {
        if let Ok(resp) = client
            .get(format!("http://127.0.0.1:{port}/health"))
            .send()
            .await
        {
            if resp.status().is_success() {
                ready = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(ready, "OPA container did not become ready in time");

    (
        guard,
        settings_dir,
        format!("http://127.0.0.1:{port}/v1/data/stormchaser/allow"),
    )
}

async fn state_with_policy(opa_url: String) -> AppState {
    std::env::set_var("API_RATE_LIMIT_BURST_SIZE", "100000");
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url)
        .await
        .unwrap();
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats = stormchaser_model::nats::connect(nats_url).await.unwrap();

    AppState {
        pool,
        nats,
        opa: Arc::new(OpaClient::new(Some(opa_url), None)),
        oidc_config: None,
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
        api_base_url: "http://localhost:3000".to_string(),
        mcp_allowed_hosts: Vec::new(),
    }
}

/// A bearer header for a user whose email is `<name>-<tag>@example.com`.
fn bearer(email: &str) -> String {
    let claims = stormchaser_model::auth::Claims {
        sub: format!("sub-of-{email}"),
        email: Some(email.to_string()),
        exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
        groups: Vec::new(),
    };
    let token = stormchaser_api::sign_local_token(&claims, stormchaser_api::JWT_SECRET).unwrap();
    format!("Bearer {token}")
}

async fn call(
    router: &axum::Router,
    method: Method,
    path: &str,
    auth: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("Authorization", auth)
        .header("Content-Type", "application/json")
        .body(match body {
            Some(json) => Body::from(json.to_string()),
            None => Body::empty(),
        })
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn ids(list: &Value) -> Vec<String> {
    list.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn users_see_and_change_only_what_they_own() {
    let tag = Uuid::new_v4().simple().to_string();
    let alice_email = format!("alice-{tag}@example.com");
    let bob_email = format!("bob-{tag}@example.com");
    let root_email = format!("root-{tag}@example.com");
    let (alice, bob, root) = (
        bearer(&alice_email),
        bearer(&bob_email),
        bearer(&root_email),
    );

    let (_opa, _settings, opa_url) = start_opa(json!({ "admins": [root_email] })).await;
    let state = state_with_policy(opa_url).await;
    let pool = state.pool.clone();
    let router = app(state);

    // --- A webhook is private to its creator ---------------------------------------
    let (status, created) = call(
        &router,
        Method::POST,
        "/api/v1/webhooks",
        &alice,
        Some(json!({"name": format!("hook-{tag}"), "source_type": "generic", "secret_token": "s3cret"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let hook = created["id"].as_str().unwrap().to_string();
    let hook_path = format!("/api/v1/webhooks/{hook}");

    assert_eq!(
        call(&router, Method::GET, &hook_path, &alice, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&router, Method::GET, &hook_path, &bob, None).await.0,
        StatusCode::FORBIDDEN
    );
    // Percent-encoding part of the id does not get round the check
    let encoded_hook_path = format!("/api/v1/webhooks/{}", hook.replacen('-', "%2D", 1));
    assert_eq!(
        call(&router, Method::GET, &encoded_hook_path, &bob, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&router, Method::GET, &encoded_hook_path, &alice, None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &router,
            Method::PATCH,
            &hook_path,
            &bob,
            Some(json!({"name": "mine now"}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&router, Method::DELETE, &hook_path, &bob, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    let (_, listed) = call(&router, Method::GET, "/api/v1/webhooks", &alice, None).await;
    assert!(ids(&listed).contains(&hook));
    let (_, listed) = call(&router, Method::GET, "/api/v1/webhooks", &bob, None).await;
    assert!(
        !ids(&listed).contains(&hook),
        "a list must not show what a get refuses"
    );

    // An admin may do anything
    assert_eq!(
        call(&router, Method::GET, &hook_path, &root, None).await.0,
        StatusCode::OK
    );
    let (_, listed) = call(&router, Method::GET, "/api/v1/webhooks", &root, None).await;
    assert!(ids(&listed).contains(&hook));

    // --- A connection is shared for reading, not for changing ----------------------
    let (status, created) = call(
        &router,
        Method::POST,
        "/api/v1/connections",
        &alice,
        Some(json!({
            "name": format!("conn-{tag}"), "connection_type": "s3",
            "config": {"bucket": "b"}, "is_default_sfs": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let conn = created["id"].as_str().unwrap().to_string();
    let conn_path = format!("/api/v1/connections/{conn}");

    assert_eq!(
        call(&router, Method::GET, &conn_path, &bob, None).await.0,
        StatusCode::OK
    );
    let (_, listed) = call(&router, Method::GET, "/api/v1/connections", &bob, None).await;
    assert!(ids(&listed).contains(&conn));
    assert_eq!(
        call(
            &router,
            Method::PATCH,
            &conn_path,
            &bob,
            Some(json!({"description": "x"}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&router, Method::DELETE, &conn_path, &bob, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    // --- Only an admin may choose the default storage backend ----------------------
    // (Only refusals are tested: the default is shared by every test using this database.)
    assert_eq!(
        call(
            &router,
            Method::PATCH,
            &conn_path,
            &alice,
            Some(json!({"is_default_sfs": true}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN,
        "alice made her connection the default storage backend"
    );
    let (status, _) = call(
        &router,
        Method::POST,
        "/api/v1/connections",
        &alice,
        Some(json!({
            "name": format!("default-{tag}"), "connection_type": "s3",
            "config": {"bucket": "b"}, "is_default_sfs": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (_, listed) = call(&router, Method::GET, "/api/v1/connections", &alice, None).await;
    assert!(!listed
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["name"] == format!("default-{tag}")));

    // --- A rule can only be attached to a webhook its creator may change ------------
    let (status, created) = call(
        &router,
        Method::POST,
        "/api/v1/connections",
        &alice,
        Some(json!({
            "name": format!("repo-{tag}"), "connection_type": "git",
            "config": {"repo_url": "https://git.example.com/repo.git"}, "is_default_sfs": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let repo = created["id"].as_str().unwrap().to_string();
    let rule = |webhook: &str| {
        json!({
            "name": format!("rule-{tag}"), "webhook_id": webhook,
            "event_type_pattern": ".*", "workflow_name": "w", "connection": repo,
            "workflow_path": "w.storm", "git_ref": "main", "input_mappings": {}
        })
    };
    assert_eq!(
        call(
            &router,
            Method::POST,
            "/api/v1/rules",
            &bob,
            Some(rule(&hook))
        )
        .await
        .0,
        StatusCode::FORBIDDEN,
        "bob attached a rule to alice's webhook"
    );
    assert_eq!(
        call(
            &router,
            Method::POST,
            "/api/v1/rules",
            &bob,
            Some(rule(&Uuid::new_v4().to_string()))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &router,
            Method::POST,
            "/api/v1/rules",
            &alice,
            Some(rule(&hook))
        )
        .await
        .0,
        StatusCode::CREATED
    );

    // --- A run and everything under it belong to whoever started it ----------------
    let run_id = Uuid::new_v4();
    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, 'ownership-test', $2, 'running', 1, 'http://git.local', 'workflow.storm', 'main')")
        .bind(run_id)
        .bind(&alice_email)
        .execute(&pool)
        .await
        .unwrap();
    let run_path = format!("/api/v1/runs/{run_id}");
    let step = Uuid::new_v4();
    for sub in ["", "/artifacts", "/reports", "/summaries"] {
        let path = format!("{run_path}{sub}");
        assert_eq!(
            call(&router, Method::GET, &path, &bob, None).await.0,
            StatusCode::FORBIDDEN,
            "{path}"
        );
        assert_ne!(
            call(&router, Method::GET, &path, &alice, None).await.0,
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
    assert_eq!(
        call(
            &router,
            Method::GET,
            &format!("{run_path}/steps/{step}/logs"),
            &bob,
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&router, Method::DELETE, &run_path, &bob, None).await.0,
        StatusCode::FORBIDDEN
    );

    let (_, listed) = call(&router, Method::GET, "/api/v1/runs?limit=100", &bob, None).await;
    assert!(!ids(&listed).contains(&run_id.to_string()));

    // --- Objects with no owner: readable by all, changed only by admins ------------
    let legacy = Uuid::new_v4();
    sqlx::query("INSERT INTO webhooks (id, name, source_type) VALUES ($1, $2, 'generic')")
        .bind(legacy)
        .bind(format!("legacy-{tag}"))
        .execute(&pool)
        .await
        .unwrap();
    let legacy_path = format!("/api/v1/webhooks/{legacy}");
    assert_eq!(
        call(&router, Method::GET, &legacy_path, &bob, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&router, Method::DELETE, &legacy_path, &bob, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert!(call(&router, Method::DELETE, &legacy_path, &root, None)
        .await
        .0
        .is_success());

    // --- Owners can clean up after themselves --------------------------------------
    assert!(call(&router, Method::DELETE, &hook_path, &alice, None)
        .await
        .0
        .is_success());
    assert!(call(&router, Method::DELETE, &conn_path, &alice, None)
        .await
        .0
        .is_success());
    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_report_is_only_served_under_its_own_run() {
    let tag = Uuid::new_v4().simple().to_string();
    let alice_email = format!("alice-{tag}@example.com");
    let bob_email = format!("bob-{tag}@example.com");

    let (_opa, _settings, opa_url) = start_opa(json!({})).await;
    let state = state_with_policy(opa_url).await;
    let pool = state.pool.clone();
    let router = app(state);

    // Each has a run; Alice's has a report
    let (alice_run, bob_run, step, report) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    for (run, owner) in [(alice_run, &alice_email), (bob_run, &bob_email)] {
        sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, 'report-owner-test', $2, 'running', 1, 'http://git.local', 'workflow.storm', 'main')")
            .bind(run)
            .bind(owner)
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, status, step_type) VALUES ($1, $2, 'test', 'running', 'RunContainer')")
        .bind(step)
        .bind(alice_run)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO step_test_reports (id, run_id, step_instance_id, report_name, file_name, format, content, checksum) VALUES ($1, $2, $3, 'unit', 'r.xml', 'junit', 'alice-only', 'x')")
        .bind(report)
        .bind(alice_run)
        .bind(step)
        .execute(&pool)
        .await
        .unwrap();

    let alice = bearer(&alice_email);
    let bob = bearer(&bob_email);
    let (status, body) = call(
        &router,
        Method::GET,
        &format!("/api/v1/runs/{alice_run}/reports/{report}"),
        &alice,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["content"], "alice-only");

    // Bob may read his own run, but naming Alice's report under it gets him nothing
    let (status, _) = call(
        &router,
        Method::GET,
        &format!("/api/v1/runs/{bob_run}/reports/{report}"),
        &bob,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    sqlx::query("DELETE FROM workflow_runs WHERE id = ANY($1)")
        .bind(vec![alice_run, bob_run])
        .execute(&pool)
        .await
        .unwrap();
}

/// Allows everything except using a connection someone else owns.
const OWNER_ONLY_USE_POLICY: &str = r#"
package stormchaser

import rego.v1

default allow := false

allow if not input.action

allow if {
	input.action
	input.action != "use"
}

allow if {
	input.action == "use"
	input.resource.owner == input.subject.email
}
"#;

#[tokio::test]
async fn starting_a_run_needs_use_of_its_git_connection() {
    let tag = Uuid::new_v4().simple().to_string();
    let alice_email = format!("alice-{tag}@example.com");
    let bob_email = format!("bob-{tag}@example.com");
    let (alice, bob) = (bearer(&alice_email), bearer(&bob_email));

    let (_opa, _settings, opa_url) = start_opa_with(Some(OWNER_ONLY_USE_POLICY), json!({})).await;
    let state = state_with_policy(opa_url).await;
    let pool = state.pool.clone();
    let router = app(state);

    let repo_name = format!("repo-{tag}");
    let (status, created) = call(
        &router,
        Method::POST,
        "/api/v1/connections",
        &alice,
        Some(json!({
            "name": repo_name, "connection_type": "git",
            "config": {"repo_url": "https://git.example.com/repo.git"}, "is_default_sfs": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let repo = created["id"].as_str().unwrap().to_string();

    let enqueue = |connection: &str| {
        json!({
            "workflow_name": format!("w-{tag}"), "connection": connection,
            "workflow_path": "w.storm", "git_ref": "main", "inputs": {}
        })
    };
    for connection in [&repo, &repo_name] {
        assert_eq!(
            call(
                &router,
                Method::POST,
                "/api/v1/runs",
                &bob,
                Some(enqueue(connection))
            )
            .await
            .0,
            StatusCode::FORBIDDEN,
            "bob started a run from alice's connection {connection}"
        );
    }
    let (status, started) = call(
        &router,
        Method::POST,
        "/api/v1/runs",
        &alice,
        Some(enqueue(&repo)),
    )
    .await;
    assert!(status.is_success(), "{status}: {started}");

    sqlx::query("DELETE FROM workflow_runs WHERE workflow_name = $1")
        .bind(format!("w-{tag}"))
        .execute(&pool)
        .await
        .unwrap();
    assert!(call(
        &router,
        Method::DELETE,
        &format!("/api/v1/connections/{repo}"),
        &alice,
        None
    )
    .await
    .0
    .is_success());
}
