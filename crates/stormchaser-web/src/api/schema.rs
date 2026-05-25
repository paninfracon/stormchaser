#[allow(unused_imports)]
#[cfg(feature = "ssr")]
use super::require_auth;
#[allow(unused_imports)]
use super::*;
#[allow(unused_imports)]
use crate::models::*;
#[allow(unused_imports)]
use leptos::prelude::*;
#[allow(unused_imports)]
use leptos::server_fn::codec::Json;

#[server(input = Json, output = Json)]
pub async fn hydrate_schema_complete(
    schema: serde_json::Value,
    inputs: serde_json::Value,
    queries: Option<serde_json::Value>,
) -> Result<(serde_json::Value, String, Vec<String>), ServerFnError> {
    let cookie = require_auth().await?;
    let query_url =
        std::env::var("QUERY_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".to_string());

    let client = super::http_client();
    let payload = if let Some(q) = queries {
        serde_json::json!({ "schema": schema, "inputs": inputs, "queries": q })
    } else {
        serde_json::json!({ "schema": schema, "inputs": inputs })
    };

    let res = client
        .post(format!("{}/api/v1/schema/hydrate", query_url))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .json(&payload)
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        use futures::stream::StreamExt;
        let mut byte_stream = res.bytes_stream();
        let mut buffer = String::new();

        let mut final_schema = schema.clone();
        let mut final_status = "Update pending".to_string();
        let mut final_errors = vec![];

        while let Some(item) = byte_stream.next().await {
            if let Ok(chunk) = item {
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(idx) = buffer.find("\n\n") {
                    let event_str = buffer[..idx].to_string();
                    buffer = buffer[idx + 2..].to_string();

                    if let Some(data_idx) = event_str.find("data: ") {
                        let json_str = &event_str[data_idx + 6..];
                        if let Ok(event) = serde_json::from_str::<serde_json::Value>(json_str) {
                            if let Some(status) = event.get("status").and_then(|s| s.as_str()) {
                                final_status = status.to_string();
                                if let Some(hydrated) = event.get("hydrated_schema") {
                                    final_schema = hydrated.clone();
                                }
                                if let Some(errors) =
                                    event.get("validation_errors").and_then(|e| e.as_array())
                                {
                                    final_errors = errors
                                        .iter()
                                        .filter_map(|e| e.as_str().map(|s| s.to_string()))
                                        .collect();
                                }

                                if status == "Completed"
                                    || status == "Incomplete Input"
                                    || status == "Schema validation failed"
                                {
                                    return Ok((final_schema, final_status, final_errors));
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok((final_schema, final_status, final_errors))
    } else {
        Err(ServerFnError::new(format!(
            "Hydration failed: {}",
            res.status()
        )))
    }
}
