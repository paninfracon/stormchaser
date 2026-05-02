#![allow(clippy::explicit_auto_deref)]
use crate::handler::{
    archive_workflow, dispatch_pending_steps, fetch_outputs, fetch_run, fetch_run_context,
    fetch_step_instance,
};
use crate::workflow_machine::{state, WorkflowMachine};
use anyhow::{Context, Result};
use chrono::Utc;
use flate2::read::GzDecoder;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use std::io::Read;
use std::sync::Arc;
use stormchaser_dsl::ast::Workflow;
use stormchaser_model::step::{StepInstance, StepStatus};
use stormchaser_tls::TlsReloader;
use tar::Archive;
use tracing::{debug, error, info};
use uuid::Uuid;

use super::dispatch::{dispatch_step_instance, find_step};
use super::quota::release_step_quota_for_instance;
use super::scheduling::schedule_step;

use stormchaser_dsl::ast;
use stormchaser_model::LogBackend;
use stormchaser_model::StorageBackend;

#[tracing::instrument(skip(payload, pool), fields(run_id = tracing::field::Empty, step_id = tracing::field::Empty))]
/// Handle step unpacking sfs.
pub async fn handle_step_unpacking_sfs(payload: Value, pool: PgPool) -> Result<()> {
    let run_id_str = payload["run_id"].as_str().context("Missing run_id")?;
    let run_id = Uuid::parse_str(run_id_str)?;
    let step_id_str = payload["step_id"].as_str().context("Missing step_id")?;
    let step_id = Uuid::parse_str(step_id_str)?;

    let span = tracing::Span::current();
    span.record("run_id", tracing::field::display(run_id));
    span.record("step_id", tracing::field::display(step_id));
    let runner_id = payload["runner_id"].as_str().unwrap_or("unknown");

    info!(
        "Step {} (Run {}) is now unpacking SFS on runner {}",
        step_id, run_id, runner_id
    );

    let instance = fetch_step_instance(step_id, &pool).await?;
    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(
            instance,
        );
    let _ = machine
        .start_unpacking(runner_id.to_string(), &mut *pool.acquire().await?)
        .await?;

    Ok(())
}

#[tracing::instrument(skip(payload, pool), fields(run_id = tracing::field::Empty, step_id = tracing::field::Empty))]
/// Handle step packing sfs.
pub async fn handle_step_packing_sfs(payload: Value, pool: PgPool) -> Result<()> {
    let run_id_str = payload["run_id"].as_str().context("Missing run_id")?;
    let run_id = Uuid::parse_str(run_id_str)?;
    let step_id_str = payload["step_id"].as_str().context("Missing step_id")?;
    let step_id = Uuid::parse_str(step_id_str)?;

    let span = tracing::Span::current();
    span.record("run_id", tracing::field::display(run_id));
    span.record("step_id", tracing::field::display(step_id));

    info!("Step {} (Run {}) is now packing SFS", step_id, run_id);

    let instance = fetch_step_instance(step_id, &pool).await?;
    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Running>::from_instance(
            instance,
        );
    let _ = machine.start_packing(&mut *pool.acquire().await?).await?;

    Ok(())
}

#[tracing::instrument(skip(payload, pool), fields(run_id = tracing::field::Empty, step_id = tracing::field::Empty))]
/// Handle step running.
pub async fn handle_step_running(payload: Value, pool: PgPool) -> Result<()> {
    let run_id_str = payload["run_id"].as_str().context("Missing run_id")?;
    let run_id = Uuid::parse_str(run_id_str)?;
    let step_id_str = payload["step_id"].as_str().context("Missing step_id")?;
    let step_id = Uuid::parse_str(step_id_str)?;

    let span = tracing::Span::current();
    span.record("run_id", tracing::field::display(run_id));
    span.record("step_id", tracing::field::display(step_id));
    let runner_id = payload["runner_id"].as_str().unwrap_or("unknown");

    info!(
        "Step {} (Run {}) is now running on runner {}",
        step_id, run_id, runner_id
    );

    // 1. Fetch current instance
    let instance = fetch_step_instance(step_id, &pool).await?;

    // 2. Use state machine to transition
    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(
            instance.clone(),
        );
    let _ = machine
        .start(runner_id.to_string(), &mut *pool.acquire().await?)
        .await?;

    crate::STEPS_STARTED.add(
        1,
        &[
            opentelemetry::KeyValue::new("step_name", instance.step_name),
            opentelemetry::KeyValue::new("step_type", instance.step_type),
            opentelemetry::KeyValue::new("runner_id", runner_id.to_string()),
        ],
    );

    Ok(())
}

#[tracing::instrument(skip(payload, pool, nats_client, log_backend, tls_reloader), fields(run_id = tracing::field::Empty, step_id = tracing::field::Empty))]
/// Handle step completed.
pub async fn handle_step_completed(
    payload: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    log_backend: Arc<Option<LogBackend>>,
    tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    let run_id_str = payload["run_id"].as_str().context("Missing run_id")?;
    let run_id = Uuid::parse_str(run_id_str)?;
    let step_id_str = payload["step_id"].as_str().context("Missing step_id")?;
    let step_id = Uuid::parse_str(step_id_str)?;

    let span = tracing::Span::current();
    span.record("run_id", tracing::field::display(run_id));
    span.record("step_id", tracing::field::display(step_id));

    info!("Step {} (Run {}) completed successfully", step_id, run_id);

    let mut tx = pool.begin().await?;

    // Lock the workflow run to serialize state evaluations and prevent parallel join race conditions
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

    // 1. Update StepInstance status
    let instance = fetch_step_instance(step_id, &mut *tx).await?;

    // Ensure we don't process duplicate completion events
    if instance.status == StepStatus::Succeeded || instance.status == StepStatus::Failed {
        return Ok(());
    }

    let _ = release_step_quota_for_instance(&mut *tx, run_id, step_id).await;

    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Running>::from_instance(
            instance.clone(),
        );
    let _ = machine.succeed(&mut *tx).await?;

    let attributes = [
        opentelemetry::KeyValue::new("step_name", instance.step_name),
        opentelemetry::KeyValue::new("step_type", instance.step_type),
        opentelemetry::KeyValue::new("runner_id", instance.runner_id.unwrap_or_default()),
    ];

    crate::STEPS_COMPLETED.add(1, &attributes);

    if let Some(started_at) = instance.started_at {
        let duration = (Utc::now() - started_at).to_std().unwrap_or_default();
        crate::STEP_DURATION.record(duration.as_secs_f64(), &attributes);
    }

    // 2. Fetch ALL steps again to ensure we have the latest status for everyone
    let all_steps: Vec<StepInstance> =
        crate::db::get_step_instances_by_run_id(&mut *tx, run_id).await?;

    // 3. Persist outputs
    if let Some(outputs) = payload["outputs"].as_object() {
        for (key, value) in outputs {
            crate::db::upsert_step_output(&mut *tx, step_id, key, value).await?;
        }
    }

    // 3.5 Persist storage hashes if provided
    if let Some(storage_hashes) = payload["storage_hashes"].as_object() {
        for (name, hash_val) in storage_hashes {
            if let Some(hash) = hash_val.as_str() {
                crate::db::upsert_run_storage_state(&mut *tx, run_id, name, hash).await?;
            }
        }
    }

    // 3.6 Persist artifact metadata if provided
    if let Some(artifacts) = payload["artifacts"].as_object() {
        let context = fetch_run_context(run_id, &mut *tx).await?;
        let workflow: Workflow = serde_json::from_value(context.workflow_definition)
            .context("Failed to parse workflow definition from DB")?;

        for (name, meta) in artifacts {
            // Find which storage this artifact belongs to
            let storage = workflow
                .storage
                .iter()
                .find(|s| s.artifacts.iter().any(|a| a.name == *name));
            if let Some(s) = storage {
                let backend_id: Option<Uuid> = if let Some(ref backend_name) = s.backend {
                    crate::db::get_storage_backend_id_by_name(&mut *tx, backend_name).await?
                } else {
                    crate::db::get_default_sfs_backend_id(&mut *tx).await?
                };

                if let Some(bid) = backend_id {
                    let remote_path =
                        format!("artifacts/{}/{}/{}/{}", run_id, step_id, s.name, name);
                    crate::db::insert_artifact_registry(
                        &mut *tx,
                        run_id,
                        step_id,
                        name,
                        bid,
                        remote_path,
                        meta.clone(),
                    )
                    .await?;
                }
            }
        }
    }

    // 3.8 Persist test reports if provided
    persist_step_test_reports(&payload, &mut tx, run_id, step_id, &pool).await?;

    // 2.5 Scrape outputs from logs if configured
    let context = fetch_run_context(run_id, &mut *tx).await?;
    let workflow: Workflow = serde_json::from_value(context.workflow_definition)
        .context("Failed to parse workflow definition from DB")?;

    let all_steps_initial: Vec<StepInstance> =
        crate::db::get_step_instances_by_run_id(&mut *tx, run_id).await?;

    let current_step_instance = all_steps_initial
        .iter()
        .find(|s| s.id == step_id)
        .context("Completed step not found in DB")?;

    let mut dsl_step = find_step(&workflow.steps, &current_step_instance.step_name).cloned();

    if let Some(step) = &mut dsl_step {
        if step.r#type == "TerraformApply" {
            step.outputs.push(stormchaser_model::dsl::OutputExtraction {
                name: "terraform".to_string(),
                source: "stdout".to_string(),
                marker: Some("--- TF OUTPUTS ---".to_string()),
                format: Some("json".to_string()),
                regex: Some(r"--- TF OUTPUTS ---\s*(.*)".to_string()),
                group: Some(1),
                sensitive: Some(false),
            });
        } else if step.r#type == "TerraformPlan" {
            step.outputs.push(stormchaser_model::dsl::OutputExtraction {
                name: "plan_summary".to_string(),
                source: "stdout".to_string(),
                marker: Some("--- TF PLAN SUMMARY ---".to_string()),
                format: Some("string".to_string()),
                regex: Some(r"--- TF PLAN SUMMARY ---\s*(.*)".to_string()),
                group: Some(1),
                sensitive: Some(false),
            });
            step.outputs.push(stormchaser_model::dsl::OutputExtraction {
                name: "plan_json".to_string(),
                source: "stdout".to_string(),
                marker: Some("--- TF PLAN JSON ---".to_string()),
                format: Some("json".to_string()),
                regex: Some(r"--- TF PLAN JSON ---\s*(.*)".to_string()),
                group: Some(1),
                sensitive: Some(false),
            });
        }
    }

    if let (Some(dsl_step), Some(backend)) = (&dsl_step, &*log_backend) {
        if !dsl_step.outputs.is_empty() {
            tracing::info!("Scraping outputs from logs for step {}", dsl_step.name);
            let logs = backend
                .fetch_step_logs(
                    &dsl_step.name,
                    step_id,
                    current_step_instance.started_at,
                    current_step_instance.finished_at,
                    Some(5000), // Get up to 5000 lines for output scraping
                )
                .await
                .unwrap_or_default();

            let mut filtered_logs = Vec::new();
            let mut in_output_block = dsl_step.start_marker.is_none();

            for line in &logs {
                if let Some(start) = &dsl_step.start_marker {
                    if line.contains(start) {
                        in_output_block = true;
                        continue;
                    }
                }
                if let Some(end) = &dsl_step.end_marker {
                    if line.contains(end) {
                        in_output_block = false;
                        continue;
                    }
                }
                if in_output_block {
                    filtered_logs.push(line);
                }
            }

            for extraction in &dsl_step.outputs {
                if extraction.source == "logs" || extraction.source == "stdout" {
                    if let Some(regex_str) = &extraction.regex {
                        if let Ok(re) = regex::Regex::new(regex_str) {
                            for line in filtered_logs.iter().rev() {
                                if let Some(caps) = re.captures(line) {
                                    let value = if let Some(marker) = &extraction.marker {
                                        if line.contains(marker) {
                                            caps.get(extraction.group.unwrap_or(1) as usize)
                                                .map(|m| m.as_str().to_string())
                                        } else {
                                            None
                                        }
                                    } else {
                                        caps.get(extraction.group.unwrap_or(1) as usize)
                                            .map(|m| m.as_str().to_string())
                                    };

                                    if let Some(val) = value {
                                        let final_val =
                                            if extraction.format.as_deref() == Some("json") {
                                                serde_json::from_str(&val)
                                                    .unwrap_or(serde_json::json!(val))
                                            } else {
                                                serde_json::json!(val)
                                            };

                                        crate::db::upsert_step_output_with_sensitivity(
                                            &mut *tx,
                                            step_id,
                                            &extraction.name,
                                            &final_val,
                                            extraction.sensitive.unwrap_or(false),
                                        )
                                        .await?;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some(dsl_step) = dsl_step {
        // 3.1 Check if this step is iterated and if we need to schedule more batches
        let all_instances_of_this_step: Vec<&StepInstance> = all_steps
            .iter()
            .filter(|s| s.step_name == dsl_step.name)
            .collect();

        let finished_instances = all_instances_of_this_step
            .iter()
            .filter(|s| s.status == StepStatus::Succeeded || s.status == StepStatus::Skipped)
            .count();

        let total_instances = all_instances_of_this_step.len();

        if finished_instances < total_instances {
            // Some instances are still running or waiting
            let waiting_instances: Vec<&&StepInstance> = all_instances_of_this_step
                .iter()
                .filter(|s| s.status == StepStatus::WaitingForEvent)
                .collect();

            if !waiting_instances.is_empty() {
                let running_or_pending = all_instances_of_this_step
                    .iter()
                    .filter(|s| s.status == StepStatus::Running || s.status == StepStatus::Pending)
                    .count();

                let max_parallel = dsl_step
                    .strategy
                    .as_ref()
                    .and_then(|s| s.max_parallel)
                    .unwrap_or(u32::MAX);

                if (running_or_pending as u32) < max_parallel {
                    let to_schedule = max_parallel - (running_or_pending as u32);
                    for next_instance in waiting_instances.iter().take(to_schedule as usize) {
                        let machine =
                            crate::step_machine::StepMachine::<
                                crate::step_machine::state::WaitingForEvent,
                            >::from_instance((**next_instance).clone());
                        let _ = machine.reschedule(&mut *tx).await?;

                        let inst_data: (Value, Value) =
                            crate::db::get_step_spec_and_params(&mut *tx, next_instance.id).await?;

                        dispatch_step_instance(
                            run_id,
                            next_instance.id,
                            &dsl_step.name,
                            &dsl_step.r#type,
                            &inst_data.0,
                            &inst_data.1,
                            nats_client.clone(),
                            pool.clone(),
                            tls_reloader.clone(),
                        )
                        .await?;
                    }
                }
            }
            tx.commit().await?;
            return Ok(());
        }

        // 4. All instances of THIS step are done, evaluate successors
        if !dsl_step.next.is_empty() {
            let hcl_ctx = crate::hcl_eval::create_context(
                context.inputs.clone(),
                run_id,
                fetch_outputs(run_id, &mut *tx).await?,
            );

            for next_step_name in &dsl_step.next {
                let predecessors: Vec<&ast::Step> = workflow
                    .steps
                    .iter()
                    .filter(|s| s.next.contains(next_step_name))
                    .collect();

                let all_predecessors_done = predecessors.iter().all(|pred_dsl| {
                    let pred_instances: Vec<&StepInstance> = all_steps
                        .iter()
                        .filter(|s| s.step_name == pred_dsl.name)
                        .collect();

                    !pred_instances.is_empty()
                        && pred_instances.iter().all(|s| {
                            s.status == StepStatus::Succeeded || s.status == StepStatus::Skipped
                        })
                });

                if all_predecessors_done {
                    if let Some(next_dsl) =
                        workflow.steps.iter().find(|s| s.name == *next_step_name)
                    {
                        #[allow(clippy::explicit_auto_deref)]
                        schedule_step(
                            run_id,
                            next_dsl,
                            &mut *tx,
                            nats_client.clone(),
                            &hcl_ctx,
                            pool.clone(),
                            &workflow,
                        )
                        .await?;
                    }
                }
            }
        }
    }

    // 5. Check if all steps in the workflow are Done
    let all_steps_final: Vec<StepInstance> =
        crate::db::get_step_instances_by_run_id(&mut *tx, run_id).await?;

    let all_dsl_steps_done = workflow.steps.iter().all(|dsl_step| {
        let step_instances: Vec<&StepInstance> = all_steps_final
            .iter()
            .filter(|s| s.step_name == dsl_step.name)
            .collect();

        !step_instances.is_empty()
            && step_instances
                .iter()
                .all(|s| s.status == StepStatus::Succeeded || s.status == StepStatus::Skipped)
    });

    if all_dsl_steps_done {
        let run = fetch_run(run_id, &mut *tx).await?;
        let machine = WorkflowMachine::<state::Running>::new_from_run(run.clone());
        let _ = machine.succeed(&mut *tx).await?;

        crate::RUNS_COMPLETED.add(
            1,
            &[
                opentelemetry::KeyValue::new("workflow_name", run.workflow_name),
                opentelemetry::KeyValue::new("initiating_user", run.initiating_user),
            ],
        );

        tx.commit().await?;
        info!(
            "Workflow run {} completed successfully, archiving...",
            run_id
        );
        archive_workflow(run_id, pool.clone()).await?;
        return Ok(());
    } else {
        tx.commit().await?;
    }

    if let Err(e) = dispatch_pending_steps(run_id, pool, nats_client, tls_reloader).await {
        error!(
            "Failed to dispatch pending steps for run {}: {:?}",
            run_id, e
        );
    }

    Ok(())
}

#[tracing::instrument(skip(payload, pool, nats_client, tls_reloader), fields(run_id = tracing::field::Empty, step_id = tracing::field::Empty))]
/// Handle step failed.
pub async fn handle_step_failed(
    payload: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    let run_id_str = payload["run_id"].as_str().context("Missing run_id")?;
    let run_id = Uuid::parse_str(run_id_str)?;
    let step_id_str = payload["step_id"].as_str().context("Missing step_id")?;
    let step_id = Uuid::parse_str(step_id_str)?;

    let span = tracing::Span::current();
    span.record("run_id", tracing::field::display(run_id));
    span.record("step_id", tracing::field::display(step_id));
    let error_msg = payload["error"].as_str().unwrap_or("Unknown error");
    let exit_code = payload["exit_code"].as_i64().map(|c| c as i32);

    info!("Step {} (Run {}) failed: {}", step_id, run_id, error_msg);

    let mut tx = pool.begin().await?;

    if crate::db::lock_workflow_run(&mut *tx, run_id)
        .await?
        .is_none()
    {
        return Ok(());
    }

    let instance = fetch_step_instance(step_id, &mut *tx).await?;

    // Ensure we don't process duplicate completion events
    if instance.status == StepStatus::Succeeded || instance.status == StepStatus::Failed {
        return Ok(());
    }

    let _ = release_step_quota_for_instance(&mut *tx, run_id, step_id).await;

    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Running>::from_instance(
            instance.clone(),
        );
    let _ = machine
        .fail(error_msg.to_string(), exit_code, &mut *tx)
        .await?;

    let attributes = [
        opentelemetry::KeyValue::new("step_name", instance.step_name),
        opentelemetry::KeyValue::new("step_type", instance.step_type),
        opentelemetry::KeyValue::new("runner_id", instance.runner_id.unwrap_or_default()),
        opentelemetry::KeyValue::new("error", error_msg.to_string()),
    ];

    crate::STEPS_FAILED.add(1, &attributes);

    if let Some(started_at) = instance.started_at {
        let duration = (Utc::now() - started_at).to_std().unwrap_or_default();
        crate::STEP_DURATION.record(duration.as_secs_f64(), &attributes);
    }

    if let Some(outputs) = payload["outputs"].as_object() {
        for (key, value) in outputs {
            crate::db::upsert_step_output(&mut *tx, step_id, key, value).await?;
        }
    }

    // Persist test reports even on failure
    persist_step_test_reports(&payload, &mut tx, run_id, step_id, &pool).await?;

    let run = fetch_run(run_id, &mut *tx).await?;
    let run_machine = WorkflowMachine::<state::Running>::new_from_run(run.clone());
    let _ = run_machine
        .fail(format!("Step {} failed: {}", step_id, error_msg), &mut *tx)
        .await?;

    crate::RUNS_FAILED.add(
        1,
        &[
            opentelemetry::KeyValue::new("workflow_name", run.workflow_name),
            opentelemetry::KeyValue::new("initiating_user", run.initiating_user),
            opentelemetry::KeyValue::new("error", format!("Step {} failed", step_id)),
        ],
    );

    tx.commit().await?;
    info!("Workflow run {} failed, archiving...", run_id);
    archive_workflow(run_id, pool.clone()).await?;

    if let Err(e) = dispatch_pending_steps(run_id, pool, nats_client, tls_reloader).await {
        error!(
            "Failed to dispatch pending steps after failure for run {}: {:?}",
            run_id, e
        );
    }

    Ok(())
}

async fn persist_step_test_reports(
    payload: &Value,
    tx: &mut Transaction<'_, Postgres>,
    run_id: Uuid,
    step_id: Uuid,
    pool: &PgPool,
) -> Result<()> {
    if let Some(reports) = payload["test_reports"].as_object() {
        for (_key, report_val) in reports {
            let name = report_val
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let file_name = report_val
                .get("file_name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let format = report_val
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let hash = report_val
                .get("hash")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if report_val
                .get("is_claim")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                let remote_path = report_val.get("remote_path").and_then(|v| v.as_str());
                let backend_id = report_val.get("backend_id").and_then(|v| {
                    if let Some(s) = v.as_str() {
                        Uuid::parse_str(s).ok()
                    } else {
                        None
                    }
                });

                if let (Some(path), Some(bid)) = (remote_path, backend_id) {
                    // Download and parse
                    let backend: StorageBackend =
                        crate::db::storage::get_storage_backend_by_id(pool, bid)
                            .await?
                            .ok_or_else(|| anyhow::anyhow!("Storage backend not found"))?;

                    let client = crate::s3::get_s3_client(&backend).await?;
                    let bucket = backend.config["bucket"]
                        .as_str()
                        .context("Missing bucket")?;
                    let response = client.get_object().bucket(bucket).key(path).send().await?;
                    let data = response.body.collect().await?.to_vec();

                    // It's a tar.gz
                    let mut summaries = Vec::new();
                    let mut test_cases = Vec::new();
                    let mut raw_contents = Vec::new();

                    {
                        // Scope for !Send Archive
                        let tar_gz = GzDecoder::new(&data[..]);
                        let mut archive = Archive::new(tar_gz);
                        for entry in archive.entries()? {
                            let mut entry = entry?;
                            let mut content = String::new();
                            entry.read_to_string(&mut content)?;

                            if format == "junit" {
                                if let Ok((summary, cases)) =
                                    crate::junit::parse_junit(&content, name, run_id, step_id)
                                {
                                    summaries.push(summary);
                                    test_cases.extend(cases);
                                }
                            }
                            raw_contents.push(content);
                        }
                    }

                    for case in test_cases {
                        crate::db::insert_step_test_case(&mut **tx, run_id, step_id, name, &case)
                            .await?;
                    }

                    if let Some(final_summary) = crate::junit::aggregate_summaries(&summaries) {
                        crate::db::insert_step_test_summary(
                            &mut **tx,
                            run_id,
                            step_id,
                            name,
                            &final_summary,
                        )
                        .await?;
                    }

                    let combined_raw = raw_contents.join("\n---\n");

                    crate::db::insert_step_test_report(
                        &mut **tx,
                        run_id,
                        step_id,
                        name,
                        file_name,
                        format,
                        Some(&combined_raw),
                        hash,
                        Some(bid),
                        Some(path),
                    )
                    .await?;
                }
            } else if let Some(content) = report_val.get("content").and_then(|v| v.as_str()) {
                // Legacy in-memory report
                if format == "junit" {
                    if let Ok((summary, cases)) =
                        crate::junit::parse_junit(content, name, run_id, step_id)
                    {
                        crate::db::insert_step_test_summary(
                            &mut **tx, run_id, step_id, name, &summary,
                        )
                        .await?;
                        for case in cases {
                            crate::db::insert_step_test_case(
                                &mut **tx, run_id, step_id, name, &case,
                            )
                            .await?;
                        }
                    }
                }

                crate::db::insert_step_test_report(
                    &mut **tx,
                    run_id,
                    step_id,
                    name,
                    file_name,
                    format,
                    Some(content),
                    hash,
                    None,
                    None,
                )
                .await?;
            }
        }
    }
    Ok(())
}

/// Handles incoming queries for step status or output data over NATS.
pub async fn handle_step_query(
    payload: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    reply: Option<String>,
) -> Result<()> {
    let step_id_str = payload["step_id"].as_str().context("Missing step_id")?;
    let step_id = Uuid::parse_str(step_id_str)?;

    let step: Option<StepInstance> = crate::db::get_step_instance_by_id(&pool, step_id)
        .await
        .map(|v: Option<StepInstance>| v)?;

    if let Some(reply_subject) = reply {
        let response = if let Some(s) = step {
            serde_json::json!({
                "step_id": step_id,
                "status": s.status,
                "exists": true
            })
        } else {
            serde_json::json!({
                "step_id": step_id,
                "exists": false
            })
        };
        nats_client
            .publish(reply_subject, response.to_string().into())
            .await?;
    }

    Ok(())
}
