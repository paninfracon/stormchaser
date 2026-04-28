use sqlx::PgPool;
use stormchaser_model::test_report::{TestCase, TestCaseStatus, TestSummary};
use uuid::Uuid;

#[tokio::test]
async fn test_report_persistence_integration() {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://stormchaser:stormchaser@localhost:5432/stormchaser".to_string()
    });
    let pool = PgPool::connect(&db_url).await.unwrap();

    let run_id = Uuid::new_v4();
    let step_id = Uuid::new_v4();

    // 1. Insert Run and Step Instance first to satisfy FK constraints
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

    // 2. Test Report Insertion
    stormchaser_engine::db::insert_step_test_report(
        &pool,
        run_id,
        step_id,
        "api-tests",
        "results.xml",
        "junit",
        Some("<testsuite/>"),
        "hash123",
        None,
        None,
    )
    .await
    .unwrap();

    // 3. Test Summary Insertion
    let summary = TestSummary {
        total_tests: 10,
        passed: 9,
        failed: 1,
        ..Default::default()
    };
    stormchaser_engine::db::insert_step_test_summary(&pool, run_id, step_id, "api-tests", &summary)
        .await
        .unwrap();

    // 4. Test Case Insertion
    let test_case = TestCase {
        id: Uuid::new_v4(),
        run_id,
        step_instance_id: step_id,
        report_name: "api-tests".to_string(),
        test_suite: Some("suite1".to_string()),
        test_case: "test1".to_string(),
        status: TestCaseStatus::Passed,
        duration_ms: Some(100),
        message: None,
        created_at: chrono::Utc::now(),
    };
    stormchaser_engine::db::insert_step_test_case(&pool, run_id, step_id, "api-tests", &test_case)
        .await
        .unwrap();

    // 5. Verify
    let report_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM step_test_reports WHERE run_id = $1")
            .bind(run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(report_count, 1);

    let summary_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM step_test_summaries WHERE run_id = $1")
            .bind(run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(summary_count, 1);

    let case_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM step_test_cases WHERE run_id = $1")
            .bind(run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(case_count, 1);
}
