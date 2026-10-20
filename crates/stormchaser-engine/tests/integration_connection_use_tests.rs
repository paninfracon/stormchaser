//! The engine asks policy before using a connection on a run's behalf.
//!
//! The shipped policy lets every run use every connection, so these tests run OPA with a
//! stricter rule (owner only) to show that the engine asks and obeys.

#![expect(clippy::disallowed_methods, reason = "tests call the OPA they start")]

use serde_json::json;
use sqlx::PgPool;
use std::collections::HashMap;
use std::env::var;
use std::net::TcpListener;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_engine::authz;
use stormchaser_model::{Connection, RunId};
use stormchaser_opa::OpaClient;
use uuid::Uuid;

const OPA_IMAGE: &str = "openpolicyagent/opa:0.68.0";

const OWNER_ONLY_POLICY: &str = r#"package stormchaser

import rego.v1

default allow := false

allow if {
	input.action == "use"
	input.resource.type == "connection"
	input.resource.owner == input.subject.email
}
"#;

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

async fn start_opa(policy: &str) -> (ContainerGuard, tempfile::TempDir, String) {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let name = format!("opa-connection-use-test-{}", Uuid::new_v4());
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("policy.rego"), policy).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();

    let status = Command::new("docker")
        .args(["run", "-d", "--name", &name])
        .args(["-p", &format!("127.0.0.1:{port}:8181")])
        .args(["-v", &format!("{}:/etc/opa:ro", dir.path().display())])
        .arg(OPA_IMAGE)
        .args([
            "run",
            "--server",
            "--addr",
            "0.0.0.0:8181",
            "/etc/opa/policy.rego",
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
        dir,
        format!("http://127.0.0.1:{port}/v1/data/stormchaser/allow"),
    )
}

async fn pool() -> PgPool {
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    PgPool::connect(&db_url).await.unwrap()
}

async fn insert_run(pool: &PgPool, user: &str) -> RunId {
    let run_id = RunId::new_v4();
    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, 'connection-use-test', $2, 'running', 1, 'http://git.local', 'workflow.storm', 'main')")
        .bind(run_id)
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
    run_id
}

// One test: the policy client is installed once per process.
#[tokio::test]
async fn a_run_uses_a_connection_only_if_policy_lets_its_user() {
    let (_opa, _dir, opa_url) = start_opa(OWNER_ONLY_POLICY).await;
    authz::install(Arc::new(OpaClient::new(Some(opa_url), None)));
    let pool = pool().await;

    let tag = Uuid::new_v4().simple().to_string();
    let alice = format!("alice-{tag}@example.com");
    let bob = format!("bob-{tag}@example.com");
    let name = format!("conn-use-{tag}");
    sqlx::query("INSERT INTO connections (id, name, connection_type, config, created_by) VALUES ($1, $2, 'postgres', $3, $4)")
        .bind(Uuid::new_v4())
        .bind(&name)
        // Nothing listens here; a query that gets past policy fails to connect instead
        .bind(json!({"url": "postgres://nobody@127.0.0.1:1/none"}))
        .bind(&alice)
        .execute(&pool)
        .await
        .unwrap();
    let connection: Connection = stormchaser_engine::db::get_storage_backend_by_name(&pool, &name)
        .await
        .unwrap()
        .unwrap();

    // Asked directly, as a user
    authz::ensure_user_may_use(&pool, &alice, &connection)
        .await
        .expect("the owner may use it");
    let denied = authz::ensure_user_may_use(&pool, &bob, &connection)
        .await
        .unwrap_err();
    assert!(denied.to_string().contains("does not allow"), "{denied}");

    // Asked for a run: the run acts as whoever started it
    let alice_run = insert_run(&pool, &alice).await;
    let bob_run = insert_run(&pool, &bob).await;
    authz::ensure_run_may_use(&pool, alice_run, &connection)
        .await
        .expect("the owner's run may use it");
    assert!(authz::ensure_run_may_use(&pool, bob_run, &connection)
        .await
        .is_err());

    // A workflow query is refused before the engine connects anywhere
    let params = HashMap::from([
        ("connection".to_string(), name.clone()),
        ("query".to_string(), "SELECT 1".to_string()),
    ]);
    let refused =
        stormchaser_engine::query::execute_query("sql", &params, Some(&pool), None, Some(&bob))
            .await
            .unwrap_err();
    assert!(refused.to_string().contains("does not allow"), "{refused}");
    let attempted =
        stormchaser_engine::query::execute_query("sql", &params, Some(&pool), None, Some(&alice))
            .await
            .unwrap_err();
    assert!(
        !attempted.to_string().contains("does not allow"),
        "the owner gets past policy and fails on the unreachable database: {attempted}"
    );

    sqlx::query("DELETE FROM workflow_runs WHERE id = ANY($1)")
        .bind(vec![alice_run.into_inner(), bob_run.into_inner()])
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM connections WHERE name = $1")
        .bind(&name)
        .execute(&pool)
        .await
        .unwrap();
}
