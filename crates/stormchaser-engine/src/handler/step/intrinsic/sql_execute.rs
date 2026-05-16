use crate::handler::{fetch_outputs, fetch_run_context, fetch_step_instance};
use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value;
use sqlx::Connection;
use sqlx::PgPool;
use std::time::Duration;
use stormchaser_model::dsl::SqlExecuteSpec;
use stormchaser_model::events::{
    EventSource, EventType, SchemaVersion, StepCompletedEvent, StepEventType, StepFailedEvent,
};
use stormchaser_model::nats::{publish_cloudevent, NatsSubject};
use stormchaser_model::{RunId, StepInstanceId};
use tracing::{error, info};

pub async fn try_dispatch(
    run_id: RunId,
    step_id: StepInstanceId,
    fencing_token: i64,
    step_type: &str,
    spec: &Value,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<bool> {
    if step_type == "SqlExecute" {
        let spec_clone = spec.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_sql_execute(
                run_id,
                step_id,
                fencing_token,
                spec_clone,
                pool.clone(),
                nats_client.clone(),
            )
            .await
            {
                error!("SqlExecute failed for run {}: {:?}", run_id, e);
                let fail_event = StepFailedEvent {
                    run_id,
                    step_id,
                    fencing_token,
                    event_type: EventType::Step(StepEventType::Failed),
                    error: format!("SqlExecute failed: {:?}", e),
                    runner_id: Some("intrinsic-sql".to_string()),
                    exit_code: Some(1),
                    storage_hashes: None,
                    outputs: None,
                    artifacts: None,
                    test_reports: None,
                    timestamp: Utc::now(),
                };
                let _ = publish_cloudevent(
                    &async_nats::jetstream::new(nats_client),
                    NatsSubject::StepFailed,
                    EventType::Step(StepEventType::Failed),
                    EventSource::System,
                    serde_json::to_value(fail_event).unwrap(),
                    Some(SchemaVersion::new("1.0".to_string())),
                    None,
                )
                .await;
            }
        });
        return Ok(true);
    }

    Ok(false)
}

async fn handle_sql_execute(
    run_id: RunId,
    step_id: StepInstanceId,
    fencing_token: i64,
    spec: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    let spec: SqlExecuteSpec = serde_json::from_value(spec.get("spec").unwrap_or(&spec).clone())?;

    info!(
        "Executing SQL query via connection {} for run {}",
        spec.connection, run_id
    );

    // 1. Mark as Running
    let instance = fetch_step_instance(step_id, &pool).await?;
    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(
            instance,
        );
    let _ = machine
        .start("sql_execute".to_string(), &mut *pool.acquire().await?)
        .await?;

    let conn = crate::db::connections::get_storage_backend_by_name::<
        _,
        stormchaser_model::Connection,
    >(&pool, &spec.connection)
    .await?
    .context(format!("Connection {} not found", spec.connection))?;

    let url = conn
        .config
        .get("url")
        .and_then(|v| v.as_str())
        .context("Connection missing 'url'")?;

    // Render the template in the query if necessary
    let run_context = fetch_run_context(run_id, &pool).await?;
    let outputs = fetch_outputs(run_id, &pool).await?;
    let template_ctx = serde_json::json!({
        "inputs": run_context.inputs,
        "steps": outputs,
        "run": { "id": run_id.to_string() }
    });

    let query = {
        use minijinja::Environment;
        let env = Environment::new();
        env.render_str(&spec.query, &template_ctx)
            .map_err(|e| anyhow::anyhow!("Failed to render SQL query template: {:?}", e))?
    };

    let rows_affected = match conn.connection_type {
        stormchaser_model::connections::ConnectionType::Postgres => {
            execute_sql_query(&conn.connection_type, url, &query).await?
        }
        stormchaser_model::connections::ConnectionType::Mysql => {
            anyhow::bail!("MySQL support is not compiled into the engine");
        }
        _ => {
            anyhow::bail!(
                "Connection type {:?} is not supported for SqlExecute",
                conn.connection_type
            );
        }
    };

    let output = serde_json::json!({
        "rows_affected": rows_affected
    });

    let completed_event = StepCompletedEvent {
        run_id,
        step_id,
        fencing_token,
        event_type: EventType::Step(StepEventType::Completed),
        runner_id: Some("intrinsic-sql".to_string()),
        exit_code: Some(0),
        storage_hashes: None,
        outputs: Some(vec![("result".to_string(), output)].into_iter().collect()),
        artifacts: None,
        test_reports: None,
        timestamp: Utc::now(),
    };

    publish_cloudevent(
        &async_nats::jetstream::new(nats_client),
        NatsSubject::StepCompleted,
        EventType::Step(StepEventType::Completed),
        EventSource::System,
        serde_json::to_value(completed_event).unwrap(),
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await?;

    Ok(())
}

async fn execute_sql_query(
    connection_type: &stormchaser_model::connections::ConnectionType,
    url: &str,
    query: &str,
) -> Result<u64> {
    match connection_type {
        stormchaser_model::connections::ConnectionType::Postgres => {
            let mut conn =
                tokio::time::timeout(Duration::from_secs(30), sqlx::PgConnection::connect(url))
                    .await
                    .context("Connection attempt timed out")??;

            let result = sqlx::query(query).execute(&mut conn).await?;
            let affected = result.rows_affected();
            Ok(affected)
        }
        other => anyhow::bail!(
            "Connection type {:?} is not supported for SqlExecute",
            other
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::execute_sql_query;
    use stormchaser_model::connections::ConnectionType;

    fn database_url_from_env() -> String {
        if let Ok(url) = std::env::var("DATABASE_URL") {
            return url;
        }
        dotenvy::dotenv().ok();
        let password = std::env::var("STORMCHASER_DEV_PASSWORD")
            .expect("STORMCHASER_DEV_PASSWORD must be set when DATABASE_URL is unset");
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            password
        )
    }

    #[tokio::test]
    async fn execute_sql_query_supports_postgres_success() {
        let database_url = database_url_from_env();
        let result = execute_sql_query(&ConnectionType::Postgres, &database_url, "SELECT 1")
            .await
            .expect("postgres query should execute successfully");
        assert_eq!(result, 1);
    }

    #[tokio::test]
    async fn execute_sql_query_rejects_unsupported_connection_type() {
        let err = execute_sql_query(
            &ConnectionType::HttpApi,
            "https://paninfracon.net",
            "SELECT 1",
        )
        .await
        .expect_err("unsupported connection types should fail");
        assert!(
            err.to_string().contains("not supported"),
            "expected unsupported connection error, got: {err}"
        );
    }
}
