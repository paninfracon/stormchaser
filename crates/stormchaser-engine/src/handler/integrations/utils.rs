use anyhow::Result;
use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use std::collections::HashMap;
use stormchaser_model::events::{StepCompletedEvent, StepFailedEvent};
use stormchaser_model::{Connection, RunId, StepInstanceId};

/// Fetches the URL from a configured HttpApi connection by name.
pub async fn fetch_http_api_url_from_connection(
    pool: &PgPool,
    run_id: RunId,
    connection_name: &str,
) -> Result<String> {
    let mut conn = pool.acquire().await?;
    if let Some(connection) = crate::db::connections::get_storage_backend_by_name::<
        &mut sqlx::PgConnection,
        Connection,
    >(&mut *conn, connection_name)
    .await?
    {
        drop(conn);
        crate::authz::ensure_run_may_use(pool, run_id, &connection).await?;
        if connection.connection_type == stormchaser_model::ConnectionType::HttpApi {
            if let Some(url) = connection
                .config
                .get("base_url")
                .and_then(|u| u.as_str())
                // Keep legacy `url` support for existing HttpApi connections.
                .or_else(|| connection.config.get("url").and_then(|u| u.as_str()))
            {
                Ok(url.to_string())
            } else {
                anyhow::bail!(
                    "Connection {} is missing 'base_url' (or legacy 'url') in its config",
                    connection_name
                )
            }
        } else {
            anyhow::bail!("Connection {} must be of type HttpApi", connection_name)
        }
    } else {
        anyhow::bail!("Connection {} not found", connection_name)
    }
}

/// Reports that a step the engine runs itself has completed. The result goes through
/// the outbox to the step-completed handler, as a runner's would, carrying the step's
/// completion token. Tried again while the database is unreachable: no event would
/// bring the result back, and the step would never end.
pub async fn report_step_completed(
    pool: &PgPool,
    run_id: RunId,
    step_id: StepInstanceId,
    fencing_token: i64,
    outputs: Option<HashMap<String, Value>>,
) -> Result<()> {
    crate::retry::persist("record a built-in step's result", || async {
        let mut conn = pool.acquire().await?;
        queue_step_completed(&mut conn, run_id, step_id, fencing_token, outputs.clone()).await
    })
    .await
}

/// Reports a built-in step's completion together with the one output it produced, so
/// that the step cannot be left with one and not the other. Tried again as
/// [`report_step_completed`] is.
pub async fn report_step_output_and_completion(
    pool: &PgPool,
    run_id: RunId,
    step_id: StepInstanceId,
    fencing_token: i64,
    key: &str,
    value: &Value,
) -> Result<()> {
    crate::retry::persist("record a built-in step's output and result", || async {
        let mut tx = pool.begin().await?;
        crate::db::upsert_step_output(&mut *tx, step_id, key, value).await?;
        let outputs = HashMap::from([(key.to_string(), value.clone())]);
        queue_step_completed(&mut tx, run_id, step_id, fencing_token, Some(outputs)).await?;
        tx.commit().await?;
        Ok(())
    })
    .await
}

/// Reports that a step the engine runs itself has failed, as [`report_step_completed`]
/// does its completion. The failure goes to the step-failed handler, as a runner's
/// would, so that the run's failure handling follows; marking the step failed here
/// instead would leave the handler nothing to do.
pub async fn report_step_failed(
    pool: &PgPool,
    run_id: RunId,
    step_id: StepInstanceId,
    fencing_token: i64,
    error: impl Into<String>,
) -> Result<()> {
    let error = error.into();
    crate::retry::persist("record a built-in step's failure", || async {
        let mut conn = pool.acquire().await?;
        queue_step_failed(&mut conn, run_id, step_id, fencing_token, error.clone()).await
    })
    .await
}

/// Queues a built-in step's completion on `conn`, without trying again. On a
/// transaction, the result commits with what the step wrote.
pub async fn queue_step_completed(
    conn: &mut PgConnection,
    run_id: RunId,
    step_id: StepInstanceId,
    fencing_token: i64,
    outputs: Option<HashMap<String, Value>>,
) -> Result<()> {
    let event = StepCompletedEvent {
        completion_token: crate::db::read_step_completion_token(&mut *conn, step_id).await?,
        outputs,
        exit_code: Some(0),
        ..StepCompletedEvent::new(run_id, step_id, fencing_token)
    };
    crate::db::outbox::insert_step_completed(conn, &event).await
}

/// Queues a built-in step's failure on `conn`, without trying again.
pub async fn queue_step_failed(
    conn: &mut PgConnection,
    run_id: RunId,
    step_id: StepInstanceId,
    fencing_token: i64,
    error: impl Into<String>,
) -> Result<()> {
    let event = StepFailedEvent {
        completion_token: crate::db::read_step_completion_token(&mut *conn, step_id).await?,
        ..StepFailedEvent::new(run_id, step_id, fencing_token, error)
    };
    crate::db::outbox::insert_step_failed(conn, &event).await
}

/// Moves a built-in step the engine has claimed from initializing to running, before
/// the engine runs it. `label` names the built-in in the step's `runner_id`.
pub async fn start_built_in(pool: &PgPool, step_id: StepInstanceId, label: &str) -> Result<()> {
    use crate::step_machine::{state, StepMachine};
    let mut conn = pool.acquire().await?;
    let instance = crate::handler::fetch_step_instance(step_id, &mut *conn).await?;
    StepMachine::<state::Initializing>::load(instance)?
        .start(Some(label.to_string()), &mut conn)
        .await?;
    Ok(())
}
