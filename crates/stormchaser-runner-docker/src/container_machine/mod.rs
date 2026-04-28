pub mod crypto;
pub mod docker_utils;
pub mod transitions;

use bollard::Docker;
use serde::{Deserialize, Serialize};
use stormchaser_model::dsl::Step;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerMetadata {
    pub run_id: Uuid,
    pub step_id: Uuid,
    pub step_dsl: Step,
    pub received_at: chrono::DateTime<chrono::Utc>,
    pub encryption_key: Option<String>,
    pub storage: Option<std::collections::HashMap<String, serde_json::Value>>,
    pub test_report_urls: Option<std::collections::HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContainerMetrics {
    pub exit_code: Option<i64>,
    pub duration_ms: u64,
    pub latency_ms: u64,
    pub storage_hashes: Option<std::collections::HashMap<String, String>>,
    pub artifacts: Option<std::collections::HashMap<String, serde_json::Value>>,
    pub test_reports: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContainerState {
    Succeeded(ContainerMetrics),
    Failed(String, ContainerMetrics),
}

pub enum StartResult {
    Running(DockerContainerMachine<state::Running>),
    #[allow(dead_code)]
    Failed(DockerContainerMachine<state::Finished>),
}

pub mod state {
    pub struct Initialized;
    pub struct Running {
        pub container_name: String,
        pub dispatched_at: chrono::DateTime<chrono::Utc>,
        pub volumes_to_cleanup: Vec<String>,
        pub storage_names: Vec<String>,
        pub mounts: Vec<bollard::service::Mount>,
    }
    pub struct Finished {
        pub result: super::ContainerState,
    }
}

#[derive(Clone)]
pub struct DockerContainerMachine<S> {
    pub nats: Option<async_nats::Client>,
    pub docker: Docker,
    pub metadata: ContainerMetadata,
    pub state: S,
}

impl DockerContainerMachine<state::Initialized> {
    pub fn new(
        docker: Docker,
        metadata: ContainerMetadata,
        nats: Option<async_nats::Client>,
    ) -> Self {
        Self {
            nats,
            docker,
            metadata,
            state: state::Initialized,
        }
    }
}
