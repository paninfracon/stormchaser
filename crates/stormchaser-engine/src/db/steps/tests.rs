use sqlx::{Executor, Postgres};
use stormchaser_model::test_report;
use uuid::Uuid;

/// Get test summaries for run.
pub async fn get_test_summaries_for_run<'a, E>(
    executor: E,
    run_id: Uuid,
) -> Result<Vec<test_report::TestSummary>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query_as(
        r#"
        WITH combined AS (
            SELECT * FROM step_test_summaries
            UNION ALL
            SELECT * FROM archived_step_test_summaries
        )
        SELECT * FROM combined WHERE run_id = $1 ORDER BY created_at ASC
        "#,
    )
    .bind(run_id)
    .fetch_all(executor)
    .await
}

/// Get test cases for report.
pub async fn get_test_cases_for_report<'a, E>(
    executor: E,
    run_id: Uuid,
    report_name: &str,
) -> Result<Vec<test_report::TestCase>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query_as(
        r#"
        WITH combined AS (
            SELECT id, run_id, step_instance_id, report_name, test_suite, test_case, status::text as status, duration_ms, message, created_at FROM step_test_cases
            UNION ALL
            SELECT id, run_id, step_instance_id, report_name, test_suite, test_case, status::text as status, duration_ms, message, created_at FROM archived_step_test_cases
        )
        SELECT id, run_id, step_instance_id, report_name, test_suite, test_case, status::test_case_status as status, duration_ms, message, created_at
        FROM combined WHERE run_id = $1 AND report_name = $2 ORDER BY created_at ASC
        "#,
    )
    .bind(run_id)
    .bind(report_name)
    .fetch_all(executor)
    .await
}
