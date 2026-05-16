use crate::handler::{fetch_outputs, fetch_run_context, fetch_step_instance};
use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_model::dsl::{RestApiResponseExtractor, RestApiSpec};
use stormchaser_model::events::{
    EventSource, EventType, SchemaVersion, StepCompletedEvent, StepEventType,
};
use stormchaser_model::nats::NatsSubject;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_tls::TlsReloader;
use tracing::{info, warn};

/// Attempts to dispatch a REST API step instance.
pub async fn try_dispatch(
    run_id: RunId,
    step_instance_id: StepInstanceId,
    fencing_token: i64,
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
                fencing_token,
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

fn prepare_template_context(inputs: Value, outputs: Value, run_id: RunId) -> Value {
    serde_json::json!({
        "inputs": inputs,
        "steps": outputs,
        "run": {
            "id": run_id.to_string(),
        }
    })
}

async fn handle_rest_api_invoke(
    run_id: RunId,
    step_id: StepInstanceId,
    fencing_token: i64,
    spec: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    let mut spec: RestApiSpec = serde_json::from_value(spec.get("spec").unwrap_or(&spec).clone())?;

    // Load connection if specified
    if let Some(conn_name) = &spec.connection {
        if let Some(conn) = crate::db::connections::get_storage_backend_by_name::<
            _,
            stormchaser_model::Connection,
        >(&pool, conn_name)
        .await?
        {
            if conn.connection_type == stormchaser_model::connections::ConnectionType::HttpApi {
                if let Some(base_url) = conn.config.get("base_url").and_then(|v| v.as_str()) {
                    let mut url = spec.url.clone();
                    if !url.starts_with("http://") && !url.starts_with("https://") {
                        let base = base_url.trim_end_matches('/');
                        let path = url.trim_start_matches('/');
                        url = format!("{}/{}", base, path);
                    }
                    spec.url = url;
                }

                if let Some(headers) = conn.config.get("headers").and_then(|v| v.as_object()) {
                    let mut current_headers = spec.headers.unwrap_or_default();
                    for (k, v) in headers {
                        if let Some(s) = v.as_str() {
                            current_headers.insert(k.clone(), s.to_string());
                        }
                    }
                    spec.headers = Some(current_headers);
                }

                if let Some(auth_token) = conn.encrypted_credentials {
                    let mut current_headers = spec.headers.unwrap_or_default();
                    if !current_headers.contains_key("Authorization")
                        && !current_headers.contains_key("authorization")
                    {
                        current_headers.insert(
                            "Authorization".to_string(),
                            format!("Bearer {}", auth_token),
                        );
                    }
                    spec.headers = Some(current_headers);
                }
            } else {
                warn!("Connection {} is not of type HttpApi", conn_name);
            }
        } else {
            anyhow::bail!("Connection {} not found", conn_name);
        }
    }

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

    let template_ctx = prepare_template_context(run_context.inputs, outputs, run_id);

    // 3. Render Body if present
    let rendered_body = render_request_body(&spec, &template_ctx)?;

    // 4. Build and Execute Request
    execute_request(
        run_id,
        step_id,
        fencing_token,
        &spec,
        rendered_body,
        pool,
        nats_client,
    )
    .await
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

fn build_request_method(spec: &RestApiSpec) -> reqwest::Method {
    match spec
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
    }
}

fn apply_extractors(
    spec: &RestApiSpec,
    body_val: &Value,
    body_text: &str,
    final_outputs: &mut serde_json::Map<String, Value>,
) {
    if let Some(extractors) = &spec.extractors {
        for ext in extractors {
            match extractor_mode(ext) {
                ExtractorMode::Json => {
                    if let Some(path) = &ext.json_pointer {
                        let pointer_path = normalize_json_pointer(path);
                        if let Some(val) = body_val.pointer(&pointer_path) {
                            final_outputs.insert(ext.name.clone(), val.clone());
                        }
                    } else {
                        final_outputs.insert(ext.name.clone(), body_val.clone());
                    }
                }
                ExtractorMode::Regex => {
                    if let Some(regex_str) = &ext.regex {
                        if let Ok(re) = regex::Regex::new(regex_str) {
                            if let Some(caps) = re.captures(body_text) {
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
                ExtractorMode::Unsupported => {
                    warn!(
                        extractor = %ext.name,
                        "Skipping RestApi extractor with an unsupported or ambiguous configuration"
                    );
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExtractorMode {
    Json,
    Regex,
    Unsupported,
}

fn extractor_mode(extractor: &RestApiResponseExtractor) -> ExtractorMode {
    match extractor.format.as_deref() {
        Some("json") => ExtractorMode::Json,
        Some("regex") if extractor.regex.is_some() => ExtractorMode::Regex,
        Some("regex") => ExtractorMode::Unsupported,
        Some(_) => ExtractorMode::Unsupported,
        None => match (extractor.json_pointer.is_some(), extractor.regex.is_some()) {
            (true, false) => ExtractorMode::Json,
            (false, true) => ExtractorMode::Regex,
            _ => ExtractorMode::Unsupported,
        },
    }
}

fn normalize_json_pointer(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path.replace(".", "/"))
    }
}

fn parse_response(
    status_is_success: bool,
    body_bytes: &[u8],
    spec: &RestApiSpec,
    run_id: RunId,
    step_id: StepInstanceId,
    fencing_token: i64,
) -> Result<StepCompletedEvent> {
    if status_is_success {
        let body_val: Value = serde_json::from_slice(body_bytes)
            .unwrap_or_else(|_| serde_json::json!({ "text": String::from_utf8_lossy(body_bytes) }));
        let body_text = String::from_utf8_lossy(body_bytes).to_string();

        let mut final_outputs = serde_json::Map::new();
        final_outputs.insert("response".to_string(), body_val.clone());

        apply_extractors(spec, &body_val, &body_text, &mut final_outputs);

        Ok(StepCompletedEvent {
            run_id,
            step_id,
            fencing_token,
            event_type: EventType::Step(StepEventType::Completed),
            test_reports: None,
            artifacts: None,
            storage_hashes: None,
            outputs: Some(final_outputs.into_iter().collect()),
            timestamp: chrono::Utc::now(),
            exit_code: Some(0),
            runner_id: Some("engine-intrinsic".to_string()),
        })
    } else {
        let error_body = String::from_utf8_lossy(body_bytes).to_string();
        anyhow::bail!("RestApi failed with error body: {}", error_body);
    }
}

async fn execute_request(
    run_id: RunId,
    step_id: StepInstanceId,
    fencing_token: i64,
    spec: &RestApiSpec,
    rendered_body: Option<String>,
    _pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    let client = reqwest::Client::new();
    let method = build_request_method(spec);

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
    let body_bytes = res.bytes().await?;

    match parse_response(
        status.is_success(),
        &body_bytes,
        spec,
        run_id,
        step_id,
        fencing_token,
    ) {
        Ok(event) => {
            let js = async_nats::jetstream::new(nats_client);
            let event_payload = serde_json::to_value(event)
                .context("failed to serialize RestApi completion event")?;

            stormchaser_model::nats::publish_cloudevent(
                &js,
                NatsSubject::StepCompleted,
                EventType::Step(StepEventType::Completed),
                EventSource::System,
                event_payload,
                Some(SchemaVersion::new("1.0".to_string())),
                None,
            )
            .await
            .context("failed to publish RestApi completion event")?;
            Ok(())
        }
        Err(e) => {
            anyhow::bail!("RestApi failed with status {}: {}", status, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn test_render_request_body() {
        let spec = RestApiSpec {
            connection: None,
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

    #[test]
    fn test_build_request_method() {
        let mut spec = RestApiSpec {
            connection: None,
            url: "http://example.com".to_string(),
            method: None,
            headers: None,
            body: None,
            timeout: None,
            extractors: None,
        };
        assert_eq!(build_request_method(&spec), reqwest::Method::GET);

        spec.method = Some("post".to_string());
        assert_eq!(build_request_method(&spec), reqwest::Method::POST);

        spec.method = Some("PUT".to_string());
        assert_eq!(build_request_method(&spec), reqwest::Method::PUT);

        spec.method = Some("DELETE".to_string());
        assert_eq!(build_request_method(&spec), reqwest::Method::DELETE);

        spec.method = Some("PATCH".to_string());
        assert_eq!(build_request_method(&spec), reqwest::Method::PATCH);

        spec.method = Some("UNKNOWN".to_string());
        assert_eq!(build_request_method(&spec), reqwest::Method::GET);
    }

    #[test]
    fn test_apply_extractors_json() {
        let spec = RestApiSpec {
            connection: None,
            url: "http://example.com".to_string(),
            method: None,
            headers: None,
            body: None,
            timeout: None,
            extractors: Some(vec![
                RestApiResponseExtractor {
                    name: "full_body".to_string(),
                    format: Some("json".to_string()),
                    json_pointer: None,
                    regex: None,
                    group: None,
                    sensitive: None,
                },
                RestApiResponseExtractor {
                    name: "nested_value".to_string(),
                    format: Some("json".to_string()),
                    json_pointer: Some("data.items.0.id".to_string()),
                    regex: None,
                    group: None,
                    sensitive: None,
                },
                RestApiResponseExtractor {
                    name: "pointer_value".to_string(),
                    format: Some("json".to_string()),
                    json_pointer: Some("/data/items/1/id".to_string()),
                    regex: None,
                    group: None,
                    sensitive: None,
                },
            ]),
        };

        let body_val = json!({
            "data": {
                "items": [
                    { "id": "123" },
                    { "id": "456" }
                ]
            }
        });
        let body_text = body_val.to_string();

        let mut final_outputs = serde_json::Map::new();
        apply_extractors(&spec, &body_val, &body_text, &mut final_outputs);

        assert_eq!(final_outputs.get("full_body").unwrap(), &body_val);
        assert_eq!(final_outputs.get("nested_value").unwrap(), &json!("123"));
        assert_eq!(final_outputs.get("pointer_value").unwrap(), &json!("456"));
    }

    #[test]
    fn test_apply_extractors_regex() {
        let spec = RestApiSpec {
            connection: None,
            url: "http://example.com".to_string(),
            method: None,
            headers: None,
            body: None,
            timeout: None,
            extractors: Some(vec![
                RestApiResponseExtractor {
                    name: "token".to_string(),
                    format: Some("regex".to_string()),
                    json_pointer: None,
                    regex: Some(r"Token is ([A-Z0-9]+)".to_string()),
                    group: Some(1),
                    sensitive: None,
                },
                RestApiResponseExtractor {
                    name: "no_group".to_string(),
                    format: Some("regex".to_string()),
                    json_pointer: None,
                    regex: Some(r"Status: \d+".to_string()),
                    group: Some(0),
                    sensitive: None,
                },
            ]),
        };

        let body_val = json!({});
        let body_text = "Welcome. Token is ABC123DEF. Status: 200 OK.";

        let mut final_outputs = serde_json::Map::new();
        apply_extractors(&spec, &body_val, body_text, &mut final_outputs);

        assert_eq!(final_outputs.get("token").unwrap(), &json!("ABC123DEF"));
        assert_eq!(
            final_outputs.get("no_group").unwrap(),
            &json!("Status: 200")
        );
    }

    #[test]
    fn test_extractor_mode_defaults_json_pointer_extractors_to_json() {
        let extractor = RestApiResponseExtractor {
            name: "token".to_string(),
            format: None,
            json_pointer: Some("/data/token".to_string()),
            regex: None,
            group: None,
            sensitive: Some(true),
        };

        assert_eq!(extractor_mode(&extractor), ExtractorMode::Json);
    }

    #[test]
    fn test_extractor_mode_rejects_ambiguous_extractors() {
        let extractor = RestApiResponseExtractor {
            name: "token".to_string(),
            format: None,
            json_pointer: Some("/data/token".to_string()),
            regex: Some("Token is ([A-Z0-9]+)".to_string()),
            group: Some(1),
            sensitive: None,
        };

        assert_eq!(extractor_mode(&extractor), ExtractorMode::Unsupported);
    }

    #[test]
    fn test_prepare_template_context() {
        let run_id = RunId::new(Uuid::new_v4());
        let inputs = json!({"foo": "bar"});
        let outputs = json!({"step1": {"result": 42}});
        let ctx = prepare_template_context(inputs.clone(), outputs.clone(), run_id);

        assert_eq!(ctx["inputs"], inputs);
        assert_eq!(ctx["steps"], outputs);
        assert_eq!(ctx["run"]["id"], json!(run_id.to_string()));
    }

    #[test]
    fn test_parse_response_success() {
        use stormchaser_model::StepInstanceId;
        let run_id = RunId::new(Uuid::new_v4());
        let step_id = StepInstanceId::new(Uuid::new_v4());
        let spec = RestApiSpec {
            connection: None,
            url: "http://example.com".to_string(),
            method: None,
            headers: None,
            body: None,
            timeout: None,
            extractors: None,
        };

        let body = b"{\"hello\":\"world\"}";
        let result = parse_response(true, body, &spec, run_id, step_id, 1).unwrap();

        assert_eq!(result.exit_code, Some(0));
        let outputs = result.outputs.unwrap();
        assert_eq!(outputs.get("response").unwrap(), &json!({"hello": "world"}));
    }

    #[test]
    fn test_parse_response_failure() {
        use stormchaser_model::StepInstanceId;
        let run_id = RunId::new(Uuid::new_v4());
        let step_id = StepInstanceId::new(Uuid::new_v4());
        let spec = RestApiSpec {
            connection: None,
            url: "http://example.com".to_string(),
            method: None,
            headers: None,
            body: None,
            timeout: None,
            extractors: None,
        };

        let body = b"Internal Server Error";
        let err = parse_response(false, body, &spec, run_id, step_id, 1).unwrap_err();

        assert!(err
            .to_string()
            .contains("RestApi failed with error body: Internal Server Error"));
    }
}
