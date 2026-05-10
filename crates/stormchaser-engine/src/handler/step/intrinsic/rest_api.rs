use crate::handler::{fetch_outputs, fetch_run_context, fetch_step_instance};
use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_model::dsl::RestApiSpec;
use stormchaser_model::events::{EventSource, EventType, StepCompletedEvent, StepEventType};
use stormchaser_model::nats::NatsSubject;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_tls::TlsReloader;
use tracing::info;

/// Attempts to dispatch a REST API step instance.
pub async fn try_dispatch(
    run_id: RunId,
    step_instance_id: StepInstanceId,
    step_type: &str,
    resolved_spec: &Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    _tls_reloader: Arc<TlsReloader>,
) -> Result<bool> {
    if step_type == "RestApi" {
        let pool = pool.clone();
        let nats_client = nats_client.clone();
        let spec = resolved_spec.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_rest_api_invoke(
                run_id,
                step_instance_id,
                spec,
                pool.clone(),
                nats_client.clone(),
            )
            .await
            {
                if let Ok(instance) = fetch_step_instance(step_instance_id, &pool).await {
                    let machine = crate::step_machine::StepMachine::<
                        crate::step_machine::state::Pending,
                    >::from_instance(instance);
                    if let Ok(mut conn) = pool.acquire().await {
                        if let Ok(_machine) = machine
                            .start("error-recovery".to_string(), &mut *conn)
                            .await
                        {
                            if let Ok(instance) = fetch_step_instance(step_instance_id, &pool).await
                            {
                                let machine =
                                    crate::step_machine::StepMachine::<
                                        crate::step_machine::state::Running,
                                    >::from_instance(instance);
                                let _ = machine
                                    .fail(
                                        format!("RestApi invoke failed: {:?}", e),
                                        None,
                                        &mut *conn,
                                    )
                                    .await;
                            }
                        }
                    }
                }
            }
        });
        return Ok(true);
    }

    Ok(false)
}

async fn handle_rest_api_invoke(
    run_id: RunId,
    step_id: StepInstanceId,
    spec: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    let spec: RestApiSpec = serde_json::from_value(spec)?;

    info!("Invoking REST API {} for run {}", spec.url, run_id);

    // 1. Mark as Running
    let instance = fetch_step_instance(step_id, &pool).await?;
    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(
            instance,
        );
    let _ = machine
        .start("rest_api".to_string(), &mut *pool.acquire().await?)
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
    let rendered_body = render_request_body(&spec, &template_ctx)?;

    // 4. Build and Execute Request
    execute_request(run_id, step_id, &spec, rendered_body, pool, nats_client).await
}

fn render_request_body(spec: &RestApiSpec, template_ctx: &Value) -> Result<Option<String>> {
    use minijinja::Environment;
    if let Some(body_tmpl) = &spec.body {
        let env = Environment::new();
        Ok(Some(env.render_str(body_tmpl, template_ctx).map_err(
            |e| anyhow::anyhow!("Failed to render request body: {:?}", e),
        )?))
    } else {
        Ok(None)
    }
}

async fn execute_request(
    run_id: RunId,
    step_id: StepInstanceId,
    spec: &RestApiSpec,
    rendered_body: Option<String>,
    _pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    let client = reqwest::Client::new();
    let method = match spec
        .method
        .as_deref()
        .unwrap_or("GET")
        .to_uppercase()
        .as_str()
    {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "PATCH" => reqwest::Method::PATCH,
        _ => reqwest::Method::GET,
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
        let body_text = String::from_utf8_lossy(&body_bytes).to_string();

        let mut final_outputs = serde_json::Map::new();
        final_outputs.insert("response".to_string(), body_val.clone());

        if let Some(extractors) = &spec.extractors {
            for ext in extractors {
                if ext.format.as_deref() == Some("json") {
                    if let Some(path) = &ext.regex {
                        // Use JSON pointer for field extraction
                        let pointer_path = if path.starts_with('/') {
                            path.clone()
                        } else {
                            format!("/{}", path.replace(".", "/"))
                        };
                        if let Some(val) = body_val.pointer(&pointer_path) {
                            final_outputs.insert(ext.name.clone(), val.clone());
                        }
                    } else {
                        final_outputs.insert(ext.name.clone(), body_val.clone());
                    }
                } else if ext.format.as_deref() == Some("regex") {
                    if let Some(regex_str) = &ext.regex {
                        if let Ok(re) = regex::Regex::new(regex_str) {
                            if let Some(caps) = re.captures(&body_text) {
                                if let Some(val) = caps
                                    .get(ext.group.unwrap_or(1) as usize)
                                    .map(|m| m.as_str().to_string())
                                {
                                    final_outputs.insert(ext.name.clone(), serde_json::json!(val));
                                }
                            }
                        }
                    }
                }
            }
        }

        let event = StepCompletedEvent {
            run_id,
            step_id,
            event_type: EventType::Step(StepEventType::Completed),
            test_reports: None,
            artifacts: None,
            storage_hashes: None,
            outputs: Some(final_outputs.into_iter().collect()),
            timestamp: chrono::Utc::now(),
            exit_code: Some(0),
            runner_id: Some("engine-intrinsic".to_string()),
        };

        let js = async_nats::jetstream::new(nats_client);
        let _ = stormchaser_model::nats::publish_cloudevent(
            &js,
            NatsSubject::StepCompleted,
            EventType::Step(StepEventType::Completed),
            EventSource::System,
            serde_json::to_value(event).unwrap(),
            None,
            None,
        )
        .await;

        Ok(())
    } else {
        let error_body = res
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        anyhow::bail!("RestApi failed with status {}: {}", status, error_body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_render_request_body() {
        let spec = RestApiSpec {
            url: "http://example.com".to_string(),
            method: Some("POST".to_string()),
            headers: None,
            body: Some("Hello {{ inputs.name }}!".to_string()),
            timeout: None,
            extractors: None,
        };
        let ctx = json!({
            "inputs": {
                "name": "World"
            }
        });
        let rendered = render_request_body(&spec, &ctx).unwrap();
        assert_eq!(rendered, Some("Hello World!".to_string()));
    }
}
