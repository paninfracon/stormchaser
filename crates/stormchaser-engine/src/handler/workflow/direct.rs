use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_dsl::StormchaserParser;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::events::WorkflowStartPendingEvent;
use stormchaser_model::events::{EventSource, EventType, SchemaVersion, WorkflowEventType};
use stormchaser_model::workflow::{RunStatus, WorkflowRun};
use stormchaser_model::RunId;
use tracing::{debug, info};

use super::resolve::{check_policy, run_queries, validate_inputs};

#[tracing::instrument(skip(payload, pool, opa_client, _nats_client), fields(run_id = tracing::field::Empty))]
/// Handle workflow direct.
pub async fn handle_workflow_direct(
    payload: Value,
    pool: PgPool,
    opa_client: Arc<OpaClient>,
    _nats_client: async_nats::Client,
) -> Result<()> {
    let run_id_str = payload["run_id"].as_str().context("Missing run_id")?;
    let run_id = uuid::Uuid::parse_str(run_id_str).map(RunId::new)?;
    tracing::Span::current().record("run_id", tracing::field::display(run_id));

    // The stream can deliver this event again after the run was created; its queries
    // and checks must not run a second time
    if crate::db::workflow_run_exists(&pool, run_id).await? {
        debug!("Direct run {} already exists, skipping", run_id);
        return Ok(());
    }

    let workflow_content = payload["dsl"].as_str().context("Missing dsl content")?;
    let initiating_user = payload["initiating_user"]
        .as_str()
        .unwrap_or("system")
        .to_string();
    let inputs = payload["inputs"].clone();

    info!("Handling direct one-off workflow run: {}", run_id);

    // Nothing is recorded until the run is created below, so an event that fails
    // transiently, an unreachable OPA server or database, can simply be delivered again
    let parsed_workflow = StormchaserParser::new()
        .parse(workflow_content)
        .context("Direct workflow parsing failed")?;
    check_policy(
        &opa_client,
        run_id,
        &initiating_user,
        &parsed_workflow,
        &inputs,
    )
    .await?;
    let (query_results, mut query_secrets) =
        run_queries(run_id, &initiating_user, &parsed_workflow, &inputs, &pool).await?;
    let inputs_to_save = validate_inputs(run_id, &parsed_workflow, inputs, query_results)?;

    // 2. Create WorkflowRun and RunContext in DB
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

    // The workflow's author chooses these, so the operator's ceilings apply
    let limits = crate::run_limits::RunLimits::from_env();
    let max_concurrency = limits.concurrency(
        parsed_workflow
            .quotas
            .as_ref()
            .and_then(|q| q.max_concurrency)
            .unwrap_or(10),
    ) as i32;
    let max_cpu = limits.cpu(
        &parsed_workflow
            .quotas
            .as_ref()
            .and_then(|q| q.max_cpu.clone())
            .unwrap_or_else(|| "1".to_string()),
    );
    let max_memory = limits.memory(
        &parsed_workflow
            .quotas
            .as_ref()
            .and_then(|q| q.max_memory.clone())
            .unwrap_or_else(|| "4Gi".to_string()),
    );
    let max_storage = parsed_workflow
        .quotas
        .as_ref()
        .and_then(|q| q.max_storage.clone())
        .unwrap_or_else(|| "10Gi".to_string());
    let timeout = limits.timeout(
        &parsed_workflow
            .quotas
            .as_ref()
            .and_then(|q| q.timeout.clone())
            .unwrap_or_else(|| "1h".to_string()),
    );

    crate::db::insert_full_workflow_run(
        &mut *tx,
        &run,
        &parsed_workflow.dsl_version,
        serde_json::to_value(&parsed_workflow)?,
        Some(workflow_content),
        inputs_to_save,
        max_concurrency,
        &max_cpu,
        &max_memory,
        &max_storage,
        &timeout,
    )
    .await?;
    query_secrets.extend(crate::hcl_eval::take_alias_secrets(run_id));
    crate::db::add_sensitive_values(&mut *tx, run_id, &query_secrets).await?;

    // 4. Emit event for transition to StartPending
    let event = WorkflowStartPendingEvent {
        run_id,
        event_type: EventType::Workflow(WorkflowEventType::StartPending),
        timestamp: chrono::Utc::now(),
        status: stormchaser_model::workflow::RunStatus::StartPending,
    };
    use stormchaser_model::nats::NatsSubject;
    crate::db::outbox::insert_outbox_event(
        &mut *tx,
        NatsSubject::RunStartPending(Some(stormchaser_model::nats::compute_shard_id(&run_id))),
        EventType::Workflow(WorkflowEventType::StartPending),
        EventSource::System,
        serde_json::to_value(event)?,
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await
    .with_context(|| {
        format!(
            "Failed to publish start_pending event to outbox for direct run {}",
            run_id
        )
    })?;

    tx.commit().await?;

    info!(
        "Successfully initialized direct one-off workflow run {}",
        run_id
    );

    Ok(())
}
