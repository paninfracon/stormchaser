/// Records the key issued to `runner_id`, replacing any earlier one.
pub async fn store_runner_session<'a, E>(
    executor: E,
    runner_id: &str,
    session_key: &[u8],
) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'a, Database = sqlx::Postgres>,
{
    sqlx::query(
        r#"
        INSERT INTO runner_sessions (runner_id, session_key, issued_at) VALUES ($1, $2, NOW())
        ON CONFLICT (runner_id) DO UPDATE SET session_key = EXCLUDED.session_key, issued_at = NOW()
        "#,
    )
    .bind(runner_id)
    .bind(session_key)
    .execute(executor)
    .await?;
    Ok(())
}
