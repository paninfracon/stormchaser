use std::env;
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use stormchaser_model::auth::{ApiOpaContext, OpaAuthorizer, OpaClient};
use uuid::Uuid;

/// Pinned OPA image used by integration tests.
const OPA_IMAGE: &str = "openpolicyagent/opa:0.68.0";

/// Helper to get a random available port
fn get_free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

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
async fn test_mcp_opa_policy() {
    let port = get_free_port();
    let container_name = format!("opa-test-{}", Uuid::new_v4());

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let repo_root = Path::new(&manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let policy_dir = repo_root.join("deploy").join("opa");

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
        .status()
        .expect("Failed to start OPA container");

    assert!(status.success(), "Failed to start OPA container");
    let _guard = ContainerGuard {
        name: container_name.clone(),
    };

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
    if !ready {
        let out = Command::new("docker")
            .arg("logs")
            .arg(&container_name)
            .output()
            .unwrap();
        panic!(
            "OPA container did not become ready in time. Stdout: {} Stderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let opa_url = format!("http://127.0.0.1:{}/v1/data/stormchaser/allow", port);
    let client = OpaClient::new(Some(opa_url), None);

    // Rule 1: Block calls via the MCP server that are not read-only
    // Unauthenticated GET (read-only) -> allow
    let ctx = ApiOpaContext {
        path: "/api/v1/runs",
        method: "GET",
        token: None,
    };
    assert!(
        client.check(ctx).await.unwrap(),
        "Unauth GET should be allowed"
    );

    // Unauthenticated POST (non-read-only) -> deny
    let ctx = ApiOpaContext {
        path: "/api/v1/runs",
        method: "POST",
        token: None,
    };
    assert!(
        !client.check(ctx).await.unwrap(),
        "Unauth POST should be denied"
    );

    // Unauthenticated POST to /api/v1/auth/login -> allow
    let ctx = ApiOpaContext {
        path: "/api/v1/auth/login",
        method: "POST",
        token: None,
    };
    assert!(
        client.check(ctx).await.unwrap(),
        "Unauth POST to login should be allowed"
    );

    // Unauthenticated POST to /api/v1/auth/exchange -> allow
    let ctx = ApiOpaContext {
        path: "/api/v1/auth/exchange",
        method: "POST",
        token: None,
    };
    assert!(
        client.check(ctx).await.unwrap(),
        "Unauth POST to exchange should be allowed"
    );

    // Unauthenticated POST to /api/v1/mcp/messages -> allow
    let ctx = ApiOpaContext {
        path: "/api/v1/mcp/messages",
        method: "POST",
        token: None,
    };
    assert!(
        client.check(ctx).await.unwrap(),
        "Unauth POST to mcp messages should be allowed"
    );

    // Authenticated POST to /api/v1/runs -> allow
    let ctx = ApiOpaContext {
        path: "/api/v1/runs",
        method: "POST",
        token: Some("dummy_token"),
    };
    assert!(
        client.check(ctx).await.unwrap(),
        "Auth POST to runs should be allowed"
    );

    // Rule 2: Block calls to the MCP server that are not read-only
    // Authenticated POST to /api/v1/mcp/messages -> allow
    let ctx = ApiOpaContext {
        path: "/api/v1/mcp/messages",
        method: "POST",
        token: Some("dummy_token"),
    };
    assert!(
        client.check(ctx).await.unwrap(),
        "Auth POST to mcp messages should be allowed"
    );

    // Authenticated POST to /api/v1/mcp/something_else -> deny
    let ctx = ApiOpaContext {
        path: "/api/v1/mcp/something_else",
        method: "POST",
        token: Some("dummy_token"),
    };
    assert!(
        !client.check(ctx).await.unwrap(),
        "Auth POST to other mcp paths should be denied"
    );

    // Authenticated GET to /api/v1/mcp/sse -> allow
    let ctx = ApiOpaContext {
        path: "/api/v1/mcp/sse",
        method: "GET",
        token: Some("dummy_token"),
    };
    assert!(
        client.check(ctx).await.unwrap(),
        "Auth GET to mcp should be allowed"
    );

    // Unauthenticated GET to /api/v1/mcp/sse -> allow
    let ctx = ApiOpaContext {
        path: "/api/v1/mcp/sse",
        method: "GET",
        token: None,
    };
    assert!(
        client.check(ctx).await.unwrap(),
        "Unauth GET to mcp should be allowed"
    );
}
