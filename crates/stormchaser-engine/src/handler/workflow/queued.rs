use crate::git_cache::GitCache;
use crate::handler::{fetch_inputs, fetch_run};
use crate::workflow_machine::{state, WorkflowMachine};
use anyhow::{Context, Result};
use sqlx::PgPool;
use std::fs;
use std::sync::Arc;
use stormchaser_dsl::StormchaserParser;
use stormchaser_model::auth::{EngineOpaContext, OpaClient};
use stormchaser_model::events::WorkflowStartPendingEvent;
use stormchaser_model::RunId;
use stormchaser_tls::TlsReloader;
use tracing::{debug, error, info};

#[tracing::instrument(skip(pool, git_cache, opa_client, nats_client, _tls_reloader), fields(run_id = %run_id))]
/// Handles the event when a workflow is queued and ready for resolution.
pub async fn handle_workflow_queued(
    run_id: RunId,
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
    let event = WorkflowStartPendingEvent {
        run_id,
        event_type: "workflow_start_pending".to_string(),
        timestamp: chrono::Utc::now(),
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
    .with_context(|| format!("Failed to publish start_pending event for {}", run_id))?;

    // 8. Transition to Running
    let _ = machine.start(&mut *pool.acquire().await?).await?;

    info!(
        "Successfully resolved and parsed workflow file, started run {}",
        run_id
    );

    Ok(())
}
