#[cfg(not(target_arch = "wasm32"))]
pub mod elasticsearch;
#[cfg(not(target_arch = "wasm32"))]
pub mod loki;

use crate::id::*;
#[cfg(not(target_arch = "wasm32"))]
use anyhow::Result;
#[cfg(not(target_arch = "wasm32"))]
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
#[cfg(not(target_arch = "wasm32"))]
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use tokio::sync::mpsc;

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

#[cfg(not(target_arch = "wasm32"))]
impl LogBackend {
    pub(crate) fn create_client(&self) -> ClientWithMiddleware {
        let retry_policy = ExponentialBackoff::builder().build_with_max_retries(5);
        ClientBuilder::new(reqwest::Client::new())
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build()
    }

    /// Fetches historical logs for a specific step instance.
    pub async fn fetch_step_logs(
        &self,
        step_name: &str,
        step_id: StepInstanceId,
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
                loki::fetch_loki_logs(self, url, &job_name, started_at, finished_at, limit).await
            }
            LogBackend::Elasticsearch { url, index } => {
                elasticsearch::fetch_elasticsearch_logs(
                    self,
                    url,
                    index,
                    &job_name,
                    started_at,
                    finished_at,
                    limit,
                )
                .await
            }
        }
    }

    /// Opens a real-time stream of log lines for a currently executing step instance.
    pub async fn stream_step_logs(
        &self,
        step_name: &str,
        step_id: StepInstanceId,
    ) -> Result<mpsc::Receiver<Result<String>>> {
        let job_name = format!(
            "storm-{}-{}",
            step_name.to_lowercase().replace('_', "-"),
            &step_id.to_string()[..8]
        );

        match self {
            LogBackend::Loki { url } => loki::stream_loki_logs(url, &job_name).await,
            LogBackend::Elasticsearch { .. } => {
                anyhow::bail!(
                    "Log streaming is not currently supported for Elasticsearch backends"
                );
            }
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stream_step_logs_elasticsearch_unsupported() {
        let backend = LogBackend::Elasticsearch {
            url: "http://localhost:9200".to_string(),
            index: "my-index".to_string(),
        };
        let step_id = StepInstanceId::new_v4();

        let result = backend.stream_step_logs("test-step", step_id).await;
        let err = result.unwrap_err();
        assert!(err.to_string().contains("not currently supported"));
    }
}
