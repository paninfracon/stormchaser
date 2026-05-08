use sqlx::PgPool;

/// Retrieves run outputs for OPA evaluation.
pub async fn get_run_outputs_for_opa(
    pool: &PgPool,
    run_id: stormchaser_model::RunId,
) -> Result<serde_json::Map<String, serde_json::Value>, sqlx::Error> {
    use sqlx::Row;
    let outputs_rows = sqlx::query(
        r#"
        SELECT i.step_name, o.key as output_key, o.value as output_value
        FROM combined_step_instances i
        JOIN combined_step_outputs o ON i.id = o.step_instance_id
        WHERE i.run_id = $1
        "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;

    let mut run_outputs_map = serde_json::Map::new();
    for row in outputs_rows {
        let step_name: String = row.get("step_name");
        let output_key: String = row.get("output_key");
        let output_value: serde_json::Value = row.get("output_value");

        if !run_outputs_map.contains_key(&step_name) {
            run_outputs_map.insert(step_name.clone(), serde_json::json!({"outputs": {}}));
        }
        if let Some(step_obj) = run_outputs_map
            .get_mut(&step_name)
            .and_then(|v| v.as_object_mut())
        {
            if let Some(outputs_obj) = step_obj.get_mut("outputs").and_then(|v| v.as_object_mut()) {
                outputs_obj.insert(output_key, output_value);
            }
        }
    }
    Ok(run_outputs_map)
}

/// Retrieves workflow context for OPA evaluation.
pub async fn get_workflow_context_for_opa(
    pool: &PgPool,
    run_id: stormchaser_model::RunId,
) -> Result<Option<WorkflowOpaContextData>, sqlx::Error> {
    let context_row = sqlx::query(
        r#"
        SELECT
            wr.initiating_user,
            rc.workflow_definition,
            rc.inputs as run_inputs
        FROM workflow_runs wr
        JOIN run_contexts rc ON wr.id = rc.run_id
        WHERE wr.id = $1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?;

    if let Some(row) = context_row {
        use sqlx::Row;
        let initiating_user: String = row.get("initiating_user");
        let workflow_definition: serde_json::Value = row.get("workflow_definition");
        let run_inputs: serde_json::Value = row.get("run_inputs");
        Ok(Some(WorkflowOpaContextData {
            initiating_user,
            workflow_definition,
            run_inputs,
        }))
    } else {
        Ok(None)
    }
}

/// Data returned for workflow OPA context
pub struct WorkflowOpaContextData {
    pub initiating_user: String,
    pub workflow_definition: serde_json::Value,
    pub run_inputs: serde_json::Value,
}
