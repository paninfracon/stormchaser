use sqlx::{Connection, Executor, Postgres};
use stormchaser_model::{RunId, StepInstanceId};

#[allow(clippy::too_many_arguments)]
/// Get run quota by id.
pub async fn get_run_quota_by_id<'a, E, O>(executor: E, run_id: RunId) -> Result<O, sqlx::Error>
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

/// Claims a step's share of its run's quota, if the run has room for it, and records
/// on the step that it holds it.
#[allow(clippy::too_many_arguments)]
pub async fn claim_step_quota(
    executor: &mut sqlx::PgConnection,
    run_id: RunId,
    step_id: StepInstanceId,
    cpu_req: f64,
    mem_req: i64,
    max_cpu: f64,
    max_mem: i64,
) -> Result<bool, sqlx::Error> {
    use sqlx::Row;

    let mut tx = executor.begin().await?;
    let row = sqlx::query(
        "SELECT current_cpu_usage, current_memory_usage FROM run_quotas WHERE run_id = $1 FOR UPDATE",
    )
    .bind(run_id)
    .fetch_one(&mut *tx)
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
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE step_instances SET quota_held = true WHERE id = $1")
            .bind(step_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Gives back a step's share of its run's quota, if the step holds it. A step that
/// never claimed it, or has given it back already, gives back nothing.
pub async fn release_step_quota(
    executor: &mut sqlx::PgConnection,
    run_id: RunId,
    step_id: StepInstanceId,
    cpu_req: f64,
    mem_req: i64,
) -> Result<(), sqlx::Error> {
    use sqlx::Row;

    let mut tx = executor.begin().await?;
    let held = sqlx::query(
        "UPDATE step_instances SET quota_held = false WHERE id = $1 AND quota_held RETURNING id",
    )
    .bind(step_id)
    .fetch_optional(&mut *tx)
    .await?
    .is_some();
    if !held {
        return Ok(());
    }

    let row =
        sqlx::query("SELECT current_memory_usage FROM run_quotas WHERE run_id = $1 FOR UPDATE")
            .bind(run_id)
            .fetch_optional(&mut *tx)
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
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}
