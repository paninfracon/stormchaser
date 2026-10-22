/// Deletes archived runs and everything archived with them, returning how many runs
/// were deleted.
///
/// Some of a run's archived rows do not cascade from it, so deleting the run alone
/// would leave them behind for good: nothing finds them once their run is gone.
pub async fn delete_archived_runs(
    conn: &mut sqlx::PgConnection,
    runs: &[uuid::Uuid],
) -> Result<u64, sqlx::Error> {
    for statement in [
        "DELETE FROM archived_step_status_history WHERE step_instance_id IN (SELECT id FROM archived_step_instances WHERE run_id = ANY($1))",
        "DELETE FROM archived_step_test_cases WHERE run_id = ANY($1)",
        "DELETE FROM archived_step_test_summaries WHERE run_id = ANY($1)",
    ] {
        sqlx::query(statement).bind(runs).execute(&mut *conn).await?;
    }
    let deleted = sqlx::query("DELETE FROM archived_workflow_runs WHERE id = ANY($1)")
        .bind(runs)
        .execute(&mut *conn)
        .await?;
    Ok(deleted.rows_affected())
}
