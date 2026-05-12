use crate::logging::LogBackend;
use anyhow::Result;
use chrono::Utc;
use serde_json::Value;
use tokio::sync::mpsc;

pub(crate) async fn stream_loki_logs(
    loki_url: &str,
    job_name: &str,
) -> Result<mpsc::Receiver<Result<String>>> {
    use futures::StreamExt;

    let query = format!("{{job_name=\"{}\"}}", job_name);

    let base_url = loki_url.trim_end_matches('/');
    let ws_url = if base_url.starts_with("http://") {
        base_url.replace("http://", "ws://")
    } else if base_url.starts_with("https://") {
        base_url.replace("https://", "wss://")
    } else {
        base_url.to_string()
    };

    let ws_url = format!(
        "{}/loki/api/v1/tail?query={}",
        ws_url,
        urlencoding::encode(&query)
    );

    tracing::debug!("Connecting to Loki WebSocket: {}", ws_url);
    let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url).await?;
    tracing::debug!("Connected to Loki WebSocket");
    let (tx, rx) = mpsc::channel(100);

    tokio::spawn(async move {
        let mut read_stream = ws_stream;
        while let Some(msg) = read_stream.next().await {
            match msg {
                Ok(tungstenite::Message::Text(text)) => {
                    tracing::trace!("Received Loki WebSocket text: {}", text);
                    if let Ok(data) = serde_json::from_str::<Value>(&text) {
                        if let Some(streams) = data.get("streams").and_then(|s| s.as_array()) {
                            for stream in streams {
                                if let Some(values) =
                                    stream.get("values").and_then(|v| v.as_array())
                                {
                                    for entry in values {
                                        if let Some(log_line) =
                                            entry.get(1).and_then(|v| v.as_str())
                                        {
                                            if tx.send(Ok(log_line.to_string())).await.is_err() {
                                                return; // Receiver dropped
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(tungstenite::Message::Close(_)) => {
                    break;
                }
                Err(e) => {
                    let _ = tx
                        .send(Err(anyhow::anyhow!("WebSocket error: {}", e)))
                        .await;
                    break;
                }
                _ => {}
            }
        }
    });

    Ok(rx)
}

pub(crate) async fn fetch_loki_logs(
    backend: &LogBackend,
    loki_url: &str,
    job_name: &str,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    finished_at: Option<chrono::DateTime<chrono::Utc>>,
    limit: Option<usize>,
) -> Result<Vec<String>> {
    let query = format!("{{job_name=\"{}\"}}", job_name);
    let url = format!("{}/loki/api/v1/query_range", loki_url.trim_end_matches('/'));

    let client = backend.create_client();
    let forward = "forward".to_string();

    let mut start_time = started_at
        .map(|t| t - chrono::Duration::minutes(1))
        .unwrap_or_else(|| Utc::now() - chrono::Duration::days(30))
        .timestamp_nanos_opt()
        .unwrap_or(0);

    let end_time = finished_at
        .unwrap_or_else(Utc::now)
        // Add a small buffer to end time to ensure we get the final logs
        .checked_add_signed(chrono::Duration::minutes(5))
        .unwrap_or_else(Utc::now)
        .timestamp_nanos_opt()
        .unwrap_or(0)
        .to_string();

    let mut logs = Vec::new();
    let target_limit = limit.unwrap_or(usize::MAX);
    let mut seen = std::collections::HashSet::new(); // to prevent duplicate lines if timestamps collide

    loop {
        let req_limit = std::cmp::min(target_limit.saturating_sub(logs.len()), 5000).to_string();
        let start_time_str = start_time.to_string();

        let resp = client
            .get(&url)
            .query(&[
                ("query", &query),
                ("direction", &forward),
                ("limit", &req_limit),
                ("start", &start_time_str),
                ("end", &end_time),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow::anyhow!("Loki returned status {}", resp.status()));
        }

        let data: serde_json::Value = resp.json().await?;
        let mut fetched_count = 0;
        let mut highest_timestamp = start_time;
        let mut new_logs_added = false;

        if let Some(streams) = data["data"]["result"].as_array() {
            for stream in streams {
                if let Some(values) = stream["values"].as_array() {
                    for entry in values {
                        fetched_count += 1;

                        // Track highest timestamp for pagination
                        if let Some(ts_str) = entry.get(0).and_then(|v| v.as_str()) {
                            if let Ok(ts) = ts_str.parse::<i64>() {
                                if ts > highest_timestamp {
                                    highest_timestamp = ts;
                                }
                            }
                        }

                        if let Some(log_line) = entry.get(1).and_then(|v| v.as_str()) {
                            // Prevent duplicates that might occur on exact page boundaries
                            let entry_key = (
                                entry
                                    .get(0)
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                log_line.to_string(),
                            );
                            if seen.insert(entry_key) {
                                new_logs_added = true;
                                for line in log_line.lines() {
                                    logs.push(line.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        if fetched_count == 0
            || logs.len() >= target_limit
            || (fetched_count > 0 && !new_logs_added)
        {
            break;
        }

        // Advance start_time to highest_timestamp. The `seen` set prevents duplicate processing.
        start_time = highest_timestamp;
    }

    if let Some(l) = limit {
        logs.truncate(l);
    }

    Ok(logs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::LogBackend;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_fetch_loki_logs_success() {
        let mock_server = MockServer::start().await;

        let loki_response = serde_json::json!({
            "status": "success",
            "data": {
                "resultType": "streams",
                "result": [
                    {
                        "stream": {
                            "job_name": "storm-test-step-12345678"
                        },
                        "values": [
                            [ "1610000000000000000", "log line 1" ],
                            [ "1610000001000000000", "log line 2" ]
                        ]
                    }
                ]
            }
        });

        Mock::given(method("GET"))
            .and(path("/loki/api/v1/query_range"))
            .respond_with(ResponseTemplate::new(200).set_body_json(loki_response))
            .mount(&mock_server)
            .await;

        let backend = LogBackend::Loki {
            url: mock_server.uri(),
        };

        let logs = fetch_loki_logs(
            &backend,
            &mock_server.uri(),
            "storm-test-step-12345678",
            None,
            None,
            Some(2),
        )
        .await
        .expect("Failed to fetch loki logs");

        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0], "log line 1");
        assert_eq!(logs[1], "log line 2");
    }

    #[tokio::test]
    async fn test_fetch_loki_logs_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/loki/api/v1/query_range"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let backend = LogBackend::Loki {
            url: mock_server.uri(),
        };

        let result = fetch_loki_logs(
            &backend,
            &mock_server.uri(),
            "storm-test-step-12345678",
            None,
            None,
            Some(2),
        )
        .await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Loki returned status"));
    }

    #[tokio::test]
    async fn test_stream_step_logs_loki_connection_refused() {
        let result = stream_loki_logs("http://127.0.0.1:1", "storm-test-step-12345678").await;
        // The connection should fail
        assert!(result.is_err());
    }
}
