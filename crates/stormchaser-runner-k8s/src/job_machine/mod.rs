pub mod crypto;
pub mod k8s_utils;
pub mod transitions;

use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::ResourceRequirements as K8sResources;
use kube::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use stormchaser_model::dsl::Step;
use uuid::Uuid;

use stormchaser_model::dsl;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobMetadata {
    pub run_id: Uuid,
    pub step_id: Uuid,
    pub step_dsl: Step,
    pub namespace: String,
    pub received_at: DateTime<Utc>,
    pub cluster_version: String,
    pub encryption_key: Option<String>,
    pub storage: Option<std::collections::HashMap<String, serde_json::Value>>,
    pub test_report_urls: Option<std::collections::HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JobMetrics {
    pub exit_code: Option<i32>,
    pub attempts: i32,
    pub duration_ms: u64,
    pub latency_ms: u64,
    pub storage_hashes: Option<std::collections::HashMap<String, String>>,
    pub artifacts: Option<std::collections::HashMap<String, serde_json::Value>>,
    pub test_reports: Option<std::collections::HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobState {
    Succeeded(JobMetrics),
    Failed(String, JobMetrics),
}

pub enum StartResult {
    Running(K8sJobMachine<state::Running>),
    Failed(K8sJobMachine<state::Finished>),
}

pub mod state {
    #[derive(Clone)]
    pub struct Initialized;
    pub struct Running {
        pub job_name: String,
        pub dispatched_at: chrono::DateTime<chrono::Utc>,
    }
    pub struct Finished {
        pub result: super::JobState,
    }
}

#[derive(Clone)]
pub struct K8sJobMachine<S> {
    pub client: Client,
    pub metadata: JobMetadata,
    pub state: S,
}

#[derive(Debug, Deserialize)]
pub struct K8sJobSpec {
    pub image: String,
    pub command: Option<Vec<String>>,
    pub args: Option<Vec<String>>,
    pub env: Option<Vec<dsl::EnvVar>>,
    pub resources: Option<K8sResources>,
    pub active_deadline_seconds: Option<i64>,
    pub backoff_limit: Option<i32>,
    pub completions: Option<i32>,
    pub parallelism: Option<i32>,
    pub ttl_seconds_after_finished: Option<i32>,
    pub privileged: Option<bool>,
    pub node_selector: Option<BTreeMap<String, String>>,
    pub service_account_name: Option<String>,
    pub restart_policy: Option<String>,
    pub labels: Option<BTreeMap<String, String>>,
    pub annotations: Option<BTreeMap<String, String>>,
    pub storage_mounts: Option<Vec<dsl::StorageMount>>,
    pub secret_mounts: Option<Vec<dsl::SecretMount>>,
    pub config_map_mounts: Option<Vec<dsl::ConfigMapMount>>,
    #[allow(dead_code)]
    pub minimum_version: Option<String>,
}

impl K8sJobMachine<state::Initialized> {
    pub fn new(client: Client, metadata: JobMetadata) -> Self {
        Self {
            client,
            metadata,
            state: state::Initialized,
        }
    }
}
