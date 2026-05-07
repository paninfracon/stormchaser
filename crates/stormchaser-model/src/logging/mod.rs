pub mod elasticsearch;
pub mod loki;

use crate::id::*;
use anyhow::Result;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use serde::{Deserialize, Serialize};
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
        step_id: StepId,
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
        step_id: StepId,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StepId;

    #[tokio::test]
    async fn test_stream_step_logs_elasticsearch_unsupported() {
        let backend = LogBackend::Elasticsearch {
            url: "http://localhost:9200".to_string(),
            index: "my-index".to_string(),
        };
        let step_id = StepId::new_v4();

        let result = backend.stream_step_logs("test-step", step_id).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not currently supported"));
    }
}
