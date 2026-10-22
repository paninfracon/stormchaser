use super::resolve::{check_policy, run_queries, validate_inputs};
use crate::git_cache::GitCache;
use crate::handler::{fetch_inputs, fetch_run};
use crate::workflow_machine::{state, WorkflowMachine};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use stormchaser_dsl::StormchaserParser;
use stormchaser_model::dsl::{self, Workflow};
use stormchaser_model::events::WorkflowStartPendingEvent;
use stormchaser_model::events::{EventSource, EventType, SchemaVersion, WorkflowEventType};
use stormchaser_model::nats::NatsSubject;
use stormchaser_model::workflow::{RunStatus, WorkflowRun};
use stormchaser_model::RunId;
use stormchaser_opa::OpaClient;
use stormchaser_tls::TlsReloader;
use tracing::{debug, info};

#[tracing::instrument(skip(payload, pool, git_cache, opa_client, _nats_client, _tls_reloader), fields(run_id = %run_id))]
/// Handles the event when a workflow is queued and ready for resolution.
///
/// A failure of the workflow's own (an unreadable file, a denied policy, invalid
/// inputs) fails the run. A transient one (the database, a broker, a connection) leaves
/// the run resolving and returns the error, so that the event is delivered again and
/// resolution starts over; the resolver crash recovery worker fails a run that stays
/// resolving.
pub async fn handle_workflow_queued(
    run_id: RunId,
    payload: serde_json::Value,
    pool: PgPool,
    git_cache: Arc<GitCache>,
    opa_client: Arc<OpaClient>,
    _nats_client: async_nats::Client,
    _tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    info!("Handling queued workflow run: {}", run_id);

    let run = fetch_run(run_id, &pool).await?;
    // The stream can deliver this event again: after the run has moved on, which leaves
    // nothing to do, or after a transient failure left it resolving, to try again
    let machine = match run.status {
        RunStatus::Queued => {
            WorkflowMachine::<state::Queued>::load(run)?
                .start_resolving(&mut *pool.acquire().await?)
                .await?
        }
        RunStatus::Resolving => WorkflowMachine::<state::Resolving>::load(run)?,
        status => {
            debug!(
                "Run {} is {:?}, not queued; skipping its queued event",
                run_id, status
            );
            return Ok(());
        }
    };

    let resolved = match resolve(machine.run(), &payload, &pool, &git_cache, &opa_client).await {
        Ok(resolved) => resolved,
        Err(e) if crate::retry::is_transient(&e) => return Err(e),
        Err(e) => {
            let reason = format!("{:#}", e);
            info!("Run {} failed to resolve: {}", run_id, reason);
            machine.fail(reason, &mut *pool.acquire().await?).await?;
            return Ok(());
        }
    };

    let mut tx = pool.begin().await?;

    crate::db::update_run_context(
        &mut *tx,
        serde_json::to_value(&resolved.workflow)?,
        Some(resolved.source.as_str()),
        &resolved.workflow.dsl_version,
        resolved.inputs,
        resolved.secrets,
        resolved.sensitive_values,
        run_id,
    )
    .await
    .with_context(|| format!("Failed to update run context for {}", run_id))?;

    // The start_pending handler schedules the first steps and moves the run to Running
    machine.start_pending(&mut *tx).await?;
    let event = WorkflowStartPendingEvent {
        run_id,
        event_type: EventType::Workflow(WorkflowEventType::StartPending),
        timestamp: chrono::Utc::now(),
        status: RunStatus::StartPending,
    };
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
    .with_context(|| format!("Failed to enqueue start_pending event for {}", run_id))?;

    tx.commit().await?;

    info!(
        "Successfully resolved and parsed workflow file for run {}",
        run_id
    );
    Ok(())
}

/// What a run resolves to, ready to record in its context
struct Resolved {
    workflow: Workflow,
    source: String,
    inputs: Value,
    secrets: Value,
    sensitive_values: Vec<String>,
}

async fn resolve(
    run: &WorkflowRun,
    payload: &Value,
    pool: &PgPool,
    git_cache: &Arc<GitCache>,
    opa_client: &OpaClient,
) -> Result<Resolved> {
    let run_id = run.id;
    let repo_path = fetch_repo(run, git_cache).await?;
    let (workflow, source) = load_workflow(&repo_path, &run.workflow_path)?;
    let inputs = fetch_inputs(run_id, pool).await?;

    // Before any query runs, so that a denied request cannot cause the SQL, API or
    // AWS side effects of the queries' parameters
    check_policy(opa_client, run_id, &run.initiating_user, &workflow, &inputs).await?;

    let (query_results, query_secrets) =
        run_queries(run_id, &run.initiating_user, &workflow, &inputs, pool).await?;
    let inputs = validate_inputs(run_id, &workflow, inputs, query_results)?;

    // A run whose secrets cannot be read does not go ahead without them
    let sops_file: Option<String> = serde_json::from_value(payload["sops_file"].clone()).ok();
    let sops_role_arn: Option<String> =
        serde_json::from_value(payload["sops_role_arn"].clone()).ok();
    let (secrets, mut sensitive_values) = crate::handler::workflow::sops::decrypt_sops_secrets(
        &repo_path,
        sops_file.as_deref(),
        sops_role_arn.as_deref(),
    )
    .await
    .context("Failed to decrypt SOPS secrets")?;

    sensitive_values.extend(query_secrets);
    sensitive_values.extend(crate::hcl_eval::take_alias_secrets(run_id));
    sensitive_values.sort();
    sensitive_values.dedup();

    Ok(Resolved {
        workflow,
        source,
        inputs,
        secrets,
        sensitive_values,
    })
}

/// The run's repository, with its workflow file, in the git cache
async fn fetch_repo(run: &WorkflowRun, git_cache: &Arc<GitCache>) -> Result<PathBuf> {
    // Cloning blocks on the network, so it runs off the async executor
    let git_cache = git_cache.clone();
    let repo_url = run.repo_url.clone();
    let git_ref = run.git_ref.clone();
    let workflow_path = run.workflow_path.clone();
    tokio::task::spawn_blocking(move || {
        git_cache.ensure_files(&repo_url, &git_ref, std::slice::from_ref(&workflow_path))
    })
    .await
    .unwrap_or_else(|e| Err(anyhow!("Git resolution task failed: {}", e)))
    .context("Git resolution failed")
}

/// Refuses a path that could leave the repository
fn ensure_relative(path: &str, what: &str) -> Result<()> {
    let path_ref = Path::new(path);
    if path_ref.is_absolute()
        || path_ref
            .components()
            .any(|c| matches!(c, Component::ParentDir))
    {
        bail!(
            "{} path {} is invalid (absolute or contains '..')",
            what,
            path
        );
    }
    Ok(())
}

/// Reads and parses the run's workflow file and inlines the workflows it includes.
/// Files are read from the git object database, not the working tree, so a symlink
/// committed to the repository cannot redirect the read.
fn load_workflow(repo_path: &Path, workflow_path: &str) -> Result<(Workflow, String)> {
    ensure_relative(workflow_path, "Workflow")?;
    let source = crate::git_cache::read_blob_at_head(repo_path, workflow_path)
        .context("Failed to load workflow file")?;
    debug!("Workflow content loaded: {} bytes", source.len());

    let parser = StormchaserParser::new();
    let mut workflow = parser.parse(&source).context("Workflow parsing failed")?;

    let mut resolved_includes = HashSet::new();
    // They are inlined, so they leave the AST
    let mut includes_to_process = std::mem::take(&mut workflow.includes);
    while let Some(inc) = includes_to_process.pop() {
        if !resolved_includes.insert(inc.workflow.clone()) {
            continue; // Prevent infinite loops
        }
        ensure_relative(&inc.workflow, "Included workflow")?;
        let inc_source = crate::git_cache::read_blob_at_head(repo_path, &inc.workflow)
            .context("Failed to load included workflow")?;
        let inc_workflow = parser
            .parse(&inc_source)
            .with_context(|| format!("Included workflow {} parsing failed", inc.workflow))?;

        workflow.step_libraries.extend(inc_workflow.step_libraries);
        inline_steps(&mut workflow, &inc, inc_workflow.steps);
        includes_to_process.extend(inc_workflow.includes);
    }
    // The includes may bring step libraries the workflow's own steps name
    stormchaser_dsl::libraries::apply_step_libraries(&mut workflow)?;
    Ok((workflow, source))
}

/// Adds an included workflow's steps, prefixed with the include's name to avoid name
/// collisions, with the include's inputs substituted into their parameters
fn inline_steps(workflow: &mut Workflow, inc: &dsl::Include, steps: Vec<dsl::Step>) {
    let prefix = format!("{}.", inc.name);
    for mut step in steps {
        step.name = format!("{}{}", prefix, step.name);
        for next_ref in &mut step.next {
            *next_ref = format!("{}{}", prefix, next_ref);
        }
        // Word-boundary aware, so that replacing 'inputs.id' does not touch 'my_inputs.id'
        for (k, v) in &inc.inputs {
            let var_pattern = format!(r"\binputs\.{}\b", regex::escape(k));
            if let Ok(re) = regex::Regex::new(&var_pattern) {
                let val_str = v.to_string();
                for param_val in step.params.values_mut() {
                    *param_val = re.replace_all(param_val, val_str.as_str()).to_string();
                }
            }
        }
        workflow.steps.push(step);
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_include_input_substitution_regex() {
        let mut params = std::collections::HashMap::new();
        params.insert("cmd".to_string(), "echo ${inputs.id}".to_string());
        params.insert(
            "other".to_string(),
            "${my_inputs.id} and ${inputs.identity}".to_string(),
        );

        let mut inputs = std::collections::HashMap::new();
        inputs.insert("id".to_string(), serde_json::json!("123"));

        for (k, v) in &inputs {
            let var_pattern = format!(r"\binputs\.{}\b", regex::escape(k));
            if let Ok(re) = regex::Regex::new(&var_pattern) {
                let val_str = match v {
                    serde_json::Value::String(s) => s.to_string(),
                    _ => v.to_string(),
                };

                for param_val in params.values_mut() {
                    *param_val = re.replace_all(param_val, val_str.as_str()).to_string();
                }
            }
        }

        assert_eq!(params["cmd"], "echo ${123}");
        assert_eq!(params["other"], "${my_inputs.id} and ${inputs.identity}");
    }
}
