#![allow(clippy::explicit_auto_deref)]
use anyhow::{Context, Result};
use sqlx::PgPool;
use stormchaser_model::runner::RunnerStatus;
use tracing::{debug, error, info};

pub async fn handle_runner_registration(payload: serde_json::Value, pool: PgPool) -> Result<()> {
    let runner_id = payload["runner_id"].as_str().context("Missing runner_id")?;
    let runner_type = payload["runner_type"]
        .as_str()
        .context("Missing runner_type")?;
    let protocol_version = payload["protocol_version"]
        .as_str()
        .context("Missing protocol_version")?;
    let nats_subject = payload["nats_subject"]
        .as_str()
        .context("Missing nats_subject")?;

    let capabilities: Vec<String> = if let Some(caps) = payload["capabilities"].as_array() {
        caps.iter()
            .filter_map(|c| c.as_str().map(|s| s.to_string()))
            .collect()
    } else {
        Vec::new()
    };

    info!("Registering runner: {} (type: {})", runner_id, runner_type);

    let mut step_defs = Vec::new();
    if let Some(step_types) = payload["step_types"].as_array() {
        for st in step_types {
            let step_type = st["step_type"]
                .as_str()
                .context("Missing step_type in step_types array")?;
            let schema = st["schema"].clone();
            let documentation = st["documentation"].as_str();

            step_defs.push(crate::db::StepDefinitionInput {
                step_type: step_type.to_string(),
                schema,
                documentation: documentation.map(|s| s.to_string()),
            });
        }
    }

    let mut conn = pool.acquire().await?;
    crate::db::register_runner_with_steps(
        &mut *conn,
        runner_id,
        runner_type,
        protocol_version,
        &capabilities,
        nats_subject,
        step_defs,
    )
    .await
    .with_context(|| format!("Failed to register runner {} with steps", runner_id))?;

    Ok(())
}

pub async fn handle_wasm_registration(payload: serde_json::Value, pool: PgPool) -> Result<()> {
    let step_type = payload["step_type"].as_str().context("Missing step_type")?;
    let module = payload["wasm_module"]
        .as_str()
        .context("Missing wasm_module")?;
    let function = payload["wasm_function"].as_str().unwrap_or("run");
    let config = payload["wasm_config"].clone();
    let schema = payload["schema"].clone();
    let documentation = payload["documentation"].as_str();

    info!("Registering WASM step type: {}", step_type);

    crate::db::upsert_step_definition_with_wasm(
        &pool,
        step_type,
        &schema,
        documentation,
        module,
        function,
        &config,
    )
    .await
    .with_context(|| format!("Failed to register WASM step type {}", step_type))?;

    Ok(())
}

pub async fn handle_runner_heartbeat(payload: serde_json::Value, pool: PgPool) -> Result<()> {
    let runner_id = payload["runner_id"].as_str().context("Missing runner_id")?;

    debug!("Received heartbeat from runner: {}", runner_id);

    let rows_affected = crate::db::update_runner_heartbeat(&pool, RunnerStatus::Online, runner_id)
        .await
        .with_context(|| format!("Failed to update heartbeat for runner {}", runner_id))?
        .rows_affected();

    if rows_affected == 0 {
        error!("Received heartbeat from unknown runner: {}", runner_id);
    }

    Ok(())
}

pub async fn handle_runner_offline(payload: serde_json::Value, pool: PgPool) -> Result<()> {
    let runner_id = payload["runner_id"].as_str().context("Missing runner_id")?;

    info!("Runner going offline: {}", runner_id);

    crate::db::update_runner_status(&pool, RunnerStatus::Offline, runner_id)
        .await
        .with_context(|| format!("Failed to set runner {} to offline", runner_id))?;

    Ok(())
}
