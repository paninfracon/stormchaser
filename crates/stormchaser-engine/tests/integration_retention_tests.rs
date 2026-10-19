//! Archived runs are deleted, with everything archived with them, once they are older
//! than the archive retention.

mod common;

use common::setup_db;
use std::time::Duration;
use stormchaser_engine::workers::purge_expired_runs;
use uuid::Uuid;

/// An archived run that finished `days_ago`, with a step and its history
async fn archived_run(pool: &sqlx::PgPool, days_ago: i32) -> (Uuid, Uuid) {
    let run_id = Uuid::new_v4();
    let step_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO archived_workflow_runs (id, workflow_name, status, version, fencing_token, created_at, updated_at, finished_at, repo_url, workflow_path, git_ref, initiating_user) \
         VALUES ($1, 'retention-test', 'succeeded', 1, 1, NOW() - make_interval(days => $2), NOW() - make_interval(days => $2), NOW() - make_interval(days => $2), '', '', '', 'test')",
    )
    .bind(run_id)
    .bind(days_ago)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO archived_step_instances (id, run_id, step_name, step_type, status, created_at) VALUES ($1, $2, 'build', 'RunContainer', 'succeeded', NOW())",
    )
    .bind(step_id)
    .bind(run_id)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO archived_step_status_history (id, step_instance_id, status, created_at) VALUES ((random() * 1e15)::bigint, $1, 'succeeded', NOW())")
        .bind(step_id)
        .execute(pool)
        .await
        .unwrap();
    (run_id, step_id)
}

async fn exists(pool: &sqlx::PgPool, query: &'static str, id: Uuid) -> bool {
    sqlx::query_scalar::<_, i64>(query)
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
        > 0
}

#[tokio::test]
async fn runs_older_than_the_retention_are_deleted_and_newer_ones_kept() {
    let pool = setup_db().await;
    let (old_run, old_step) = archived_run(&pool, 400).await;
    let (new_run, _) = archived_run(&pool, 1).await;

    purge_expired_runs(&pool, Duration::from_secs(365 * 86400))
        .await
        .unwrap();

    const RUN: &str = "SELECT COUNT(*) FROM archived_workflow_runs WHERE id = $1";
    assert!(!exists(&pool, RUN, old_run).await);
    assert!(
        !exists(
            &pool,
            "SELECT COUNT(*) FROM archived_step_instances WHERE id = $1",
            old_step
        )
        .await
    );
    assert!(
        !exists(
            &pool,
            "SELECT COUNT(*) FROM archived_step_status_history WHERE step_instance_id = $1",
            old_step
        )
        .await
    );
    assert!(exists(&pool, RUN, new_run).await);

    sqlx::query("DELETE FROM archived_step_status_history WHERE step_instance_id IN (SELECT id FROM archived_step_instances WHERE run_id = $1)")
        .bind(new_run)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM archived_workflow_runs WHERE id = $1")
        .bind(new_run)
        .execute(&pool)
        .await
        .unwrap();
}
