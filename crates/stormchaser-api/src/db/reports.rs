use crate::{TestReportSummary, TestSummaryResponse};
use sqlx::PgPool;
use stormchaser_model::RunId;
use stormchaser_model::TestCase;
use stormchaser_model::TestReportId;

/// Retrieves test reports for a specific workflow run.
/// List run test reports.
pub async fn list_run_test_reports(
    pool: &PgPool,
    run_id: RunId,
) -> Result<Vec<TestReportSummary>, sqlx::Error> {
    sqlx::query_as(
        r#"
            WITH combined_reports AS (
                SELECT id, run_id, report_name, file_name, format, checksum, created_at, backend_id, remote_path FROM step_test_reports
                UNION ALL
                SELECT id, run_id, report_name, file_name, format, checksum, created_at, backend_id, remote_path FROM archived_step_test_reports
            )
            SELECT id, report_name, file_name, format, checksum, created_at, backend_id, remote_path FROM combined_reports WHERE run_id = $1 ORDER BY created_at ASC
            "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
}

/// Retrieves test summaries for a specific workflow run.
/// List run test summaries.
pub async fn list_run_test_summaries(
    pool: &PgPool,
    run_id: RunId,
) -> Result<Vec<TestSummaryResponse>, sqlx::Error> {
    sqlx::query_as(
        r#"
            WITH combined_summaries AS (
                SELECT id, run_id, step_instance_id, report_name, total_tests, passed, failed, skipped, errors, duration_ms, created_at FROM step_test_summaries
                UNION ALL
                SELECT id, run_id, step_instance_id, report_name, total_tests, passed, failed, skipped, errors, duration_ms, created_at FROM archived_step_test_summaries
            )
            SELECT id, run_id, step_instance_id, report_name, total_tests, passed, failed, skipped, errors, duration_ms, created_at FROM combined_summaries WHERE run_id = $1 ORDER BY created_at ASC
            "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
}

/// Retrieves individual test cases for a workflow run.
/// List run test cases.
pub async fn list_run_test_cases(
    pool: &PgPool,
    run_id: RunId,
) -> Result<Vec<TestCase>, sqlx::Error> {
    sqlx::query_as(
        r#"
            WITH combined_cases AS (
                SELECT id, run_id, step_instance_id, report_name, test_suite, test_case, status::test_case_status as "status", duration_ms, message, created_at FROM step_test_cases
                UNION ALL
                SELECT id, run_id, step_instance_id, report_name, test_suite, test_case, status::test_case_status as "status", duration_ms, message, created_at FROM archived_step_test_cases
            )
            SELECT * FROM combined_cases WHERE run_id = $1 ORDER BY created_at ASC
            "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
}

/// Retrieves a test report by ID.
/// Get test report.
pub async fn get_test_report(
    pool: &PgPool,
    report_id: TestReportId,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
            WITH combined_reports AS (
                SELECT id, content FROM step_test_reports
                UNION ALL
                SELECT id, content FROM archived_step_test_reports
            )
            SELECT content FROM combined_reports WHERE id = $1
            "#,
    )
    .bind(report_id)
    .fetch_optional(pool)
    .await
}
