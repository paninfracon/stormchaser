use crate::handler::{fetch_outputs, fetch_run_context, fetch_step_instance};
use anyhow::Result;
use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use tracing::info;
use uuid::Uuid;

/// Handle jinja render.
pub async fn handle_jinja_render(
    run_id: Uuid,
    step_id: Uuid,
    spec: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    use stormchaser_model::dsl::JinjaRenderSpec;

    let spec: JinjaRenderSpec = serde_json::from_value(spec)?;

    info!("Rendering Jinja template for run {}", run_id);

    // 1. Mark as Running
    let instance = fetch_step_instance(step_id, &pool).await?;
    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(
            instance,
        );
    let _ = machine
        .start("jinja".to_string(), &mut *pool.acquire().await?)
        .await?;

    // 2. Prepare Context for Template Rendering
    let run_context = fetch_run_context(run_id, &pool).await?;
    let outputs = fetch_outputs(run_id, &pool).await?;

    let template_ctx = prepare_template_context(&spec, run_context, outputs, run_id);

    // 3. Render Template
    let rendered = render_template(&spec.template, &template_ctx)?;

    // 4. Save Output and Complete Step
    save_output_and_complete(run_id, step_id, &spec, rendered, pool, nats_client).await
}

use stormchaser_model::dsl;

fn prepare_template_context(
    spec: &dsl::JinjaRenderSpec,
    run_context: crate::handler::RunContext,
    outputs: Value,
    run_id: Uuid,
) -> Value {
    let mut template_ctx = serde_json::json!({
        "inputs": run_context.inputs,
        "steps": outputs,
        "run": {
            "id": run_id.to_string(),
        }
    });

    // Merge custom context if provided
    if let Some(custom_ctx) = &spec.context {
        if let Some(obj) = template_ctx.as_object_mut() {
            if let Some(custom_obj) = custom_ctx.as_object() {
                for (k, v) in custom_obj {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
    }
    template_ctx
}

fn render_template(template: &str, context: &Value) -> Result<String> {
    use minijinja::Environment;
    let env = Environment::new();
    env.render_str(template, context)
        .map_err(|e| anyhow::anyhow!("Failed to render Jinja template: {:?}", e))
}

async fn save_output_and_complete(
    run_id: Uuid,
    step_id: Uuid,
    spec: &dsl::JinjaRenderSpec,
    rendered: String,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    let instance = fetch_step_instance(step_id, &mut *tx).await?;
    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Running>::from_instance(
            instance,
        );
    let _ = machine.succeed(&mut *tx).await?;

    let output_key = spec
        .output_key
        .clone()
        .unwrap_or_else(|| "result".to_string());
    crate::db::upsert_step_output(
        &mut *tx,
        step_id,
        &output_key,
        &Value::String(rendered.clone()),
    )
    .await?;

    tx.commit().await?;

    let mut outputs_map = std::collections::HashMap::new();
    outputs_map.insert(output_key.clone(), serde_json::json!(rendered));

    let event = stormchaser_model::events::StepCompletedEvent {
        run_id,
        step_id,
        event_type: "stormchaser.v1.step.completed".to_string(),
        outputs: Some(outputs_map),
        exit_code: Some(0),
        runner_id: None,
        storage_hashes: None,
        artifacts: None,
        test_reports: None,
        timestamp: Utc::now(),
    };
    let js = async_nats::jetstream::new(nats_client);
    stormchaser_model::nats::publish_cloudevent(
        &js,
        "stormchaser.v1.step.completed",
        "stormchaser.v1.step.completed",
        "/stormchaser",
        serde_json::to_value(event).unwrap(),
        Some("1.0"),
        None,
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use stormchaser_model::dsl::JinjaRenderSpec;
    use stormchaser_model::workflow::RunContext;
    use uuid::Uuid;

    #[test]
    fn test_prepare_template_context() {
        let run_id = Uuid::new_v4();
        let spec = JinjaRenderSpec {
            template: "Hello".to_string(),
            context: Some(json!({"extra": "value"})),
            output_key: None,
        };
        let run_context = RunContext {
            run_id,
            dsl_version: "1.0".to_string(),
            workflow_definition: json!({}),
            source_code: "".to_string(),
            inputs: json!({"name": "User"}),
            secrets: json!({}),
            sensitive_values: vec![],
        };
        let outputs = json!({"prev_step": {"outputs": {"res": "done"}}});

        let ctx = prepare_template_context(&spec, run_context, outputs, run_id);

        assert_eq!(ctx["inputs"]["name"], "User");
        assert_eq!(ctx["steps"]["prev_step"]["outputs"]["res"], "done");
        assert_eq!(ctx["run"]["id"], run_id.to_string());
        assert_eq!(ctx["extra"], "value");
    }

    #[test]
    fn test_render_template_happy() {
        let ctx = json!({"name": "World"});
        let rendered = render_template("Hello {{ name }}!", &ctx).unwrap();
        assert_eq!(rendered, "Hello World!");
    }

    #[test]
    fn test_render_template_error() {
        let ctx = json!({});
        let result = render_template("Hello {{ name", &ctx);
        assert!(result.is_err());
    }
}
