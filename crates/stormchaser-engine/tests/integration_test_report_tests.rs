use sqlx::PgPool;
use std::env::var;
use stormchaser_model::test_report::{TestCase, TestCaseStatus, TestSummary};
use stormchaser_model::TestReportId;
use uuid::Uuid;

use stormchaser_engine::db;

#[tokio::test]
async fn test_report_persistence_integration() {
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
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
    db::insert_step_test_report(
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
    db::insert_step_test_summary(&pool, run_id, step_id, "api-tests", &summary)
        .await
        .unwrap();

    // 4. Test Case Insertion
    let test_case = TestCase {
        id: TestReportId::new_v4(),
        run_id: stormchaser_model::RunId::new(run_id),
        step_instance_id: stormchaser_model::StepInstanceId::new(step_id),
        report_name: "api-tests".to_string(),
        test_suite: Some("suite1".to_string()),
        test_case: "test1".to_string(),
        status: TestCaseStatus::Passed,
        duration_ms: Some(100),
        message: None,
        created_at: chrono::Utc::now(),
    };
    db::insert_step_test_case(&pool, run_id, step_id, "api-tests", &test_case)
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

/// Inserts a running run whose one step declares a junit report named `unit`.
async fn insert_step_declaring_a_report(pool: &PgPool) -> (stormchaser_model::RunId, StepId) {
    let run_id = stormchaser_model::RunId::new_v4();
    let step_id = StepId::new_v4();

    let dsl = r#"
        stormchaser_dsl_version = "v1"
        workflow "report-test" {
            steps {
                step "test" "RunContainer" {
                    image = "alpine"
                    reports {
                        report "unit" {
                            path = "out/*.xml"
                            format = "junit"
                        }
                    }
                }
            }
        }
    "#;
    let workflow = stormchaser_dsl::StormchaserParser::new()
        .parse(dsl)
        .expect("test workflow should parse");

    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, 'report-test', 'test-user', 'running', 1, 'http://git.local', 'workflow.storm', 'main')")
        .bind(run_id)
        .execute(pool)
        .await
        .unwrap();
    db::runs::insert_run_context(
        pool,
        run_id,
        "v1",
        serde_json::to_value(&workflow).unwrap(),
        Some(dsl),
        serde_json::json!({}),
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, status, step_type) VALUES ($1, $2, 'test', 'running', 'RunContainer')")
        .bind(step_id)
        .bind(run_id)
        .execute(pool)
        .await
        .unwrap();

    (run_id, step_id)
}

type StepId = stormchaser_model::StepInstanceId;
/// report_name, file_name, format, checksum, connection_id, remote_path
type StoredReport = (String, String, String, String, Option<Uuid>, Option<String>);

#[tokio::test]
async fn a_step_cannot_choose_what_the_engine_stores_as_its_report() {
    use sha2::{Digest, Sha256};
    use stormchaser_engine::handler::step::events::helpers::{
        declared_test_reports, persist_step_test_reports,
    };

    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPool::connect(&db_url).await.unwrap();
    let (run_id, step_id) = insert_step_declaring_a_report(&pool).await;

    let junit = r#"<testsuite name="s" tests="1"><testcase name="t" classname="c"/></testsuite>"#;
    let reported: std::collections::HashMap<String, serde_json::Value> =
        serde_json::from_value(serde_json::json!({
            // Declared, sent inline: kept, with the declared format and the engine's checksum
            "unit_r.xml": {
                "name": "unit", "file_name": "r.xml", "format": "html",
                "content": junit, "hash": "forged"
            },
            // Not declared, and pointing the engine at someone else's object
            "stolen": {
                "name": "stolen", "is_claim": true, "format": "junit",
                "connection_id": Uuid::new_v4(), "remote_path": "artifacts/another-run/secret"
            },
            // Not declared, inline
            "extra": { "name": "extra", "format": "junit", "content": junit },
            // Declared, but claiming an upload that lives somewhere else: the engine looks
            // only under this step's own key, finds nothing, and stores nothing
            "unit": {
                "name": "unit", "is_claim": true,
                "connection_id": Uuid::new_v4(), "remote_path": "artifacts/another-run/secret"
            }
        }))
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    let declared = declared_test_reports(&mut tx, run_id, "test").await;
    assert_eq!(declared.len(), 1);
    persist_step_test_reports(Some(&reported), &declared, &mut tx, run_id, step_id, &pool)
        .await
        .expect("forged entries are skipped, not errors");
    tx.commit().await.unwrap();

    let rows: Vec<StoredReport> = sqlx::query_as(
        "SELECT report_name, file_name, format, checksum, connection_id, remote_path FROM step_test_reports WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(
        rows.len(),
        1,
        "only the declared inline report is stored: {rows:?}"
    );
    let (name, file_name, format, checksum, connection_id, remote_path) = &rows[0];
    assert_eq!(name, "unit");
    assert_eq!(file_name, "r.xml");
    assert_eq!(format, "junit");
    assert_eq!(checksum, &hex::encode(Sha256::digest(junit.as_bytes())));
    assert_eq!((connection_id, remote_path), (&None, &None));

    let case_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM step_test_cases WHERE run_id = $1")
            .bind(run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(case_count, 1);
}
