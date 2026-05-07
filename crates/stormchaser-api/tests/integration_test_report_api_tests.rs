use sqlx::PgPool;
use stormchaser_model::test_report::{TestCaseStatus, TestSummary};
use stormchaser_model::RunId;
use uuid::Uuid;

use stormchaser_api::db;

#[tokio::test]
async fn test_report_api_integration() {
    dotenvy::dotenv().ok();
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            std::env::var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPool::connect(&db_url).await.unwrap();

    let run_id = Uuid::new_v4();
    let step_id = Uuid::new_v4();

    // 1. Setup data
    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, $2, $3, 'running', 1, 'http://git.local', 'workflow.storm', 'main')")
        .bind(run_id)
        .bind("test-workflow")
        .bind("test-user")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, status, step_type) VALUES ($1, $2, $3, 'running', 'RunContainer')")
        .bind(step_id)
        .bind(run_id)
        .bind("test-step")
        .execute(&pool)
        .await
        .unwrap();

    // 2. Insert summary and case
    let summary = TestSummary {
        total_tests: 10,
        passed: 9,
        failed: 1,
        ..Default::default()
    };

    // We use the raw SQL because we are in API crate and don't want to depend on engine internals if possible,
    // but the API's db module might already have these if I added them.
    // Let's check what's available in stormchaser_api::db

    sqlx::query("INSERT INTO step_test_summaries (run_id, step_instance_id, report_name, total_tests, passed, failed, skipped, errors, duration_ms) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)")
        .bind(run_id)
        .bind(step_id)
        .bind("api-tests")
        .bind(summary.total_tests)
        .bind(summary.passed)
        .bind(summary.failed)
        .bind(0)
        .bind(0)
        .bind(0)
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO step_test_cases (run_id, step_instance_id, report_name, test_case, status) VALUES ($1, $2, $3, $4, 'passed')")
        .bind(run_id)
        .bind(step_id)
        .bind("api-tests")
        .bind("test1")
        .execute(&pool)
        .await
        .unwrap();

    // 3. Test API functions (we test the DB layer in API crate)
    let summaries = db::list_run_test_summaries(&pool, RunId::new(run_id))
        .await
        .unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].report_name, "api-tests");
    assert_eq!(summaries[0].total_tests, 10);

    let cases = db::list_run_test_cases(&pool, RunId::new(run_id))
        .await
        .unwrap();
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].test_case, "test1");
    assert_eq!(cases[0].status, TestCaseStatus::Passed);
}
