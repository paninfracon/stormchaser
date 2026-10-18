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
/// completion token. On a transaction, the result commits with what the step wrote.
pub async fn report_step_completed(
    conn: &mut PgConnection,
    run_id: RunId,
    step_id: StepInstanceId,
    fencing_token: i64,
    outputs: Option<HashMap<String, Value>>,
) -> Result<()> {
    let event = StepCompletedEvent {
        completion_token: crate::db::step_completion_token(&mut *conn, step_id).await,
        outputs,
        exit_code: Some(0),
        ..StepCompletedEvent::new(run_id, step_id, fencing_token)
    };
    crate::db::outbox::insert_step_completed(conn, &event).await
}

/// Reports that a step the engine runs itself has failed. The failure goes through the
/// outbox to the step-failed handler, as a runner's would, so that the run's failure
/// handling follows; marking the step failed here instead would leave the handler
/// nothing to do.
pub async fn report_step_failed(
    conn: &mut PgConnection,
    run_id: RunId,
    step_id: StepInstanceId,
    fencing_token: i64,
    error: impl Into<String>,
) -> Result<()> {
    let event = StepFailedEvent {
        completion_token: crate::db::step_completion_token(&mut *conn, step_id).await,
        ..StepFailedEvent::new(run_id, step_id, fencing_token, error)
    };
    crate::db::outbox::insert_step_failed(conn, &event).await
}
