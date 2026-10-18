//! A run whose resolution meets a transient failure (here an OPA server that cannot be
//! reached) is resolved on the event's next delivery, instead of failing for good, or
//! for a direct run vanishing, on the first blip.

mod common;

use common::{delete_run, nats, setup_db, tls_reloader};
use serde_json::json;
use sqlx::PgPool;
use std::path::Path;
use std::sync::Arc;
use stormchaser_engine::git_cache::{GitCache, GIT_ALLOWED_SCHEMES_ENV};
use stormchaser_engine::handler;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::RunId;

/// An OPA client for a server that is not there: nothing listens on port 1
fn unreachable_opa() -> Arc<OpaClient> {
    Arc::new(OpaClient::new(
        Some("http://127.0.0.1:1/v1/data/stormchaser/allow".into()),
        None,
    ))
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
    let nats_client = nats().await;
    let tls_reloader = tls_reloader().await;
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

    let error = handler::handle_workflow_queued(
        run_id,
        json!({}),
        pool.clone(),
        git_cache.clone(),
        unreachable_opa(),
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

    delete_run(&pool, run_id).await;
}

#[tokio::test]
async fn a_direct_run_is_created_on_the_next_delivery_after_a_transient_failure() {
    let pool = setup_db().await;
    let nats_client = nats().await;
    let run_id = RunId::new_v4();
    let payload = json!({
        "run_id": run_id,
        "dsl": r#"
            stormchaser_dsl_version = "v1"
            workflow "direct-retry" {
                steps {
                    step "hello" "RunContainer" {
                        image = "alpine"
                        command = ["echo", "hello"]
                    }
                }
            }
        "#,
        "inputs": {},
        "initiating_user": "test"
    });

    let error = handler::handle_workflow_direct(
        payload.clone(),
        pool.clone(),
        unreachable_opa(),
        nats_client.clone(),
    )
    .await
    .expect_err("an unreachable OPA server is an error to retry");
    assert!(
        stormchaser_engine::retry::is_transient(&error),
        "not transient: {error:#}"
    );
    let created: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM workflow_runs WHERE id = $1)")
            .bind(run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!created);

    // The event delivered again, OPA back
    handler::handle_workflow_direct(
        payload,
        pool.clone(),
        Arc::new(OpaClient::new(None, None)),
        nats_client,
    )
    .await
    .unwrap();
    assert_eq!(status(&pool, run_id).await, RunStatus::StartPending);

    delete_run(&pool, run_id).await;
}
