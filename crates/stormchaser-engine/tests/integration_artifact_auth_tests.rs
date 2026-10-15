use serde_json::json;
use sqlx::PgPool;
use std::env::var;
use stormchaser_engine::handler::step::events::artifact_auth::artifact_push_auth;
use uuid::Uuid;
use wiremock::matchers::{basic_auth, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

/// A registry that answers `/v2/` with `challenge` and, at `/token`, trades the right
/// username and password for the token `scoped-token`.
async fn mock_registry(challenge: impl Fn(&str) -> String) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/"))
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header("WWW-Authenticate", challenge(&server.uri()).as_str()),
        )
        .mount(&server)
        .await;
    server
}

/// Inserts an OCI backend pointing at `registry`, and a run whose step `build` mounts a
/// volume with one artifact on it. Returns the run and step ids.
async fn insert_run(pool: &PgPool, registry: &MockServer, status: &str) -> (Uuid, Uuid) {
    insert_run_at(pool, registry, status, "build").await
}

/// A run whose step `step_name` is with a runner: `build` publishes the artifact `app`,
/// `inspect` mounts the same storage but publishes nothing.
async fn insert_run_at(
    pool: &PgPool,
    registry: &MockServer,
    status: &str,
    step_name: &str,
) -> (Uuid, Uuid) {
    let run_id = Uuid::new_v4();
    let step_id = Uuid::new_v4();
    let backend_name = format!("oci-auth-test-{}", run_id);
    let host = registry.uri().trim_start_matches("http://").to_string();

    sqlx::query("INSERT INTO connections (id, name, connection_type, config) VALUES ($1, $2, 'oci', $3)")
        .bind(Uuid::new_v4())
        .bind(&backend_name)
        .bind(json!({"registry": host, "plain_http": true, "username": "pusher", "password": "long-lived"}))
        .execute(pool)
        .await
        .unwrap();

    let dsl = format!(
        r#"
        stormchaser_dsl_version = "v1"
        workflow "artifact-auth-test" {{
            storage "ws" {{
                size = "1Gi"
                backend = "{backend_name}"
                artifact "app" {{
                    path = "bin/app"
                    retention = "7d"
                }}
            }}
            steps {{
                step "build" "RunContainer" {{
                    image = "alpine"
                    storage_mounts = [
                        {{ name = "ws", mount_path = "/workspace" }}
                    ]
                    artifacts = ["app"]
                    next = ["inspect"]
                }}
                step "inspect" "RunContainer" {{
                    image = "alpine"
                    storage_mounts = [
                        {{ name = "ws", mount_path = "/workspace" }}
                    ]
                }}
            }}
        }}
    "#
    );
    let workflow = stormchaser_dsl::StormchaserParser::new()
        .parse(&dsl)
        .expect("test workflow should parse");

    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, 'artifact-auth-test', 'test-user', 'running', 1, 'http://git.local', 'workflow.storm', 'main')")
        .bind(run_id)
        .execute(pool)
        .await
        .unwrap();
    stormchaser_engine::db::runs::insert_run_context(
        pool,
        stormchaser_model::RunId::new(run_id),
        "v1",
        serde_json::to_value(&workflow).unwrap(),
        Some(&dsl),
        json!({}),
    )
    .await
    .unwrap();
    // completion_token NULL: the step has been handed to a runner
    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, status, step_type, spec, completion_token) VALUES ($1, $2, $3, $4::step_status, 'RunContainer', $5, NULL)")
        .bind(step_id)
        .bind(run_id)
        .bind(step_name)
        .bind(status)
        .bind(json!({"image": "alpine", "storage_mounts": [{"name": "ws", "mount_path": "/workspace"}]}))
        .execute(pool)
        .await
        .unwrap();

    (run_id, step_id)
}

/// `payload` as a runner sends it: naming a runner the engine issued a key to, and
/// signed with that key.
async fn signed(pool: &PgPool, mut payload: serde_json::Value) -> serde_json::Value {
    let runner_id = format!("test-runner-{}", Uuid::new_v4());
    let key = [42u8; 32];
    stormchaser_engine::db::store_runner_session(pool, &runner_id, &key)
        .await
        .unwrap();
    payload["runner_id"] = json!(runner_id);
    stormchaser_model::runner_auth::sign_payload(
        payload,
        &key,
        stormchaser_model::nats::ARTIFACT_AUTH_SUBJECT,
    )
}

#[tokio::test]
async fn only_a_runner_signing_with_its_own_key_gets_a_token() {
    let pool = pool().await;
    let registry =
        mock_registry(|uri| format!(r#"Bearer realm="{uri}/token",service="mock-registry""#)).await;
    let (run_id, step_id) = insert_run(&pool, &registry, "running").await;

    // Unsigned, as anyone with the runner credential could send it
    let err = artifact_push_auth(
        &json!({"run_id": run_id, "step_id": step_id, "runner_id": "someone"}),
        &pool,
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("no key"), "{err}");

    // Signed, but not with the key of the runner it names
    let mut forged = signed(&pool, json!({"run_id": run_id, "step_id": step_id})).await;
    let victim = format!("test-runner-{}", Uuid::new_v4());
    stormchaser_engine::db::store_runner_session(&pool, &victim, &[7u8; 32])
        .await
        .unwrap();
    forged["runner_id"] = json!(victim);
    let err = artifact_push_auth(&forged, &pool).await.unwrap_err();
    assert!(err.to_string().contains("not signed by runner"), "{err}");
}

#[tokio::test]
async fn a_running_step_gets_a_token_for_its_own_repository_and_never_the_password() {
    let pool = pool().await;
    let registry =
        mock_registry(|uri| format!(r#"Bearer realm="{uri}/token",service="mock-registry""#)).await;
    let (run_id, step_id) = insert_run(&pool, &registry, "running").await;

    // The token endpoint answers only the stored credentials asking for this run's repository
    Mock::given(method("GET"))
        .and(path("/token"))
        .and(basic_auth("pusher", "long-lived"))
        .and(query_param("service", "mock-registry"))
        .and(query_param(
            "scope",
            format!("repository:{run_id}/ws:pull,push"),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"token": "scoped-token"})))
        .expect(1)
        .mount(&registry)
        .await;

    let config = artifact_push_auth(
        &signed(&pool, json!({"run_id": run_id, "step_id": step_id})).await,
        &pool,
    )
    .await
    .unwrap();

    let host = registry.uri().trim_start_matches("http://").to_string();
    assert_eq!(config["auths"][&host]["registrytoken"], "scoped-token");
    assert!(!config.to_string().contains("long-lived"));
}

#[tokio::test]
async fn a_step_that_publishes_no_artifacts_gets_no_push_token() {
    let pool = pool().await;
    let registry =
        mock_registry(|uri| format!(r#"Bearer realm="{uri}/token",service="mock-registry""#)).await;
    let (run_id, step_id) = insert_run_at(&pool, &registry, "running", "inspect").await;

    // It mounts the storage holding `app`, but does not list it in `artifacts`
    Mock::given(method("GET"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"token": "scoped-token"})))
        .expect(0)
        .mount(&registry)
        .await;

    let config = artifact_push_auth(
        &signed(&pool, json!({"run_id": run_id, "step_id": step_id})).await,
        &pool,
    )
    .await
    .unwrap();
    assert_eq!(config["auths"], json!({}));
}

#[tokio::test]
async fn credentials_are_refused_for_a_step_that_is_not_with_a_runner() {
    let pool = pool().await;
    let registry = mock_registry(|uri| format!(r#"Bearer realm="{uri}/token""#)).await;

    // Finished
    let (run_id, step_id) = insert_run(&pool, &registry, "succeeded").await;
    assert!(artifact_push_auth(
        &signed(&pool, json!({"run_id": run_id, "step_id": step_id})).await,
        &pool
    )
    .await
    .is_err());

    // Running, but named with another run
    let (_, step_id) = insert_run(&pool, &registry, "running").await;
    assert!(artifact_push_auth(
        &json!({"run_id": Uuid::new_v4(), "step_id": step_id}),
        &pool
    )
    .await
    .is_err());

    // Running, but one the engine completes itself
    let (run_id, step_id) = insert_run(&pool, &registry, "running").await;
    sqlx::query("UPDATE step_instances SET completion_token = 'engine-owned' WHERE id = $1")
        .bind(step_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(artifact_push_auth(
        &signed(&pool, json!({"run_id": run_id, "step_id": step_id})).await,
        &pool
    )
    .await
    .is_err());

    // Unknown step
    assert!(
        artifact_push_auth(&json!({"run_id": run_id, "step_id": Uuid::new_v4()}), &pool)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_registry_without_token_auth_gets_nothing_by_default() {
    let pool = pool().await;
    let registry = mock_registry(|_| r#"Basic realm="Registry""#.to_string()).await;
    let (run_id, step_id) = insert_run(&pool, &registry, "running").await;

    let err = artifact_push_auth(
        &signed(&pool, json!({"run_id": run_id, "step_id": step_id})).await,
        &pool,
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string().contains("does not issue scoped tokens"),
        "{err}"
    );
    assert!(!err.to_string().contains("long-lived"));
}
