use sqlx::{Executor, Postgres};
use uuid::Uuid;

#[allow(clippy::too_many_arguments)]
pub async fn get_run_quota_by_id<'a, E, O>(executor: E, run_id: Uuid) -> Result<O, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>,
{
    sqlx::query_as::<_, O>(
        r#"SELECT run_id, max_concurrency, max_cpu, max_memory, max_storage, timeout, current_cpu_usage, current_memory_usage FROM run_quotas WHERE run_id = $1"#
    )
    .bind(run_id)
    .fetch_one(executor)
    .await
}

pub async fn claim_step_quota(
    executor: &mut sqlx::PgConnection,
    run_id: Uuid,
    cpu_req: f64,
    mem_req: i64,
    max_cpu: f64,
    max_mem: i64,
) -> Result<bool, sqlx::Error> {
    use sqlx::Row;

    let row = sqlx::query(
        "SELECT current_cpu_usage, current_memory_usage FROM run_quotas WHERE run_id = $1 FOR UPDATE",
    )
    .bind(run_id)
    .fetch_one(&mut *executor)
    .await?;

    let current_cpu: f64 = row.get("current_cpu_usage");
    let current_mem_str: String = row.try_get("current_memory_usage")?;
    let current_mem: i64 = current_mem_str
        .parse()
        .map_err(|e: std::num::ParseIntError| sqlx::Error::Decode(Box::new(e)))?;

    if current_cpu + cpu_req <= max_cpu && current_mem + mem_req <= max_mem {
        let new_mem = (current_mem + mem_req).to_string();
        sqlx::query(
            "UPDATE run_quotas SET current_cpu_usage = current_cpu_usage + $1, current_memory_usage = $2 WHERE run_id = $3",
        )
        .bind(cpu_req)
        .bind(new_mem)
        .bind(run_id)
        .execute(&mut *executor)
        .await?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub async fn release_step_quota(
    executor: &mut sqlx::PgConnection,
    run_id: Uuid,
    cpu_req: f64,
    mem_req: i64,
) -> Result<(), sqlx::Error> {
    use sqlx::Row;

    let row =
        sqlx::query("SELECT current_memory_usage FROM run_quotas WHERE run_id = $1 FOR UPDATE")
            .bind(run_id)
            .fetch_optional(&mut *executor)
            .await?;

    if let Some(row) = row {
        let current_mem_str: String = row.try_get("current_memory_usage")?;
        let current_mem: i64 = current_mem_str
            .parse()
            .map_err(|e: std::num::ParseIntError| sqlx::Error::Decode(Box::new(e)))?;
        let new_mem = (current_mem - mem_req).max(0).to_string();

        sqlx::query(
            "UPDATE run_quotas SET current_cpu_usage = GREATEST(0, current_cpu_usage - $1), current_memory_usage = $2 WHERE run_id = $3",
        )
        .bind(cpu_req)
        .bind(new_mem)
        .bind(run_id)
        .execute(&mut *executor)
        .await?;
    }

    Ok(())
}
