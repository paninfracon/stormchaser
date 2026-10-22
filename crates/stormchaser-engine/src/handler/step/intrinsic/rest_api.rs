use crate::handler::integrations::utils::report_step_completed;
use crate::handler::{fetch_outputs, fetch_run_context};
use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use std::time::Duration;
use stormchaser_model::dsl::{RestApiResponseExtractor, RestApiSpec};
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use tracing::{info, warn};

fn prepare_template_context(inputs: Value, outputs: Value, run_id: RunId) -> Value {
    serde_json::json!({
        "inputs": inputs,
        "steps": outputs,
        "run": {
            "id": run_id.to_string(),
        }
    })
}

/// Runs a RestApi step, reporting its result.
pub(crate) async fn handle_rest_api_invoke(
    run_id: RunId,
    step_id: StepInstanceId,
    fencing_token: i64,
    spec: Value,
    pool: PgPool,
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
            crate::authz::ensure_run_may_use(&pool, run_id, &conn).await?;
            if conn.connection_type == stormchaser_model::connections::ConnectionType::HttpApi {
                apply_http_connection(&mut spec, &conn)?;
            } else {
                warn!("Connection {} is not of type HttpApi", conn_name);
            }
        } else {
            anyhow::bail!("Connection {} not found", conn_name);
        }
    }

    info!("Invoking REST API {} for run {}", spec.url, run_id);

    // 1. Mark as Running
    crate::handler::integrations::utils::start_built_in(&pool, step_id, "rest_api").await?;

    // 2. Prepare Context for Template Rendering
    let run_context = fetch_run_context(run_id, &pool).await?;
    let outputs = fetch_outputs(run_id, &pool).await?;

    let template_ctx = prepare_template_context(run_context.inputs, outputs, run_id);

    // 3. Render Body if present
    let rendered_body = render_request_body(&spec, &template_ctx)?;

    // 4. Build and Execute Request
    execute_request(run_id, step_id, fencing_token, &spec, rendered_body, pool).await
}

/// Resolves the step's URL against an HttpApi connection and attaches the connection's
/// headers and token.
///
/// The URL is the workflow author's; the headers and token are the connection owner's. They
/// are only attached when the request goes to the connection's own origin, so a step cannot
/// name a connection and have its credentials sent to a server of the author's choosing.
fn apply_http_connection(
    spec: &mut RestApiSpec,
    conn: &stormchaser_model::Connection,
) -> Result<()> {
    let base_url = conn
        .config
        .get("base_url")
        .and_then(|v| v.as_str())
        // Keep legacy `url` support for existing HttpApi connections.
        .or_else(|| conn.config.get("url").and_then(|v| v.as_str()));
    let headers = conn.config.get("headers").and_then(|v| v.as_object());
    let carries_credentials = conn.credentials.is_some() || headers.is_some_and(|h| !h.is_empty());

    let lowered = spec.url.to_ascii_lowercase();
    let is_absolute = lowered.starts_with("http://") || lowered.starts_with("https://");

    match base_url {
        Some(base_url) => {
            let base = reqwest::Url::parse(base_url)
                .with_context(|| format!("Connection {} has an invalid base_url", conn.name))?;
            let target = if is_absolute {
                spec.url.clone()
            } else {
                format!(
                    "{}/{}",
                    base_url.trim_end_matches('/'),
                    spec.url.trim_start_matches('/')
                )
            };
            let parsed = reqwest::Url::parse(&target)
                .with_context(|| format!("RestApi URL {} is not valid", target))?;
            if carries_credentials && parsed.origin() != base.origin() {
                anyhow::bail!(
                    "RestApi URL {} is outside connection {}: its credentials are only sent to {}",
                    spec.url,
                    conn.name,
                    base.origin().ascii_serialization()
                );
            }
            spec.url = target;
        }
        None if carries_credentials => {
            anyhow::bail!(
                "Connection {} has credentials but no base_url, so they cannot be tied to a server; set base_url on the connection",
                conn.name
            );
        }
        None => {}
    }

    let mut current_headers = spec.headers.take().unwrap_or_default();
    if let Some(headers) = headers {
        for (k, v) in headers {
            if let Some(s) = v.as_str() {
                current_headers.insert(k.clone(), s.to_string());
            }
        }
    }
    if let Some(auth_token) = &conn.credentials {
        if !current_headers.contains_key("Authorization")
            && !current_headers.contains_key("authorization")
        {
            current_headers.insert(
                "Authorization".to_string(),
                format!("Bearer {}", auth_token),
            );
        }
    }
    if !current_headers.is_empty() || headers.is_some() || conn.credentials.is_some() {
        spec.headers = Some(current_headers);
    }
    Ok(())
}

fn render_request_body(spec: &RestApiSpec, template_ctx: &Value) -> Result<Option<String>> {
    if let Some(body_tmpl) = &spec.body {
        Ok(Some(
            crate::templating::render_str(body_tmpl, template_ctx)
                .map_err(|e| anyhow::anyhow!("Failed to render request body: {:?}", e))?,
        ))
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

/// The step's outputs from a successful response
fn parse_response(
    status_is_success: bool,
    body_bytes: &[u8],
    spec: &RestApiSpec,
) -> Result<HashMap<String, Value>> {
    if status_is_success {
        let body_val: Value = serde_json::from_slice(body_bytes)
            .unwrap_or_else(|_| serde_json::json!({ "text": String::from_utf8_lossy(body_bytes) }));
        let body_text = String::from_utf8_lossy(body_bytes).to_string();

        let mut final_outputs = serde_json::Map::new();
        final_outputs.insert("response".to_string(), body_val.clone());

        apply_extractors(spec, &body_val, &body_text, &mut final_outputs);

        Ok(final_outputs.into_iter().collect())
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
    pool: PgPool,
) -> Result<()> {
    let (client, url) = stormchaser_core::net_guard::guarded_client(&spec.url).await?;
    let method = build_request_method(spec);

    let mut builder = client.request(method, url);

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
    let body_bytes = stormchaser_core::net_guard::read_body_capped(
        res,
        stormchaser_core::net_guard::MAX_RESPONSE_BYTES,
    )
    .await?;

    let outputs = parse_response(status.is_success(), &body_bytes, spec)
        .map_err(|e| anyhow::anyhow!("RestApi failed with status {}: {}", status, e))?;
    report_step_completed(&pool, run_id, step_id, fencing_token, Some(outputs)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    fn http_connection(config: Value, token: Option<&str>) -> stormchaser_model::Connection {
        serde_json::from_value(json!({
            "id": Uuid::new_v4(),
            "name": "billing",
            "description": null,
            "connection_type": "http_api",
            "config": config,
            "credentials": token,
            "aws_assume_role_arn": null,
            "is_default_sfs": false,
            "ca_cert": null,
            "client_cert": null,
            "client_key": null,
            "created_at": chrono::Utc::now(),
            "updated_at": chrono::Utc::now(),
        }))
        .unwrap()
    }

    fn spec_for(url: &str) -> RestApiSpec {
        RestApiSpec {
            connection: Some("billing".to_string()),
            url: url.to_string(),
            method: None,
            headers: None,
            body: None,
            timeout: None,
            extractors: None,
        }
    }

    #[test]
    fn connection_joins_relative_url_and_attaches_token() {
        let conn = http_connection(
            json!({"base_url": "https://api.example.com/v1/"}),
            Some("t0k"),
        );
        let mut spec = spec_for("/invoices");
        apply_http_connection(&mut spec, &conn).unwrap();
        assert_eq!(spec.url, "https://api.example.com/v1/invoices");
        assert_eq!(spec.headers.unwrap()["Authorization"], "Bearer t0k");
    }

    #[test]
    fn connection_allows_absolute_url_on_its_own_origin() {
        let conn = http_connection(
            json!({"base_url": "https://api.example.com/v1"}),
            Some("t0k"),
        );
        let mut spec = spec_for("https://api.example.com:443/v2/invoices");
        apply_http_connection(&mut spec, &conn).unwrap();
        assert!(spec.headers.unwrap().contains_key("Authorization"));
    }

    #[test]
    fn connection_credentials_are_not_sent_to_another_origin() {
        let conn = http_connection(json!({"base_url": "https://api.example.com"}), Some("t0k"));
        for url in [
            "https://attacker.example.net/collect",
            "http://api.example.com/invoices",
            "https://api.example.com:8443/invoices",
            "https://api.example.com@attacker.example.net/",
            "HTTPS://attacker.example.net/",
        ] {
            let mut spec = spec_for(url);
            let err = apply_http_connection(&mut spec, &conn).unwrap_err();
            assert!(
                err.to_string().contains("outside connection"),
                "{url}: {err}"
            );
            assert!(spec.headers.is_none(), "{url}");
        }
    }

    #[test]
    fn connection_headers_alone_are_bound_to_the_origin() {
        let conn = http_connection(
            json!({"base_url": "https://api.example.com", "headers": {"X-Api-Key": "k"}}),
            None,
        );
        let mut spec = spec_for("https://attacker.example.net/");
        assert!(apply_http_connection(&mut spec, &conn).is_err());
    }

    #[test]
    fn connection_with_credentials_needs_a_base_url() {
        let conn = http_connection(json!({}), Some("t0k"));
        let mut spec = spec_for("https://api.example.com/invoices");
        let err = apply_http_connection(&mut spec, &conn).unwrap_err();
        assert!(err.to_string().contains("no base_url"));
    }

    #[test]
    fn connection_without_credentials_does_not_restrict_the_url() {
        let conn = http_connection(json!({"base_url": "https://api.example.com"}), None);
        let mut spec = spec_for("https://other.example.net/status");
        apply_http_connection(&mut spec, &conn).unwrap();
        assert_eq!(spec.url, "https://other.example.net/status");
        assert!(spec.headers.is_none());
    }

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
        let outputs = parse_response(true, body, &spec).unwrap();

        assert_eq!(outputs.get("response").unwrap(), &json!({"hello": "world"}));
    }

    #[test]
    fn test_parse_response_failure() {
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
        let err = parse_response(false, body, &spec).unwrap_err();

        assert!(err
            .to_string()
            .contains("RestApi failed with error body: Internal Server Error"));
    }
}
