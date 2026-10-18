//! The stages of resolving a run that a run from a repository and a direct run share.
//! Each returns its error with context, so that the caller can tell a transient
//! failure, worth another try, from one of the workflow's own.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use stormchaser_model::auth::{EngineOpaContext, OpaClient};
use stormchaser_model::dsl::Workflow;
use stormchaser_model::RunId;
use tracing::debug;

/// Asks policy whether the run may go ahead. Before any query runs, so that a denied
/// request cannot cause the SQL, API or AWS side effects of the queries' parameters.
pub(super) async fn check_policy(
    opa_client: &OpaClient,
    run_id: RunId,
    initiating_user: &str,
    workflow: &Workflow,
    inputs: &Value,
) -> Result<()> {
    let opa_context = EngineOpaContext {
        run_id,
        initiating_user: initiating_user.to_string(),
        workflow_ast: serde_json::to_value(workflow)?,
        inputs: inputs.clone(),
    };
    let allowed = opa_client
        .check_context(opa_context)
        .await
        .context("OPA check failed")?;
    if !allowed {
        bail!("Execution denied by OPA policy");
    }
    debug!("OPA allowed execution for run {}", run_id);
    Ok(())
}

/// Runs the workflow's queries, returning their results by name and the secrets their
/// parameters fetched
pub(super) async fn run_queries(
    run_id: RunId,
    initiating_user: &str,
    workflow: &Workflow,
    inputs: &Value,
    pool: &PgPool,
) -> Result<(serde_json::Map<String, Value>, Vec<String>)> {
    let mut results = serde_json::Map::new();
    let mut secrets = Vec::new();
    for query in &workflow.queries {
        let hcl_ctx = crate::hcl_eval::create_context(
            inputs.clone(),
            run_id,
            serde_json::json!({}), // secrets
            serde_json::json!({}), // steps
            Some(workflow),
            None,
        );
        let mut params = serde_json::to_value(&query.params)?;
        let (resolved, fetched) = crate::hcl_eval::capturing_secrets(|| {
            crate::hcl_eval::resolve_expressions(&mut params, &hcl_ctx, true)
        });
        secrets.extend(fetched);
        resolved
            .with_context(|| format!("Failed to evaluate parameters for query {}", query.name))?;
        let params: HashMap<String, String> = serde_json::from_value(params)?;

        let rows = crate::query::execute_query(
            &query.r#type,
            &params,
            Some(pool),
            None,
            Some(initiating_user),
        )
        .await
        .with_context(|| format!("Failed to execute query {}", query.name))?;
        results.insert(query.name.clone(), Value::Array(rows));
    }
    Ok((results, secrets))
}

/// Fills in the defaults of the workflow's inputs schema and validates the inputs
/// against it
pub(super) fn validate_inputs(
    run_id: RunId,
    workflow: &Workflow,
    inputs: Value,
    query_results: serde_json::Map<String, Value>,
) -> Result<Value> {
    let Some(mut schema) = workflow.inputs_schema.clone() else {
        return Ok(inputs);
    };
    // The schema can use the queries' results
    let mut schema_ctx = crate::hcl_eval::create_context(
        inputs.clone(),
        run_id,
        serde_json::json!({}), // secrets
        serde_json::json!({}), // steps
        Some(workflow),
        None,
    );
    schema_ctx.declare_var(
        "queries",
        crate::hcl_eval::json_to_hcl(Value::Object(query_results)),
    );
    crate::hcl_eval::resolve_expressions(&mut schema, &schema_ctx, true)
        .context("Failed to evaluate expressions in inputs schema")?;

    let mut inputs = inputs;
    if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
        let mut inputs_obj = match inputs {
            Value::Object(obj) => obj,
            _ => serde_json::Map::new(),
        };
        for (key, prop) in properties {
            if let Some(default_val) = prop.get("default") {
                inputs_obj
                    .entry(key.clone())
                    .or_insert_with(|| default_val.clone());
            }
        }
        inputs = Value::Object(inputs_obj);
    }

    let validator = jsonschema::validator_for(&schema)
        .map_err(|e| anyhow!("Failed to compile input schema: {}", e))?;
    if let Err(e) = validator.validate(&inputs) {
        bail!("Input validation failed: {}", e);
    }
    Ok(inputs)
}
