use anyhow::Result;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogBackend {
    Loki { url: String },
    Elasticsearch { url: String, index: String },
}

impl LogBackend {
    fn create_client(&self) -> ClientWithMiddleware {
        let retry_policy = ExponentialBackoff::builder().build_with_max_retries(5);
        ClientBuilder::new(reqwest::Client::new())
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build()
    }

    pub async fn fetch_step_logs(
        &self,
        step_name: &str,
        step_id: uuid::Uuid,
        started_at: Option<chrono::DateTime<chrono::Utc>>,
        finished_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<String>> {
        let job_name = format!(
            "storm-{}-{}",
            step_name.to_lowercase().replace('_', "-"),
            &step_id.to_string()[..8]
        );

        match self {
            LogBackend::Loki { url } => {
                self.fetch_loki_logs(url, &job_name, started_at, finished_at)
                    .await
            }
            LogBackend::Elasticsearch { url, index } => {
                self.fetch_elasticsearch_logs(url, index, &job_name, started_at, finished_at)
                    .await
            }
        }
    }

    pub async fn stream_step_logs(
        &self,
        step_name: &str,
        step_id: uuid::Uuid,
    ) -> Result<tokio::sync::mpsc::Receiver<Result<String>>> {
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
    ) -> Result<tokio::sync::mpsc::Receiver<Result<String>>> {
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
        let (tx, rx) = tokio::sync::mpsc::channel(100);

        tokio::spawn(async move {
            let mut read_stream = ws_stream;
            while let Some(msg) = read_stream.next().await {
                match msg {
                    Ok(tungstenite::Message::Text(text)) => {
                        tracing::trace!("Received Loki WebSocket text: {}", text);
                        if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) {
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
    ) -> Result<Vec<String>> {
        let query = format!("{{job_name=\"{}\"}}", job_name);
        let url = format!("{}/loki/api/v1/query_range", loki_url.trim_end_matches('/'));

        let client = self.create_client();
        let forward = "forward".to_string();
        let limit = "5000".to_string();

        let start_time = started_at
            .map(|t| t - chrono::Duration::minutes(1))
            .unwrap_or_else(|| chrono::Utc::now() - chrono::Duration::days(30))
            .timestamp_nanos_opt()
            .unwrap_or(0)
            .to_string();

        let end_time = finished_at
            .unwrap_or_else(chrono::Utc::now)
            // Add a small buffer to end time to ensure we get the final logs
            .checked_add_signed(chrono::Duration::minutes(5))
            .unwrap_or_else(chrono::Utc::now)
            .timestamp_nanos_opt()
            .unwrap_or(0)
            .to_string();

        let resp = client
            .get(&url)
            .query(&[
                ("query", &query),
                ("direction", &forward),
                ("limit", &limit),
                ("start", &start_time),
                ("end", &end_time),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow::anyhow!("Loki returned status {}", resp.status()));
        }

        let data: serde_json::Value = resp.json().await?;
        let mut logs = Vec::new();

        if let Some(streams) = data["data"]["result"].as_array() {
            for stream in streams {
                if let Some(values) = stream["values"].as_array() {
                    for entry in values {
                        if let Some(log_line) = entry.get(1).and_then(|v| v.as_str()) {
                            logs.push(log_line.to_string());
                        }
                    }
                }
            }
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

        let query = serde_json::json!({
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
                { "@timestamp": "asc" }
            ],
            "size": 5000
        });

        let client = self.create_client();
        let resp = client.post(&url).json(&query).send().await?;

        if !resp.status().is_success() {
            return Err(anyhow::anyhow!(
                "Elasticsearch returned status {}",
                resp.status()
            ));
        }

        let data: serde_json::Value = resp.json().await?;
        let mut logs = Vec::new();

        if let Some(hits) = data["hits"]["hits"].as_array() {
            for hit in hits {
                if let Some(message) = hit["_source"]["message"].as_str() {
                    logs.push(message.to_string());
                }
            }
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
            .fetch_step_logs("test-step", step_id, None, None)
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
            .fetch_step_logs("test-step", step_id, None, None)
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
            .fetch_step_logs("test-step", step_id, None, None)
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
            .fetch_step_logs("test-step", step_id, None, None)
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
