//! A run's `max_concurrency` holds however many claims race for its steps.
//!
//! Every step that finishes dispatches its run again, so claims for one run arrive in
//! bursts. Each used to count only running steps and claim the run's whole allowance:
//! a load test limited to 50 had 469 steps in flight.

mod common;

use common::{delete_run, setup_db};
use stormchaser_engine::step_machine::bulk;
use stormchaser_model::{RunId, StepInstanceId};

#[tokio::test]
async fn racing_claims_keep_a_run_within_its_concurrency() {
    let pool = setup_db().await;
    let run_id = RunId::new_v4();
    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, 'concurrency-test', 'running', 1, '', '', '')")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
    for i in 0..30 {
        sqlx::query("INSERT INTO step_instances (id, run_id, step_name, status, step_type, spec, iteration_index) VALUES ($1, $2, 'work', 'pending', 'RunContainer', '{}', $3)")
            .bind(StepInstanceId::new_v4())
            .bind(run_id)
            .bind(i)
            .execute(&pool)
            .await
            .unwrap();
    }

    let claims = (0..10).map(|_| {
        let pool = pool.clone();
        tokio::spawn(async move { bulk::claim_for_run(&pool, run_id, 5).await.unwrap().len() })
    });
    let claimed: usize = futures::future::join_all(claims)
        .await
        .into_iter()
        .map(|c| c.unwrap())
        .sum();
    assert_eq!(claimed, 5, "the claims together took {claimed} steps");

    let in_flight: i64 = stormchaser_engine::db::count_in_flight_steps_for_run(&pool, run_id)
        .await
        .unwrap();
    assert_eq!(in_flight, 5);

    // A step that finishes makes room for exactly one more
    sqlx::query("UPDATE step_instances SET status = 'running' WHERE id = (SELECT id FROM step_instances WHERE run_id = $1 AND status = 'initializing' LIMIT 1)")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE step_instances SET status = 'succeeded' WHERE run_id = $1 AND status = 'running'",
    )
    .bind(run_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        bulk::claim_for_run(&pool, run_id, 5).await.unwrap().len(),
        1
    );
    assert!(bulk::claim_for_run(&pool, run_id, 5)
        .await
        .unwrap()
        .is_empty());

    delete_run(&pool, run_id).await;
}
