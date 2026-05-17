use crate::handler::{fetch_outputs, fetch_run_context, fetch_step_instance};
use anyhow::Result;
use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use std::time::Duration;
use stormchaser_model::dsl::WebhookInvokeSpec;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use tracing::info;

/// Handle webhook invoke.
pub async fn handle_webhook_invoke(
    run_id: RunId,
    step_id: StepInstanceId,
    spec: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    let spec: WebhookInvokeSpec = serde_json::from_value(spec)?;

    info!("Invoking webhook {} for run {}", spec.url, run_id);

    // 1. Mark as Running
    let instance = fetch_step_instance(step_id, &pool).await?;
    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(
            instance,
        );
    let _ = machine
        .start("webhook".to_string(), &mut *pool.acquire().await?)
        .await?;

    // 2. Prepare Context for Template Rendering
    let run_context = fetch_run_context(run_id, &pool).await?;
    let outputs = fetch_outputs(run_id, &pool).await?;

    let template_ctx = serde_json::json!({
        "inputs": run_context.inputs,
        "steps": outputs,
        "run": {
            "id": run_id.to_string(),
        }
    });

    // 3. Render Body if present
    let rendered_body = render_webhook_body(&spec, &template_ctx)?;

    // 4. Build and Execute Request
    execute_webhook_request(run_id, step_id, &spec, rendered_body, pool, nats_client).await
}

use stormchaser_model::dsl;

fn render_webhook_body(
    spec: &dsl::WebhookInvokeSpec,
    template_ctx: &Value,
) -> Result<Option<String>> {
    use minijinja::Environment;
    if let Some(body_tmpl) = &spec.body {
        let env = Environment::new();
        Ok(Some(env.render_str(body_tmpl, template_ctx).map_err(
            |e| anyhow::anyhow!("Failed to render webhook body: {:?}", e),
        )?))
    } else {
        Ok(None)
    }
}

async fn execute_webhook_request(
    run_id: RunId,
    step_id: StepInstanceId,
    spec: &dsl::WebhookInvokeSpec,
    rendered_body: Option<String>,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    let client = reqwest::Client::new();
    let method = match spec
        .method
        .as_deref()
        .unwrap_or("POST")
        .to_uppercase()
        .as_str()
    {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "PATCH" => reqwest::Method::PATCH,
        _ => reqwest::Method::POST,
    };

    let mut builder = client.request(method, &spec.url);

    if let Some(headers) = &spec.headers {
        for (k, v) in headers {
            builder = builder.header(k, v);
        }
    }

    if let Some(body) = rendered_body {
        builder = builder.body(body);
    }

    let timeout = spec
        .timeout
        .as_ref()
        .and_then(|t| humantime::parse_duration(t).ok())
        .unwrap_or(Duration::from_secs(30));

    let res = builder.timeout(timeout).send().await?;
    let status = res.status();

    if status.is_success() {
        let body_bytes = res.bytes().await?;
        let body_val: Value = serde_json::from_slice(&body_bytes).unwrap_or_else(
            |_| serde_json::json!({ "text": String::from_utf8_lossy(&body_bytes) }),
        );

        let instance = fetch_step_instance(step_id, &pool).await?;
        let machine =
            crate::step_machine::StepMachine::<crate::step_machine::state::Running>::from_instance(
                instance,
            );
        let _ = machine.succeed(&mut *pool.acquire().await?).await?;

        let event = serde_json::json!({
            "run_id": run_id,
            "step_id": step_id,
            "event_type": "step_completed",
            "outputs": body_val,
            "timestamp": Utc::now(),
        });
        let js = async_nats::jetstream::new(nats_client);
        js.publish("stormchaser.step.completed", event.to_string().into())
            .await?;
        Ok(())
    } else {
        let error_body = res
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        anyhow::bail!("Webhook failed with status {}: {}", status, error_body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_render_webhook_body_happy_path() {
        let spec = WebhookInvokeSpec {
            url: "http://example.com".to_string(),
            method: Some("POST".to_string()),
            headers: None,
            body: Some("Hello {{ inputs.name }}!".to_string()),
            timeout: None,
        };
        let ctx = json!({
            "inputs": {
                "name": "World"
            }
        });
        let rendered = render_webhook_body(&spec, &ctx).unwrap();
        assert_eq!(rendered, Some("Hello World!".to_string()));
    }

    #[test]
    fn test_render_webhook_body_no_body() {
        let spec = WebhookInvokeSpec {
            url: "http://example.com".to_string(),
            method: Some("POST".to_string()),
            headers: None,
            body: None,
            timeout: None,
        };
        let ctx = json!({});
        let rendered = render_webhook_body(&spec, &ctx).unwrap();
        assert_eq!(rendered, None);
    }

    #[test]
    fn test_render_webhook_body_error() {
        let spec_invalid = WebhookInvokeSpec {
            url: "http://example.com".to_string(),
            method: None,
            headers: None,
            body: Some("Hello {{ inputs.name".to_string()), // missing closing }}
            timeout: None,
        };
        let ctx = json!({"inputs": {"name": "World"}});
        let result = render_webhook_body(&spec_invalid, &ctx);
        result.unwrap_err();
    }

    #[test]
    fn test_render_webhook_body_with_complex_ctx() {
        let spec = WebhookInvokeSpec {
            url: "http://example.com".to_string(),
            method: None,
            headers: None,
            body: Some("Run ID: {{ run.id }}, Step result: {{ steps.test.status }}".to_string()),
            timeout: None,
        };
        let ctx = json!({
            "run": {
                "id": "123"
            },
            "steps": {
                "test": {
                    "status": "success"
                }
            }
        });
        let rendered = render_webhook_body(&spec, &ctx).unwrap();
        assert_eq!(
            rendered,
            Some("Run ID: 123, Step result: success".to_string())
        );
    }
}
