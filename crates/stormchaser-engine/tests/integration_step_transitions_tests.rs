//! The database owns a step's status: it refuses moves a step may not make, and records
//! every change in the step's history, whichever statement makes it.

mod common;

use common::{delete_run, setup_db};
use stormchaser_model::step::StepStatus;
use stormchaser_model::{RunId, StepInstanceId};

async fn insert_run_and_step(pool: &sqlx::PgPool, status: &str) -> (RunId, StepInstanceId) {
    let run_id = RunId::new_v4();
    let step_id = StepInstanceId::new_v4();
    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, 'step-transitions-test', 'running', 1, '', '', '')")
        .bind(run_id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, status, step_type, spec) VALUES ($1, $2, 'build', $3::step_status, 'RunContainer', '{}')")
        .bind(step_id)
        .bind(run_id)
        .bind(status)
        .execute(pool)
        .await
        .unwrap();
    (run_id, step_id)
}

async fn set_status(
    pool: &sqlx::PgPool,
    step_id: StepInstanceId,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE step_instances SET status = $2::step_status WHERE id = $1")
        .bind(step_id)
        .bind(status)
        .execute(pool)
        .await
        .map(|_| ())
}

async fn history(pool: &sqlx::PgPool, step_id: StepInstanceId) -> Vec<StepStatus> {
    sqlx::query_scalar(
        "SELECT status FROM step_status_history WHERE step_instance_id = $1 ORDER BY id",
    )
    .bind(step_id)
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn every_change_is_recorded_once() {
    let pool = setup_db().await;
    let (run_id, step_id) = insert_run_and_step(&pool, "pending").await;

    set_status(&pool, step_id, "initializing").await.unwrap();
    // Written again unchanged: not a change
    set_status(&pool, step_id, "initializing").await.unwrap();
    set_status(&pool, step_id, "running").await.unwrap();
    set_status(&pool, step_id, "succeeded").await.unwrap();

    assert_eq!(
        history(&pool, step_id).await,
        [
            StepStatus::Pending,
            StepStatus::Initializing,
            StepStatus::Running,
            StepStatus::Succeeded
        ]
    );
    delete_run(&pool, run_id).await;
}

#[tokio::test]
async fn a_finished_step_is_never_reopened() {
    let pool = setup_db().await;
    for finished in [
        "succeeded",
        "failed_ignored",
        "skipped",
        "aborted",
        "lost_zombie",
    ] {
        let (run_id, step_id) = insert_run_and_step(&pool, finished).await;
        for next in ["pending", "running", "failed", "succeeded"] {
            if next == finished {
                continue;
            }
            let err = set_status(&pool, step_id, next)
                .await
                .expect_err(&format!("{finished} -> {next} is refused"));
            assert!(err.to_string().contains("cannot move"), "{err}");
        }
        delete_run(&pool, run_id).await;
    }

    // A failure can still be ignored after the fact
    let (run_id, step_id) = insert_run_and_step(&pool, "failed").await;
    set_status(&pool, step_id, "failed_ignored").await.unwrap();
    assert!(set_status(&pool, step_id, "pending").await.is_err());
    delete_run(&pool, run_id).await;
}

#[tokio::test]
async fn only_a_pending_step_is_claimed() {
    let pool = setup_db().await;
    let (run_id, step_id) = insert_run_and_step(&pool, "running").await;
    assert!(set_status(&pool, step_id, "initializing").await.is_err());
    // Nor does a step that never ran pack its storage, or lose a runner it never had
    let (other_run, pending) = insert_run_and_step(&pool, "pending").await;
    assert!(set_status(&pool, pending, "packing_sfs").await.is_err());
    assert!(set_status(&pool, pending, "lost_zombie").await.is_err());
    delete_run(&pool, run_id).await;
    delete_run(&pool, other_run).await;
}
