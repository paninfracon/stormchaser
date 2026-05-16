use crate::handler::{dispatch_pending_steps, fetch_run, fetch_run_context, schedule_step};
use crate::workflow_machine::{state, WorkflowMachine};
use anyhow::{Context, Result};
use opentelemetry::KeyValue;
use sqlx::PgPool;
use std::collections::HashSet;
use std::sync::Arc;
use stormchaser_dsl::ast;
use stormchaser_dsl::ast::Workflow;
use stormchaser_model::events::WorkflowRunningEvent;
use stormchaser_model::events::{EventSource, EventType, WorkflowEventType};
use stormchaser_model::RunId;
use stormchaser_tls::TlsReloader;
use tracing::{debug, error, info};

#[tracing::instrument(skip(pool, nats_client, tls_reloader), fields(run_id = %run_id))]
/// Handle workflow start pending.
pub async fn handle_workflow_start_pending(
    run_id: RunId,
    pool: PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    info!("Handling start_pending workflow run: {}", run_id);

    let mut tx = pool.begin().await?;

    // Lock the workflow run to serialize state evaluations
    if crate::db::lock_workflow_run(&mut *tx, run_id)
        .await?
        .is_none()
    {
        debug!(
            "Workflow run {} already archived or missing, skipping handler",
            run_id
        );
        return Ok(());
    }

    // 1. Fetch WorkflowRun and RunContext
    let run = fetch_run(run_id, &mut *tx).await?;
    let context = fetch_run_context(run_id, &mut *tx).await?;

    // 2. Parse AST from context
    let workflow: Workflow = serde_json::from_value(context.workflow_definition)
        .context("Failed to parse workflow definition from DB")?;

    // 3. Identify starting steps (steps that are not in anyone's 'next' list)
    let all_next_steps: HashSet<String> = workflow
        .steps
        .iter()
        .flat_map(|s| s.next.iter().cloned())
        .collect();

    let initial_steps: Vec<&ast::Step> = workflow
        .steps
        .iter()
        .filter(|s| !all_next_steps.contains(&s.name))
        .collect();

    if initial_steps.is_empty() && !workflow.steps.is_empty() {
        error!("Workflow {} for run {} has steps but no initial steps found (cycle or misconfiguration)", workflow.name, run_id);
        return Err(anyhow::anyhow!("No initial steps found in workflow"));
    }

    // 4. Create HCL Context for resolving expressions
    let hcl_ctx = crate::hcl_eval::create_context(
        context.inputs.clone(),
        run_id,
        context.secrets.clone(),
        serde_json::json!({}),
        Some(&workflow),
        None,
    );

    // 5. Create StepInstances for initial steps and schedule them
    for step_dsl in initial_steps {
        #[allow(clippy::explicit_auto_deref)]
        schedule_step(
            run_id,
            step_dsl,
            &mut *tx,
            nats_client.clone(),
            &hcl_ctx,
            pool.clone(),
            &workflow,
        )
        .await?;
    }

    // 5. Transition Workflow to Running
    let machine = WorkflowMachine::<state::StartPending>::new_from_run(run.clone());
    let _ = machine.start(&mut *tx).await?;

    tx.commit().await?;

    let js = async_nats::jetstream::new(nats_client.clone());
    use stormchaser_model::nats::NatsSubject;
    if let Err(e) = stormchaser_model::nats::publish_cloudevent(
        &js,
        NatsSubject::RunRunning,
        EventType::Workflow(WorkflowEventType::Running),
        EventSource::Engine,
        serde_json::to_value(WorkflowRunningEvent {
            run_id,
            event_type: EventType::Workflow(WorkflowEventType::Running),
            timestamp: chrono::Utc::now(),
        })
        .unwrap(),
        None,
        None,
    )
    .await
    {
        error!(
            "Failed to publish workflow running event for {}: {:?}",
            run_id, e
        );
    }

    crate::RUNS_STARTED.add(
        1,
        &[
            KeyValue::new("workflow_name", run.workflow_name),
            KeyValue::new("initiating_user", run.initiating_user),
        ],
    );

    if let Err(e) = dispatch_pending_steps(run_id, pool, nats_client, tls_reloader).await {
        error!(
            "Failed to dispatch pending steps for run {}: {:?}",
            run_id, e
        );
    }

    info!("Transitioned run {} to Running", run_id);

    Ok(())
}
