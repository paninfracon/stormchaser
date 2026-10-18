//! A queued run whose resolution meets a transient failure (here an OPA server that
//! cannot be reached) stays resolving, so that the event's next delivery resolves it,
//! instead of failing for good on the first blip.

use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::env::var;
use std::path::Path;
use std::sync::Arc;
use stormchaser_engine::git_cache::{GitCache, GIT_ALLOWED_SCHEMES_ENV};
use stormchaser_engine::handler;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::RunId;
use stormchaser_tls::{TlsConfig, TlsReloader};

async fn setup_db() -> PgPool {
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap()
}

/// A repository whose `main` holds `workflow.storm`
fn repository_with_workflow(dir: &Path) {
    let repo = git2::Repository::init(dir).unwrap();
    std::fs::write(
        dir.join("workflow.storm"),
        r#"
        stormchaser_dsl_version = "v1"
        workflow "queued-retry" {
            steps {
                step "hello" "RunContainer" {
                    image = "alpine"
                    command = ["echo", "hello"]
                }
            }
        }
        "#,
    )
    .unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("workflow.storm")).unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let author = git2::Signature::now("test", "test@example.com").unwrap();
    let commit = repo
        .commit(None, &author, &author, "workflow", &tree, &[])
        .unwrap();
    repo.branch("main", &repo.find_commit(commit).unwrap(), true)
        .unwrap();
    repo.set_head("refs/heads/main").unwrap();
}

async fn status(pool: &PgPool, run_id: RunId) -> RunStatus {
    sqlx::query_scalar("SELECT status FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_transient_failure_leaves_the_run_for_the_next_delivery() {
    // This binary's one test: the setting is process-wide
    std::env::set_var(GIT_ALLOWED_SCHEMES_ENV, "file");

    let pool = setup_db().await;
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = stormchaser_model::nats::connect(nats_url).await.unwrap();
    let tls_reloader = Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap());
    let repo_dir = tempfile::tempdir().unwrap();
    repository_with_workflow(repo_dir.path());
    let git_dir = tempfile::tempdir().unwrap();
    let git_cache = Arc::new(GitCache::new(git_dir.path()));
    let run_id = RunId::new_v4();

    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, 'queued-retry', 'test', 'queued', 1, $2, 'workflow.storm', 'main')")
        .bind(run_id)
        .bind(format!("file://{}", repo_dir.path().display()))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO run_contexts (run_id, dsl_version, workflow_definition, source_code, inputs) VALUES ($1, '1.0', '{}', '', '{}')")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    // Nothing listens on port 1
    let unreachable_opa = Arc::new(OpaClient::new(
        Some("http://127.0.0.1:1/v1/data/stormchaser/allow".into()),
        None,
    ));
    let error = handler::handle_workflow_queued(
        run_id,
        json!({}),
        pool.clone(),
        git_cache.clone(),
        unreachable_opa,
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await
    .expect_err("an unreachable OPA server is an error to retry");
    assert!(
        stormchaser_engine::retry::is_transient(&error),
        "not transient: {error:#}"
    );
    assert_eq!(status(&pool, run_id).await, RunStatus::Resolving);

    // The event delivered again, OPA back
    handler::handle_workflow_queued(
        run_id,
        json!({}),
        pool.clone(),
        git_cache,
        Arc::new(OpaClient::new(None, None)),
        nats_client,
        tls_reloader,
    )
    .await
    .unwrap();
    assert_eq!(status(&pool, run_id).await, RunStatus::StartPending);

    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM outbox_events WHERE payload LIKE '%' || $1 || '%'")
        .bind(run_id.to_string())
        .execute(&pool)
        .await
        .unwrap();
}
