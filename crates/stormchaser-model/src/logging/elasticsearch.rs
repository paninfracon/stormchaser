use crate::logging::LogBackend;
use anyhow::Result;

pub(crate) async fn fetch_elasticsearch_logs(
    backend: &LogBackend,
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

        let client = backend.create_client();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::LogBackend;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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

        let logs = fetch_elasticsearch_logs(
            &backend,
            &mock_server.uri(),
            "my-index",
            "storm-test-step-12345678",
            None,
            None,
            Some(2),
        )
        .await
        .expect("Failed to fetch es logs");

        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0], "es log line 1");
        assert_eq!(logs[1], "es log line 2");
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

        let result = fetch_elasticsearch_logs(
            &backend,
            &mock_server.uri(),
            "my-index",
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
            .contains("Elasticsearch returned status"));
    }
}
