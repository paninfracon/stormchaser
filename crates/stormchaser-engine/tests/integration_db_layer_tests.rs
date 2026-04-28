use chrono::Utc;
use sqlx::postgres::PgPoolOptions;
use stormchaser_engine::db;
use stormchaser_model::runner::RunnerStatus;
use stormchaser_model::step::{StepInstance, StepStatus};
use stormchaser_model::test_report::{TestCase, TestCaseStatus, TestSummary};
use stormchaser_model::workflow::RunStatus;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://stormchaser:stormchaser@localhost:5432/stormchaser".into());
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap()
}

use stormchaser_model::runner;
use stormchaser_model::test_report;

#[tokio::test]
async fn test_db_layer_functions() {
    let pool = setup_db().await;

    // Create a new runner to test
    let runner_id = format!("test-runner-{}", Uuid::new_v4());
    db::runners::upsert_runner(
        &pool,
        &runner_id,
        "docker",
        RunnerStatus::Online,
        "v1.0",
        &["RunContainer".to_string()],
        "subject",
    )
    .await
    .unwrap();

    let runner: Option<runner::Runner> = db::runners::get_runner(&pool, &runner_id).await.unwrap();
    assert!(runner.is_some());
    assert_eq!(runner.as_ref().unwrap().status, RunnerStatus::Online);

    // Make runner stale
    sqlx::query("UPDATE runners SET last_heartbeat_at = NOW() - INTERVAL '1 minute' WHERE id = $1")
        .bind(&runner_id)
        .execute(&pool)
        .await
        .unwrap();

    db::runners::mark_stale_runners_offline(&pool, RunnerStatus::Offline, RunnerStatus::Online)
        .await
        .unwrap();

    let runner: Option<runner::Runner> = db::runners::get_runner(&pool, &runner_id).await.unwrap();
    assert_eq!(runner.as_ref().unwrap().status, RunnerStatus::Offline);

    // Test creating a workflow run
    let run_id = Uuid::new_v4();
    let mut tx = pool.begin().await.unwrap();

    db::runs::insert_workflow_run(
        &mut *tx,
        run_id,
        "test-workflow",
        Some("user"),
        Some("https://github"),
        Some("path"),
        Some("HEAD"),
        RunStatus::Running,
        Some(1),
        Utc::now(),
        Utc::now(),
        None,
    )
    .await
    .unwrap();
    db::runs::insert_run_context(
        &mut *tx,
        run_id,
        "1.0",
        serde_json::json!({}),
        Some("code"),
        serde_json::json!({}),
    )
    .await
    .unwrap();

    db::runs::update_run_context(
        &mut *tx,
        serde_json::json!({}),
        Some("new code"),
        "2.0",
        run_id,
    )
    .await
    .unwrap();

    // Create a step
    let step_id = Uuid::new_v4();
    db::steps::insert_step_instance_with_spec(
        &mut *tx,
        step_id,
        run_id,
        "step1",
        "RunContainer",
        StepStatus::Pending,
        None,
        serde_json::json!({}),
        serde_json::json!({}),
        Utc::now(),
    )
    .await
    .unwrap();

    tx.commit().await.unwrap();

    let count: i64 = db::steps::count_running_steps_for_run(&pool, run_id)
        .await
        .unwrap();
    assert_eq!(count, 0);

    let mut tx = pool.begin().await.unwrap();
    db::steps::update_step_instance_running(
        &mut *tx,
        &StepStatus::Running,
        Some(&runner_id),
        step_id,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let count: i64 = db::steps::count_running_steps_for_run(&pool, run_id)
        .await
        .unwrap();
    assert_eq!(count, 1);

    db::steps::fail_step_instance_with_error(
        &pool,
        StepStatus::Failed,
        "failed msg",
        Some(1),
        step_id,
    )
    .await
    .unwrap();

    let steps: Vec<StepInstance> = db::steps::get_step_instances_by_run_id(&pool, run_id)
        .await
        .unwrap();
    assert_eq!(steps[0].status, StepStatus::Failed);
    assert_eq!(steps[0].error, Some("failed msg".to_string()));
    assert_eq!(steps[0].exit_code, Some(1));

    // Event correlation
    let corr_id = Uuid::new_v4();
    db::events::insert_event_correlation(&pool, corr_id, step_id, run_id, "test-key", "test-val")
        .await
        .unwrap();

    // Storage
    db::storage::insert_step_test_report(
        &pool,
        run_id,
        step_id,
        "report",
        "report.xml",
        "junit",
        Some("content"),
        "checksum",
        None,
        None,
    )
    .await
    .unwrap();

    let summary = TestSummary {
        id: Uuid::new_v4(),
        run_id,
        step_instance_id: step_id,
        report_name: "report".to_string(),
        total_tests: 10,
        passed: 1,
        failed: 0,
        skipped: 0,
        errors: 0,
        duration_ms: 500,
        created_at: Utc::now(),
    };
    db::storage::insert_step_test_summary(&pool, run_id, step_id, "report", &summary)
        .await
        .unwrap();

    let tc = TestCase {
        id: Uuid::new_v4(),
        run_id,
        step_instance_id: step_id,
        report_name: "report".to_string(),
        test_suite: Some("class".to_string()),
        test_case: "name".to_string(),
        status: TestCaseStatus::Passed,
        duration_ms: Some(100),
        message: None,
        created_at: Utc::now(),
    };
    db::storage::insert_step_test_case(&pool, run_id, step_id, "report", &tc)
        .await
        .unwrap();

    let summaries: Vec<test_report::TestSummary> =
        db::steps::get_test_summaries_for_run(&pool, run_id)
            .await
            .unwrap();
    assert_eq!(summaries.len(), 1);

    let cases: Vec<test_report::TestCase> =
        db::steps::get_test_cases_for_report(&pool, run_id, "report")
            .await
            .unwrap();
    assert_eq!(cases.len(), 1);
}
