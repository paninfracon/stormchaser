use std::net::TcpListener;
use std::process::Command;
use std::time::Duration;
use stormchaser_model::auth::{ApiOpaContext, OpaClient};
use uuid::Uuid;

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

/// Helper to mock a JWT token (only signature is mocked, claims are real for testing)
fn mock_token(email: &str, groups: Vec<&str>) -> String {
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize)]
    struct Claims {
        email: String,
        groups: Vec<String>,
        exp: usize,
    }

    let claims = Claims {
        email: email.to_string(),
        groups: groups.iter().map(|&s| s.to_string()).collect(),
        exp: 2000000000, // Future
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret("secret".as_ref()),
    )
    .unwrap()
}

#[tokio::test]
async fn test_enterprise_opa_rbac_integration() {
    let port = get_free_port();
    let container_name = format!("opa-test-{}", Uuid::new_v4());

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
        .arg("openpolicyagent/opa:latest")
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

    // 3. Test Cases (API Context)
    let admin_token = mock_token("admin@paninfracon.net", vec!["Okta-Global-Admins"]);
    let dev_token = mock_token("dev@paninfracon.net", vec!["Okta-Engineering"]);
    let ops_token = mock_token("ops@paninfracon.net", vec!["Okta-SRE"]);
    let sec_token = mock_token("sec@paninfracon.net", vec!["EntraID-SecOps"]);
    let hacker_token = mock_token("hacker@evil.com", vec!["Okta-Global-Admins"]); // Admin group, bad domain

    // --- Admin Tests ---
    let admin_ctx = ApiOpaContext {
        path: "/api/v1/anything/dangerous",
        method: "DELETE",
        token: Some(&admin_token),
    };
    assert!(
        client.check(admin_ctx).await.unwrap(),
        "Admin should be allowed everything"
    );

    // --- Hacker Tests ---
    let hacker_ctx = ApiOpaContext {
        path: "/api/v1/runs",
        method: "GET",
        token: Some(&hacker_token),
    };
    assert!(
        !client.check(hacker_ctx).await.unwrap(),
        "Hacker domain should be denied despite groups"
    );

    // --- Developer Tests ---
    let dev_start_run_ctx = ApiOpaContext {
        path: "/api/v1/runs",
        method: "POST",
        token: Some(&dev_token),
    };
    assert!(
        client.check(dev_start_run_ctx).await.unwrap(),
        "Dev can start runs"
    );

    let dev_delete_webhook_ctx = ApiOpaContext {
        path: "/api/v1/webhooks/123",
        method: "DELETE",
        token: Some(&dev_token),
    };
    assert!(
        !client.check(dev_delete_webhook_ctx).await.unwrap(),
        "Dev cannot delete webhooks"
    );

    // --- Operator Tests ---
    let ops_view_run_ctx = ApiOpaContext {
        path: "/api/v1/runs",
        method: "GET",
        token: Some(&ops_token),
    };
    assert!(
        client.check(ops_view_run_ctx).await.unwrap(),
        "Ops can view runs"
    );

    let ops_start_run_ctx = ApiOpaContext {
        path: "/api/v1/runs",
        method: "POST",
        token: Some(&ops_token),
    };
    assert!(
        !client.check(ops_start_run_ctx).await.unwrap(),
        "Ops cannot start runs"
    );

    let ops_delete_webhook_ctx = ApiOpaContext {
        path: "/api/v1/webhooks/123",
        method: "DELETE",
        token: Some(&ops_token),
    };
    assert!(
        client.check(ops_delete_webhook_ctx).await.unwrap(),
        "Ops can delete webhooks"
    );

    // --- Security Tests ---
    let sec_view_reports_ctx = ApiOpaContext {
        path: "/api/v1/reports",
        method: "GET",
        token: Some(&sec_token),
    };
    assert!(
        client.check(sec_view_reports_ctx).await.unwrap(),
        "Security can view reports"
    );

    let sec_view_runs_ctx = ApiOpaContext {
        path: "/api/v1/runs",
        method: "GET",
        token: Some(&sec_token),
    };
    assert!(
        client.check(sec_view_runs_ctx).await.unwrap(),
        "Security can view runs"
    );

    let sec_start_runs_ctx = ApiOpaContext {
        path: "/api/v1/runs",
        method: "POST",
        token: Some(&sec_token),
    };
    assert!(
        !client.check(sec_start_runs_ctx).await.unwrap(),
        "Security cannot start runs"
    );
}
