use serde_json::json;
use std::net::TcpListener;
use std::process::Command;
use std::time::Duration;
use stormchaser_model::auth::{EngineOpaContext, OpaClient};
use uuid::Uuid;

/// Pinned OPA image used by integration tests.  Update this constant (and re-run
/// the tests locally) when upgrading OPA to ensure CI remains reproducible.
const OPA_IMAGE: &str = "openpolicyagent/opa:0.68.0";

/// Helper to get a random available port
fn get_free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// RAII guard that ensures a Docker container is removed even if the test panics.
struct ContainerGuard {
    name: String,
}

impl Drop for ContainerGuard {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .arg("rm")
            .arg("-f")
            .arg(&self.name)
            .status();
    }
}

#[tokio::test]
async fn test_enterprise_opa_abac_integration() {
    let port = get_free_port();
    let container_name = format!("opa-engine-test-{}", Uuid::new_v4());

    // 1. Start OPA Container with Enterprise Policies
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let repo_root = std::path::Path::new(&manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let policy_dir = repo_root.join("docs").join("enterprise-rbac");

    let status = Command::new("docker")
        .arg("run")
        .arg("-d")
        .arg("--name")
        .arg(&container_name)
        .arg("-p")
        .arg(format!("{}:8181", port))
        .arg("-v")
        .arg(format!("{}:/etc/opa:ro", policy_dir.display()))
        .arg(OPA_IMAGE)
        .arg("run")
        .arg("--server")
        .arg("--addr")
        .arg("0.0.0.0:8181")
        .arg("/etc/opa/policy.rego")
        .arg("/etc/opa/roles.json")
        .status()
        .expect("Failed to start OPA container");

    assert!(status.success(), "Failed to start OPA container");
    // Guard ensures `docker rm -f` runs on drop, even if the test panics.
    let _guard = ContainerGuard {
        name: container_name.clone(),
    };

    // Wait for OPA to be healthy
    let health_url = format!("http://127.0.0.1:{}/health", port);
    let client = reqwest::Client::new();
    let mut ready = false;
    for _ in 0..30 {
        if let Ok(resp) = client.get(&health_url).send().await {
            if resp.status().is_success() {
                ready = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(ready, "OPA container did not become ready in time");

    // 2. Setup Client pointing to the specific package entrypoint
    let opa_url = format!(
        "http://127.0.0.1:{}/v1/data/stormchaser/enterprise/allow",
        port
    );
    let client = OpaClient::new(Some(opa_url), None);

    // 3. Test Cases (Engine Context)

    // --- ABAC Example 1: Deployment to Production ---
    let prod_ctx = EngineOpaContext {
        run_id: Uuid::new_v4(),
        initiating_user: "ops@paninfracon.net".to_string(), // Operator role
        workflow_ast: json!({ "step_type": "Log" }),
        inputs: json!({ "env": "production" }),
    };
    assert!(
        client.check_context(prod_ctx).await.unwrap(),
        "Operator can deploy to production"
    );

    let dev_prod_ctx = EngineOpaContext {
        run_id: Uuid::new_v4(),
        initiating_user: "dev@paninfracon.net".to_string(), // Developer role
        workflow_ast: json!({ "step_type": "Log" }),
        inputs: json!({ "env": "production" }),
    };
    assert!(
        !client.check_context(dev_prod_ctx).await.unwrap(),
        "Developer cannot deploy to production"
    );

    let dev_staging_ctx = EngineOpaContext {
        run_id: Uuid::new_v4(),
        initiating_user: "dev@paninfracon.net".to_string(), // Developer role
        workflow_ast: json!({ "step_type": "Log" }),
        inputs: json!({ "env": "staging" }),
    };
    assert!(
        client.check_context(dev_staging_ctx).await.unwrap(),
        "Developer can deploy to staging"
    );

    // --- ABAC Example 2: Privileged Containers ---
    let privileged_container_ctx = EngineOpaContext {
        run_id: Uuid::new_v4(),
        initiating_user: "admin@paninfracon.net".to_string(), // Admin role
        workflow_ast: json!({
            "step_type": "RunContainer",
            "config": {
                "privileged": true
            }
        }),
        inputs: json!({}),
    };
    assert!(
        client
            .check_context(privileged_container_ctx)
            .await
            .unwrap(),
        "Admin can run privileged container"
    );

    let dev_privileged_ctx = EngineOpaContext {
        run_id: Uuid::new_v4(),
        initiating_user: "dev@paninfracon.net".to_string(), // Developer role
        workflow_ast: json!({
            "step_type": "RunContainer",
            "config": {
                "privileged": true
            }
        }),
        inputs: json!({}),
    };
    assert!(
        !client.check_context(dev_privileged_ctx).await.unwrap(),
        "Developer cannot run privileged container"
    );

    let dev_unprivileged_ctx = EngineOpaContext {
        run_id: Uuid::new_v4(),
        initiating_user: "dev@paninfracon.net".to_string(), // Developer role
        workflow_ast: json!({
            "step_type": "RunContainer",
            "config": {
                "privileged": false
            }
        }),
        inputs: json!({}),
    };
    assert!(
        client.check_context(dev_unprivileged_ctx).await.unwrap(),
        "Developer can run unprivileged container"
    );
}
