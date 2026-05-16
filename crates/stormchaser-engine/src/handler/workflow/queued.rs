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
use stormchaser_model::events::{EventSource, EventType, SchemaVersion, WorkflowEventType};
use stormchaser_model::RunId;
use stormchaser_tls::TlsReloader;
use tracing::{debug, error, info};

#[tracing::instrument(skip(pool, git_cache, opa_client, nats_client, _tls_reloader), fields(run_id = %run_id))]
/// Handles the event when a workflow is queued and ready for resolution.
pub async fn handle_workflow_queued(
    run_id: RunId,
    payload: serde_json::Value,
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

    let workflow_req_path = std::path::Path::new(&machine.run.workflow_path);
    if workflow_req_path.is_absolute()
        || workflow_req_path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        let err_msg = format!(
            "Workflow path {} is invalid (absolute or contains '..')",
            machine.run.workflow_path
        );
        let _ = machine
            .fail(err_msg.clone(), &mut *pool.acquire().await?)
            .await?;
        return Err(anyhow::anyhow!(err_msg));
    }
    let storm_file_path = repo_path.join(workflow_req_path);
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

        let req_inc_path = std::path::Path::new(&inc.workflow);
        if req_inc_path.is_absolute()
            || req_inc_path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            let err_msg = format!(
                "Included workflow path {} is invalid (absolute or contains '..')",
                inc.workflow
            );
            let _ = machine
                .fail(err_msg.clone(), &mut *pool.acquire().await?)
                .await?;
            return Err(anyhow::anyhow!(err_msg));
        }
        let inc_path = repo_path.join(req_inc_path);
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

            // Substitute include inputs into the step parameters using a word-boundary aware regex
            // to avoid matching similarly named variables (e.g. replacing 'inputs.id' shouldn't affect 'my_inputs.id')
            for (k, v) in &inc.inputs {
                let var_pattern = format!(r"\binputs\.{}\b", regex::escape(k));
                if let Ok(re) = regex::Regex::new(&var_pattern) {
                    let val_str = v.to_string();

                    for param_val in step.params.values_mut() {
                        *param_val = re.replace_all(param_val, val_str.as_str()).to_string();
                    }
                }
            }

            parsed_workflow.steps.push(step);
        }

        includes_to_process.extend(inc_workflow.includes);
    }

    // 5. Schema Validation and Default Value Hydration
    let mut inputs = fetch_inputs(run_id, &pool).await?;

    // Evaluate dynamic queries
    let mut query_results = serde_json::Map::new();
    for query in &parsed_workflow.queries {
        let hcl_ctx = crate::hcl_eval::create_context(
            inputs.clone(),
            run_id,
            serde_json::json!({}), // secrets
            serde_json::json!({}), // steps
            Some(&parsed_workflow),
            None,
        );
        let mut resolved_params = serde_json::to_value(&query.params)?;
        if let Err(e) = crate::hcl_eval::resolve_expressions(&mut resolved_params, &hcl_ctx, true) {
            let err_msg = format!(
                "Failed to evaluate parameters for query {}: {}",
                query.name, e
            );
            let _ = machine
                .fail(err_msg.clone(), &mut *pool.acquire().await?)
                .await?;
            return Err(anyhow::anyhow!(err_msg));
        }

        let resolved_params_map: std::collections::HashMap<String, String> =
            serde_json::from_value(resolved_params)?;

        // Execute the query
        let result_vec = match crate::query::execute_query(
            &query.r#type,
            &resolved_params_map,
            Some(&pool),
            None,
        )
        .await
        {
            Ok(res) => res,
            Err(e) => {
                let err_msg = format!("Failed to execute query {}: {}", query.name, e);
                let _ = machine
                    .fail(err_msg.clone(), &mut *pool.acquire().await?)
                    .await?;
                return Err(anyhow::anyhow!(err_msg));
            }
        };
        let result = serde_json::Value::Array(result_vec);

        query_results.insert(query.name.clone(), result);
    }

    if let Some(mut schema_val) = parsed_workflow.inputs_schema.clone() {
        // Resolve dynamic expressions (like query results) inside the schema
        let mut schema_ctx = crate::hcl_eval::create_context(
            inputs.clone(),
            run_id,
            serde_json::json!({}), // secrets
            serde_json::json!({}), // steps
            Some(&parsed_workflow),
            None,
        );
        schema_ctx.declare_var(
            "queries",
            crate::hcl_eval::json_to_hcl(serde_json::Value::Object(query_results)),
        );

        if let Err(e) = crate::hcl_eval::resolve_expressions(&mut schema_val, &schema_ctx, true) {
            let err_msg = format!("Failed to evaluate expressions in inputs schema: {}", e);
            let _ = machine
                .fail(err_msg.clone(), &mut *pool.acquire().await?)
                .await?;
            return Err(anyhow::anyhow!(err_msg));
        }

        // Hydrate defaults
        if let Some(properties) = schema_val.get("properties").and_then(|p| p.as_object()) {
            let mut inputs_obj = match inputs {
                serde_json::Value::Object(obj) => obj,
                _ => serde_json::Map::new(),
            };

            for (key, prop) in properties {
                if !inputs_obj.contains_key(key) {
                    if let Some(default_val) = prop.get("default") {
                        inputs_obj.insert(key.clone(), default_val.clone());
                    }
                }
            }
            inputs = serde_json::Value::Object(inputs_obj);
        }

        // Validate
        let compiled_schema = jsonschema::validator_for(&schema_val)
            .map_err(|e| anyhow::anyhow!("Failed to compile input schema: {}", e))?;

        if let Err(e) = compiled_schema.validate(&inputs) {
            let full_err = format!("Input validation failed: {}", e);
            let _ = machine
                .fail(full_err.clone(), &mut *pool.acquire().await?)
                .await?;
            return Err(anyhow::anyhow!(full_err));
        }
    }

    // 6. OPA Policy Check after Parsing
    // We send the full context: AST, user, and inputs
    let opa_context = EngineOpaContext {
        run_id,
        initiating_user: machine.run.initiating_user.clone(),
        workflow_ast: serde_json::to_value(&parsed_workflow)?,
        inputs: inputs.clone(),
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

    let sops_file: Option<String> = serde_json::from_value(payload["sops_file"].clone()).ok();
    let sops_role_arn: Option<String> =
        serde_json::from_value(payload["sops_role_arn"].clone()).ok();

    let (secrets, sensitive_values) = crate::handler::workflow::sops::decrypt_sops_secrets(
        &repo_path,
        sops_file.as_deref(),
        sops_role_arn.as_deref(),
    )
    .await
    .unwrap_or_else(|e| {
        error!("Failed to decrypt SOPS secrets for run {}: {:?}", run_id, e);
        (serde_json::json!({}), vec![])
    });

    // 7. Update RunContext with the definition and source code
    crate::db::update_run_context(
        &pool,
        serde_json::to_value(&parsed_workflow)?,
        Some(&workflow_content).map(|s| s.as_str()),
        &parsed_workflow.dsl_version,
        inputs,
        secrets,
        sensitive_values,
        run_id,
    )
    .await
    .with_context(|| format!("Failed to update run context for {}", run_id))?;

    // 8. Transition to StartPending
    let machine = machine.start_pending(&mut *pool.acquire().await?).await?;

    // Emit event for transition to StartPending
    let event = WorkflowStartPendingEvent {
        run_id,
        event_type: EventType::Workflow(WorkflowEventType::StartPending),
        timestamp: chrono::Utc::now(),
    };
    let js = async_nats::jetstream::new(nats_client);
    use stormchaser_model::nats::NatsSubject;
    stormchaser_model::nats::publish_cloudevent(
        &js,
        NatsSubject::RunStartPending(Some(stormchaser_model::nats::compute_shard_id(&run_id))),
        EventType::Workflow(WorkflowEventType::StartPending),
        EventSource::System,
        serde_json::to_value(event).unwrap(),
        Some(SchemaVersion::new("1.0".to_string())),
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
