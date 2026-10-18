use crate::handler::integrations::utils::report_step_completed;
use crate::handler::{fetch_outputs, fetch_run_context, fetch_step_instance};
use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::Connection;
use sqlx::PgPool;
use std::collections::HashMap;
use std::time::Duration;
use stormchaser_model::dsl::SqlExecuteSpec;
use stormchaser_model::{RunId, StepInstanceId};
use tracing::info;

/// Runs a SqlExecute step, reporting its result.
pub(crate) async fn handle_sql_execute(
    run_id: RunId,
    step_id: StepInstanceId,
    fencing_token: i64,
    spec: Value,
    pool: PgPool,
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
    crate::authz::ensure_run_may_use(&pool, run_id, &conn).await?;

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

    let query = crate::templating::render_str(&spec.query, &template_ctx)
        .map_err(|e| anyhow::anyhow!("Failed to render SQL query template: {:?}", e))?;

    let rows_affected = match conn.connection_type {
        stormchaser_model::connections::ConnectionType::Postgres => {
            execute_sql_query(
                &conn.connection_type,
                url,
                &query,
                spec.params.as_deref().unwrap_or_default(),
            )
            .await?
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

    let outputs = HashMap::from([("result".to_string(), output)]);
    report_step_completed(
        &mut *pool.acquire().await?,
        run_id,
        step_id,
        fencing_token,
        Some(outputs),
    )
    .await
}

async fn execute_sql_query(
    connection_type: &stormchaser_model::connections::ConnectionType,
    url: &str,
    query: &str,
    params: &[Value],
) -> Result<u64> {
    match connection_type {
        stormchaser_model::connections::ConnectionType::Postgres => {
            let mut retries = 5;
            let mut conn = loop {
                match tokio::time::timeout(
                    Duration::from_secs(10),
                    sqlx::PgConnection::connect(url),
                )
                .await
                {
                    Ok(Ok(c)) => break c,
                    Ok(Err(e)) => {
                        if retries == 0 {
                            return Err(e.into());
                        }
                        tracing::warn!("Transient SQL connection error: {}. Retrying...", e);
                        retries -= 1;
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                    Err(_) => {
                        if retries == 0 {
                            anyhow::bail!("Connection attempt timed out");
                        }
                        tracing::warn!("SQL connection attempt timed out. Retrying...");
                        retries -= 1;
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
            };

            // The statement is the workflow author's own SqlExecute query, executed as
            // written. Values reach it only as bound parameters.
            let mut statement = sqlx::query(sqlx::AssertSqlSafe(query));
            for param in params {
                statement = match param {
                    Value::Null => statement.bind(None::<String>),
                    Value::Bool(b) => statement.bind(*b),
                    Value::Number(n) => match n.as_i64() {
                        Some(i) => statement.bind(i),
                        None => statement.bind(n.as_f64()),
                    },
                    Value::String(s) => statement.bind(s.clone()),
                    other => statement.bind(sqlx::types::Json(other.clone())),
                };
            }
            let result = statement.execute(&mut conn).await?;
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
    use serde_json::{json, Value};
    use stormchaser_model::connections::ConnectionType;

    fn database_url_from_env() -> String {
        if let Ok(url) = std::env::var("DATABASE_URL") {
            return url;
        }
        dotenvy::dotenv().ok();
        let password = std::env::var("STORMCHASER_DEV_PASSWORD")
            .expect("STORMCHASER_DEV_PASSWORD must be set when DATABASE_URL is unset");
        format!(
            "postgres://stormchaser:{}@127.0.0.1:5432/stormchaser",
            password
        )
    }

    #[tokio::test]
    async fn execute_sql_query_supports_postgres_success() {
        let database_url = database_url_from_env();
        let result = execute_sql_query(&ConnectionType::Postgres, &database_url, "SELECT 1", &[])
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
            &[],
        )
        .await
        .expect_err("unsupported connection types should fail");
        assert!(
            err.to_string().contains("not supported"),
            "expected unsupported connection error, got: {err}"
        );
    }

    #[tokio::test]
    async fn params_are_bound_with_their_types_and_never_read_as_sql() {
        let database_url = database_url_from_env();
        let matched = execute_sql_query(
            &ConnectionType::Postgres,
            &database_url,
            "SELECT 1 WHERE $1 = 'it''s' AND $2 = 42 AND $3 AND $4 ->> 'a' = 'b' \
             AND $5::text IS NULL AND $6 > 1.4",
            &[
                json!("it's"),
                json!(42),
                json!(true),
                json!({"a": "b"}),
                Value::Null,
                json!(1.5),
            ],
        )
        .await
        .unwrap();
        assert_eq!(matched, 1);

        let injected = execute_sql_query(
            &ConnectionType::Postgres,
            &database_url,
            "SELECT 1 WHERE $1 = 'expected'",
            &[json!("x' OR '1'='1")],
        )
        .await
        .unwrap();
        assert_eq!(injected, 0);
    }
}
