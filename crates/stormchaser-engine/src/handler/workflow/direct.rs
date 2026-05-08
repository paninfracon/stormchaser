use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_dsl::StormchaserParser;
use stormchaser_model::auth::{EngineOpaContext, OpaClient};
use stormchaser_model::events::WorkflowStartPendingEvent;
use stormchaser_model::workflow::{RunStatus, WorkflowRun};
use stormchaser_model::RunId;
use tracing::{debug, error, info};

#[tracing::instrument(skip(payload, pool, opa_client, nats_client), fields(run_id = tracing::field::Empty))]
/// Handle workflow direct.
pub async fn handle_workflow_direct(
    payload: Value,
    pool: PgPool,
    opa_client: Arc<OpaClient>,
    nats_client: async_nats::Client,
) -> Result<()> {
    let run_id_str = payload["run_id"].as_str().context("Missing run_id")?;
    let run_id = uuid::Uuid::parse_str(run_id_str).map(RunId::new)?;
    tracing::Span::current().record("run_id", tracing::field::display(run_id));
    let workflow_content = payload["dsl"].as_str().context("Missing dsl content")?;
    let initiating_user = payload["initiating_user"]
        .as_str()
        .unwrap_or("system")
        .to_string();
    let inputs = payload["inputs"].clone();

    info!("Handling direct one-off workflow run: {}", run_id);

    // 1. Parse the DSL content directly
    let parser = StormchaserParser::new();
    let parsed_workflow = match parser.parse(workflow_content) {
        Ok(w) => w,
        Err(e) => {
            error!("Direct workflow parsing failed for {}: {}", run_id, e);
            return Err(e);
        }
    };

    // 2. OPA Policy Check
    let opa_context = EngineOpaContext {
        run_id,
        initiating_user: initiating_user.clone(),
        workflow_ast: serde_json::to_value(&parsed_workflow)?,
        inputs: inputs.clone(),
    };

    match opa_client.check_context(opa_context).await {
        Ok(true) => debug!("OPA allowed execution for direct run {}", run_id),
        Ok(false) => {
            let err_msg = "Execution denied by OPA policy".to_string();
            info!("Direct run {}: {}", run_id, err_msg);
            return Err(anyhow::anyhow!(err_msg));
        }
        Err(e) => {
            let err_msg = format!("OPA check failed: {}", e);
            error!("Direct run {}: {}", run_id, err_msg);
            return Err(anyhow::anyhow!(err_msg));
        }
    }

    // 3. Create WorkflowRun and RunContext in DB
    // Direct runs have no repo_url or workflow_path in the traditional sense
    let run = WorkflowRun {
        id: run_id,
        workflow_name: parsed_workflow.name.clone(),
        initiating_user: initiating_user.clone(),
        repo_url: "direct://".to_string(),
        workflow_path: "inline.storm".to_string(),
        git_ref: "HEAD".to_string(),
        status: RunStatus::StartPending,
        version: 1,
        fencing_token: Utc::now().timestamp_nanos_opt().unwrap_or(0),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        started_resolving_at: Some(Utc::now()),
        started_at: None,
        finished_at: None,
        error: None,
    };

    let mut tx = pool.begin().await?;

    crate::db::insert_full_workflow_run(
        &mut *tx,
        &run,
        &parsed_workflow.dsl_version,
        serde_json::to_value(&parsed_workflow)?,
        Some(workflow_content),
        inputs,
        10,
        "1",
        "4Gi",
        "10Gi",
        "1h",
    )
    .await?;

    tx.commit().await?;

    // 4. Emit event for transition to StartPending
    let event = WorkflowStartPendingEvent {
        run_id,
        event_type: "workflow_start_pending".to_string(),
        timestamp: Utc::now(),
    };
    let js = async_nats::jetstream::new(nats_client);
    stormchaser_model::nats::publish_cloudevent(
        &js,
        "stormchaser.v1.run.start_pending",
        "stormchaser.v1.run.start_pending",
        "/stormchaser",
        serde_json::to_value(event).unwrap(),
        Some("1.0"),
        None,
    )
    .await
    .with_context(|| {
        format!(
            "Failed to publish start_pending event for direct run {}",
            run_id
        )
    })?;

    info!(
        "Successfully initialized direct one-off workflow run {}",
        run_id
    );

    Ok(())
}
