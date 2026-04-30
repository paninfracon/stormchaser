#![allow(clippy::explicit_auto_deref)]
use super::{
    archive_workflow, dispatch_pending_steps, fetch_inputs, fetch_run, fetch_run_context,
    schedule_step,
};
use crate::git_cache::GitCache;
use crate::workflow_machine::{state, WorkflowMachine};
use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashSet;
use std::fs;
use std::sync::Arc;
use stormchaser_dsl::ast::Workflow;
use stormchaser_dsl::StormchaserParser;
use stormchaser_model::auth::{EngineOpaContext, OpaClient};
use stormchaser_model::workflow::{RunStatus, WorkflowRun};
use stormchaser_tls::TlsReloader;
use tracing::{debug, error, info};
use uuid::Uuid;

use stormchaser_dsl::ast;

#[tracing::instrument(skip(pool, nats_client, _tls_reloader), fields(run_id = %run_id))]
/// Handle workflow timeout.
pub async fn handle_workflow_timeout(
    run_id: Uuid,
    pool: PgPool,
    nats_client: async_nats::Client,
    _tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    info!("Workflow {} timed out, aborting", run_id);

    let run = fetch_run(run_id, &pool).await?;
    if matches!(
        run.status,
        RunStatus::Succeeded | RunStatus::Failed | RunStatus::Aborted
    ) {
        return Ok(());
    }

    // 1. Mark Workflow as Aborted
    let mut machine_run = run.clone();
    machine_run.error = Some("Workflow timed out".to_string());

    match run.status {
        RunStatus::Queued => {
            WorkflowMachine::<state::Queued>::new_from_run(machine_run)
                .abort(&mut *pool.acquire().await?)
                .await?;
        }
        RunStatus::Resolving => {
            WorkflowMachine::<state::Resolving>::new_from_run(machine_run)
                .abort(&mut *pool.acquire().await?)
                .await?;
        }
        RunStatus::StartPending => {
            WorkflowMachine::<state::StartPending>::new_from_run(machine_run)
                .abort(&mut *pool.acquire().await?)
                .await?;
        }
        RunStatus::Running => {
            WorkflowMachine::<state::Running>::new_from_run(machine_run)
                .abort(&mut *pool.acquire().await?)
                .await?;
        }
        _ => {} // Should not happen due to check above
    };

    // 2. Mark all non-terminal steps as failed
    crate::db::fail_pending_steps_for_run_on_timeout(&pool, run_id).await?;

    // 3. Publish abort event
    let event = serde_json::json!({
        "run_id": run_id,
        "event_type": "workflow_aborted",
        "reason": "timeout",
        "timestamp": Utc::now(),
    });
    let js = async_nats::jetstream::new(nats_client);
    js.publish("stormchaser.run.aborted", event.to_string().into())
        .await?;

    // 4. Archive
    archive_workflow(run_id, pool).await?;

    Ok(())
}

#[tracing::instrument(skip(pool, nats_client, tls_reloader), fields(run_id = %run_id))]
/// Handle workflow start pending.
pub async fn handle_workflow_start_pending(
    run_id: Uuid,
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

    // 4. Create CelContext for resolving expressions
    let hcl_ctx =
        crate::hcl_eval::create_context(context.inputs.clone(), run_id, serde_json::json!({}));

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

    crate::RUNS_STARTED.add(
        1,
        &[
            opentelemetry::KeyValue::new("workflow_name", run.workflow_name),
            opentelemetry::KeyValue::new("initiating_user", run.initiating_user),
        ],
    );

    tx.commit().await?;

    if let Err(e) = dispatch_pending_steps(run_id, pool, nats_client, tls_reloader).await {
        error!(
            "Failed to dispatch pending steps for run {}: {:?}",
            run_id, e
        );
    }

    info!("Transitioned run {} to Running", run_id);

    Ok(())
}

#[tracing::instrument(skip(payload, pool, opa_client, nats_client), fields(run_id = tracing::field::Empty))]
/// Handle workflow direct.
pub async fn handle_workflow_direct(
    payload: Value,
    pool: PgPool,
    opa_client: Arc<OpaClient>,
    nats_client: async_nats::Client,
) -> Result<()> {
    let run_id_str = payload["run_id"].as_str().context("Missing run_id")?;
    let run_id = Uuid::parse_str(run_id_str)?;
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
    let event = serde_json::json!({
        "run_id": run_id,
        "event_type": "workflow_start_pending",
        "timestamp": Utc::now(),
    });
    let js = async_nats::jetstream::new(nats_client);
    js.publish("stormchaser.run.start_pending", event.to_string().into())
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

#[tracing::instrument(skip(pool, git_cache, opa_client, nats_client, _tls_reloader), fields(run_id = %run_id))]
/// Handles the event when a workflow is queued and ready for resolution.
pub async fn handle_workflow_queued(
    run_id: Uuid,
    pool: PgPool,
    git_cache: Arc<GitCache>,
    opa_client: Arc<OpaClient>,
    nats_client: async_nats::Client,
    _tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    info!("Handling queued workflow run: {}", run_id);

    // 1. Fetch WorkflowRun from DB
    let run = fetch_run(run_id, &pool).await?;
    let machine = WorkflowMachine::<state::Queued>::new(run);

    // 2. Transition to Resolving
    let machine = machine.start_resolving(&mut *pool.acquire().await?).await?;

    // 3. Resolve the workflow file into cache
    let repo_path_res = git_cache.ensure_files(
        &machine.run.repo_url,
        &machine.run.git_ref,
        std::slice::from_ref(&machine.run.workflow_path),
    );

    let repo_path = match repo_path_res {
        Ok(path) => path,
        Err(e) => {
            let _ = machine
                .fail(
                    format!("Git resolution failed: {}", e),
                    &mut *pool.acquire().await?,
                )
                .await?;
            return Err(e);
        }
    };

    let storm_file_path = repo_path.join(&machine.run.workflow_path);
    if !storm_file_path.exists() {
        let err_msg = format!(
            "Workflow file {} not found in repo",
            machine.run.workflow_path
        );
        let _ = machine
            .fail(err_msg.clone(), &mut *pool.acquire().await?)
            .await?;
        return Err(anyhow::anyhow!(err_msg));
    }

    // 4. Read and Parse the workflow file
    let workflow_content = fs::read_to_string(&storm_file_path)
        .with_context(|| format!("Failed to read workflow file at {:?}", storm_file_path))?;

    debug!(
        "Workflow content loaded for {}: {} bytes",
        run_id,
        workflow_content.len()
    );

    let parser = StormchaserParser::new();
    let mut parsed_workflow = match parser.parse(&workflow_content) {
        Ok(w) => w,
        Err(e) => {
            let _ = machine
                .fail(
                    format!("Workflow parsing failed: {}", e),
                    &mut *pool.acquire().await?,
                )
                .await?;
            return Err(e);
        }
    };

    // 4.5 Resolve includes inline
    let mut resolved_includes = std::collections::HashSet::new();
    let mut includes_to_process = parsed_workflow.includes.clone();
    parsed_workflow.includes.clear(); // We will inline them, so remove from AST

    while let Some(inc) = includes_to_process.pop() {
        if !resolved_includes.insert(inc.workflow.clone()) {
            continue; // Prevent infinite loops
        }

        let inc_path = repo_path.join(&inc.workflow);
        if !inc_path.exists() {
            let err_msg = format!("Included workflow file {} not found", inc.workflow);
            let _ = machine
                .fail(err_msg.clone(), &mut *pool.acquire().await?)
                .await?;
            return Err(anyhow::anyhow!(err_msg));
        }

        let inc_content = fs::read_to_string(&inc_path)?;
        let inc_workflow = match parser.parse(&inc_content) {
            Ok(w) => w,
            Err(e) => {
                let err_msg = format!("Included workflow {} parsing failed: {}", inc.workflow, e);
                let _ = machine
                    .fail(err_msg.clone(), &mut *pool.acquire().await?)
                    .await?;
                return Err(anyhow::anyhow!(err_msg));
            }
        };

        // Inline step libraries
        parsed_workflow
            .step_libraries
            .extend(inc_workflow.step_libraries);

        // Inline steps, applying the include prefix to avoid name collisions and substituting inputs
        let prefix = format!("{}.", inc.name);
        for mut step in inc_workflow.steps {
            step.name = format!("{}{}", prefix, step.name);

            // Update next pointers
            for next_ref in &mut step.next {
                *next_ref = format!("{}{}", prefix, next_ref);
            }

            // Very naive input substitution using HCL templates/variables
            // In a real engine, we'd use hcl_eval, but for AST merging we can inject param overrides
            for (k, v) in &inc.inputs {
                let var_pattern = format!("inputs.{}", k);
                let val_str = v.to_string();

                for param_val in step.params.values_mut() {
                    *param_val = param_val.replace(&var_pattern, &val_str);
                }
            }

            parsed_workflow.steps.push(step);
        }

        includes_to_process.extend(inc_workflow.includes);
    }

    // 5. OPA Policy Check after Parsing
    // We send the full context: AST, user, and inputs
    let inputs = fetch_inputs(run_id, &pool).await?;

    let opa_context = EngineOpaContext {
        run_id,
        initiating_user: machine.run.initiating_user.clone(),
        workflow_ast: serde_json::to_value(&parsed_workflow)?,
        inputs,
    };

    match opa_client.check_context(opa_context).await {
        Ok(true) => debug!("OPA allowed execution for run {}", run_id),
        Ok(false) => {
            let err_msg = "Execution denied by OPA policy".to_string();
            info!("Run {}: {}", run_id, err_msg);
            let _ = machine.fail(err_msg, &mut *pool.acquire().await?).await?;
            return Ok(()); // Handled failure
        }
        Err(e) => {
            let err_msg = format!("OPA check failed: {}", e);
            error!("Run {}: {}", run_id, err_msg);
            let _ = machine.fail(err_msg, &mut *pool.acquire().await?).await?;
            return Err(e);
        }
    }

    // 6. Update RunContext with the definition and source code
    crate::db::update_run_context(
        &pool,
        serde_json::to_value(&parsed_workflow)?,
        Some(&workflow_content).map(|s| s.as_str()),
        &parsed_workflow.dsl_version,
        run_id,
    )
    .await
    .with_context(|| format!("Failed to update run context for {}", run_id))?;

    // 7. Transition to StartPending
    let machine = machine.start_pending(&mut *pool.acquire().await?).await?;

    // Emit event for transition to StartPending
    let event = serde_json::json!({
        "run_id": run_id,
        "event_type": "workflow_start_pending",
        "timestamp": chrono::Utc::now(),
    });
    let js = async_nats::jetstream::new(nats_client);
    js.publish("stormchaser.run.start_pending", event.to_string().into())
        .await
        .with_context(|| format!("Failed to publish start_pending event for {}", run_id))?;

    // 8. Transition to Running
    let _ = machine.start(&mut *pool.acquire().await?).await?;

    info!(
        "Successfully resolved and parsed workflow file, started run {}",
        run_id
    );

    Ok(())
}
