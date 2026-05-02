use anyhow::Result;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Represents the supported logging backends for step execution logs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogBackend {
    /// Grafana Loki backend.
    Loki {
        /// The URL of the Loki server.
        url: String,
    },
    /// Elasticsearch backend.
    Elasticsearch {
        /// The URL of the Elasticsearch server.
        url: String,
        /// The Elasticsearch index to query.
        index: String,
    },
}

impl LogBackend {
    fn create_client(&self) -> ClientWithMiddleware {
        let retry_policy = ExponentialBackoff::builder().build_with_max_retries(5);
        ClientBuilder::new(reqwest::Client::new())
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build()
    }

    /// Fetches historical logs for a specific step instance.
    pub async fn fetch_step_logs(
        &self,
        step_name: &str,
        step_id: Uuid,
        started_at: Option<chrono::DateTime<chrono::Utc>>,
        finished_at: Option<chrono::DateTime<chrono::Utc>>,
        limit: Option<usize>,
    ) -> Result<Vec<String>> {
        let job_name = format!(
            "storm-{}-{}",
            step_name.to_lowercase().replace('_', "-"),
            &step_id.to_string()[..8]
        );

        match self {
            LogBackend::Loki { url } => {
                self.fetch_loki_logs(url, &job_name, started_at, finished_at, limit)
                    .await
            }
            LogBackend::Elasticsearch { url, index } => {
                self.fetch_elasticsearch_logs(url, index, &job_name, started_at, finished_at, limit)
                    .await
            }
        }
    }

    /// Opens a real-time stream of log lines for a currently executing step instance.
    pub async fn stream_step_logs(
        &self,
        step_name: &str,
        step_id: Uuid,
    ) -> Result<mpsc::Receiver<Result<String>>> {
        let job_name = format!(
            "storm-{}-{}",
            step_name.to_lowercase().replace('_', "-"),
            &step_id.to_string()[..8]
        );

        match self {
            LogBackend::Loki { url } => self.stream_loki_logs(url, &job_name).await,
            LogBackend::Elasticsearch { .. } => {
                anyhow::bail!(
                    "Log streaming is not currently supported for Elasticsearch backends"
                );
            }
        }
    }

    async fn stream_loki_logs(
        &self,
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
                                                if tx.send(Ok(log_line.to_string())).await.is_err()
                                                {
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

    async fn fetch_loki_logs(
        &self,
        loki_url: &str,
        job_name: &str,
        started_at: Option<chrono::DateTime<chrono::Utc>>,
        finished_at: Option<chrono::DateTime<chrono::Utc>>,
        limit: Option<usize>,
    ) -> Result<Vec<String>> {
        let query = format!("{{job_name=\"{}\"}}", job_name);
        let url = format!("{}/loki/api/v1/query_range", loki_url.trim_end_matches('/'));

        let client = self.create_client();
        let forward = "forward".to_string();

        let mut start_time = started_at
            .map(|t| t - chrono::Duration::minutes(1))
            .unwrap_or_else(|| chrono::Utc::now() - chrono::Duration::days(30))
            .timestamp_nanos_opt()
            .unwrap_or(0);

        let end_time = finished_at
            .unwrap_or_else(chrono::Utc::now)
            // Add a small buffer to end time to ensure we get the final logs
            .checked_add_signed(chrono::Duration::minutes(5))
            .unwrap_or_else(chrono::Utc::now)
            .timestamp_nanos_opt()
            .unwrap_or(0)
            .to_string();

        let mut logs = Vec::new();
        let target_limit = limit.unwrap_or(usize::MAX);
        let mut seen = std::collections::HashSet::new(); // to prevent duplicate lines if timestamps collide

        loop {
            let req_limit =
                std::cmp::min(target_limit.saturating_sub(logs.len()), 5000).to_string();
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

    async fn fetch_elasticsearch_logs(
        &self,
        es_url: &str,
        index: &str,
        job_name: &str,
        started_at: Option<chrono::DateTime<chrono::Utc>>,
        finished_at: Option<chrono::DateTime<chrono::Utc>>,
        limit: Option<usize>,
    ) -> Result<Vec<String>> {
        let url = format!("{}/{}/_search", es_url.trim_end_matches('/'), index);

        let gte = started_at
            .map(|t| t - chrono::Duration::minutes(1))
            .unwrap_or_else(|| chrono::Utc::now() - chrono::Duration::days(30))
            .to_rfc3339();

        let lte = finished_at
            .unwrap_or_else(chrono::Utc::now)
            .checked_add_signed(chrono::Duration::seconds(5))
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339();

        let mut logs = Vec::new();
        let target_limit = limit.unwrap_or(usize::MAX);
        let mut search_after: Option<serde_json::Value> = None;

        loop {
            let req_size = std::cmp::min(target_limit.saturating_sub(logs.len()), 5000);

            let mut query = serde_json::json!({
                "query": {
                    "bool": {
                        "must": [
                            { "term": { "job_name.keyword": job_name } }
                        ],
                        "filter": [
                            {
                                "range": {
                                    "@timestamp": {
                                        "gte": gte,
                                        "lte": lte
                                    }
                                }
                            }
                        ]
                    }
                },
                "sort": [
                    { "@timestamp": "asc" },
                    { "_id": "asc" } // Add tiebreaker
                ],
                "size": req_size
            });

            if let Some(sa) = &search_after {
                query
                    .as_object_mut()
                    .unwrap()
                    .insert("search_after".to_string(), sa.clone());
            }

            let client = self.create_client();
            let resp = client.post(&url).json(&query).send().await?;

            if !resp.status().is_success() {
                return Err(anyhow::anyhow!(
                    "Elasticsearch returned status {}",
                    resp.status()
                ));
            }

            let data: serde_json::Value = resp.json().await?;
            let mut fetched_count = 0;
            let mut last_sort_value = None;
            let mut new_logs_added = false;

            if let Some(hits) = data["hits"]["hits"].as_array() {
                for hit in hits {
                    fetched_count += 1;
                    if let Some(message) = hit["_source"]["message"].as_str() {
                        new_logs_added = true;
                        for line in message.lines() {
                            logs.push(line.to_string());
                        }
                    }
                    if let Some(sort) = hit.get("sort") {
                        last_sort_value = Some(sort.clone());
                    }
                }
            }

            if fetched_count == 0
                || logs.len() >= target_limit
                || last_sort_value.is_none()
                || (fetched_count > 0 && !new_logs_added)
            {
                break;
            }

            search_after = last_sort_value;
        }

        if let Some(l) = limit {
            logs.truncate(l);
        }

        Ok(logs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
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
        let step_id = Uuid::new_v4();

        let logs = backend
            .fetch_step_logs("test-step", step_id, None, None, Some(2))
            .await
            .expect("Failed to fetch loki logs");

        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0], "log line 1");
        assert_eq!(logs[1], "log line 2");
    }

    #[tokio::test]
    async fn test_fetch_elasticsearch_logs_success() {
        let mock_server = MockServer::start().await;

        let es_response = serde_json::json!({
            "took": 1,
            "timed_out": false,
            "_shards": {
                "total": 1,
                "successful": 1,
                "skipped": 0,
                "failed": 0
            },
            "hits": {
                "total": {
                    "value": 2,
                    "relation": "eq"
                },
                "max_score": null,
                "hits": [
                    {
                        "_index": "logs",
                        "_id": "1",
                        "_score": null,
                        "_source": {
                            "message": "es log line 1"
                        },
                        "sort": [1610000000000_u64]
                    },
                    {
                        "_index": "logs",
                        "_id": "2",
                        "_score": null,
                        "_source": {
                            "message": "es log line 2"
                        },
                        "sort": [1610000001000_u64]
                    }
                ]
            }
        });

        Mock::given(method("POST"))
            .and(path("/my-index/_search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(es_response))
            .mount(&mock_server)
            .await;

        let backend = LogBackend::Elasticsearch {
            url: mock_server.uri(),
            index: "my-index".to_string(),
        };
        let step_id = Uuid::new_v4();

        let logs = backend
            .fetch_step_logs("test-step", step_id, None, None, Some(2))
            .await
            .expect("Failed to fetch es logs");

        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0], "es log line 1");
        assert_eq!(logs[1], "es log line 2");
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
        let step_id = Uuid::new_v4();

        let result = backend
            .fetch_step_logs("test-step", step_id, None, None, Some(2))
            .await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Loki returned status"));
    }

    #[tokio::test]
    async fn test_fetch_elasticsearch_logs_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/my-index/_search"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let backend = LogBackend::Elasticsearch {
            url: mock_server.uri(),
            index: "my-index".to_string(),
        };
        let step_id = Uuid::new_v4();

        let result = backend
            .fetch_step_logs("test-step", step_id, None, None, Some(2))
            .await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Elasticsearch returned status"));
    }

    #[tokio::test]
    async fn test_stream_step_logs_elasticsearch_unsupported() {
        let backend = LogBackend::Elasticsearch {
            url: "http://localhost:9200".to_string(),
            index: "my-index".to_string(),
        };
        let step_id = Uuid::new_v4();

        let result = backend.stream_step_logs("test-step", step_id).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not currently supported"));
    }

    #[tokio::test]
    async fn test_stream_step_logs_loki_connection_refused() {
        // Test that it correctly attempts to connect to the websocket and fails gracefully
        // if the server is not listening (connection refused).
        let backend = LogBackend::Loki {
            url: "http://127.0.0.1:1".to_string(),
        };
        let step_id = Uuid::new_v4();

        let result = backend.stream_step_logs("test-step", step_id).await;
        // The connection should fail
        assert!(result.is_err());
    }
}
