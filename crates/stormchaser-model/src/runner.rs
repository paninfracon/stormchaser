//! Models for workflow runner instances and step definitions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The connection status of a remote runner.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::Type, PartialEq, Eq, schemars::JsonSchema)]
#[sqlx(type_name = "runner_status", rename_all = "snake_case")]
pub enum RunnerStatus {
    /// The runner is currently online and communicating.
    Online,
    /// The runner has missed heartbeats and is considered offline.
    Offline,
}

/// Represents a registered workflow runner instance.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct Runner {
    /// Unique identifier for the runner.
    pub id: String,
    /// The type of runner environment.
    pub runner_type: String, // e.g., "k8s", "binary", "ecs"
    /// The current connection status of the runner.
    pub status: RunnerStatus,
    /// The protocol version the runner uses to communicate.
    pub protocol_version: String,
    /// A list of capabilities supported by the runner.
    pub capabilities: Vec<String>,
    /// Subject used to communicate with this specific runner.
    pub nats_subject: String, // Subject used to communicate with this specific runner
    /// Timestamp of the last received heartbeat.
    pub last_heartbeat_at: DateTime<Utc>,
    /// Timestamp when the runner first registered.
    pub registered_at: DateTime<Utc>,
}

/// Defines a reusable workflow step implementation available on runners.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct StepDefinition {
    /// Identifier for the step type (e.g., 'docker', 'wasm').
    pub step_type: String,
    /// JSON schema describing the required and optional parameters for this step.
    pub schema: Value,
    /// Optional documentation or description for the step type.
    pub documentation: Option<String>,
    /// Timestamp when this step definition was registered.
    pub registered_at: DateTime<Utc>,
}

/// Where the engine tells runners to stop a run's steps.
///
/// It sits under `stormchaser.v1.runner.`, which runners may already read. No account
/// but the engine's may publish to it, so a runner or the API cannot stop someone's run.
pub const CANCEL_SUBJECT: &str = "stormchaser.v1.runner.cancel";

/// The durable consumer every runner pulls `RunContainer` tasks from, Docker and
/// Kubernetes alike. One consumer shared by all of them hands each task to one runner.
/// A consumer per kind of runner would give every kind its own copy, and the step
/// would run once on each.
pub const CONTAINER_TASKS_CONSUMER: &str = "runner-container-tasks";
/// The tasks [`CONTAINER_TASKS_CONSUMER`] holds.
pub const CONTAINER_TASKS_SUBJECT: &str = "stormchaser.v1.*.step.scheduled.runcontainer";
/// The durable consumer Kubernetes runners pull `RunK8sJob` tasks from.
pub const K8S_JOB_TASKS_CONSUMER: &str = "runner-k8s-job-tasks";
/// The tasks [`K8S_JOB_TASKS_CONSUMER`] holds.
pub const K8S_JOB_TASKS_SUBJECT: &str = "stormchaser.v1.*.step.scheduled.runk8sjob";

/// Binds to the shared durable consumer `name`, filtered to `subject`, creating it if
/// no runner has yet.
pub async fn task_consumer(
    stream: &async_nats::jetstream::stream::Stream,
    name: &str,
    subject: &str,
) -> anyhow::Result<
    async_nats::jetstream::consumer::Consumer<async_nats::jetstream::consumer::pull::Config>,
> {
    use anyhow::Context;
    stream
        .get_or_create_consumer(
            name,
            async_nats::jetstream::consumer::pull::Config {
                durable_name: Some(name.to_string()),
                filter_subject: subject.to_string(),
                ..Default::default()
            },
        )
        .await
        .with_context(|| format!("Failed to create the JetStream consumer {name}"))
}

/// Chooses the id a runner registers under.
///
/// An id the operator sets (`RUNNER_ID`) always wins. Otherwise the id is built from the
/// host name, which for a container is its own name: it survives a restart of the runner
/// process, so the runner comes back as itself and keeps the steps assigned to it, where
/// a random id made every restart look like a new runner and left its steps to be failed
/// as lost. A random id is the last resort.
///
/// Ids must be unique among running runners. Set `RUNNER_ID` where host names are not.
pub fn runner_id(kind: &str, explicit: Option<&str>, hostname: Option<&str>) -> String {
    fn non_empty(value: Option<&str>) -> Option<&str> {
        value.map(str::trim).filter(|v| !v.is_empty())
    }
    if let Some(id) = non_empty(explicit) {
        return id.to_string();
    }
    match non_empty(hostname) {
        Some(host) => format!("{}-{}", kind, host),
        None => uuid::Uuid::new_v4().to_string(),
    }
}

/// The machine's host name as the kernel reports it, for runners started without a
/// `HOSTNAME` variable.
#[cfg(not(target_arch = "wasm32"))]
pub fn kernel_hostname() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

#[cfg(test)]
mod runner_id_tests {
    use super::runner_id;

    #[test]
    fn an_explicit_id_wins() {
        assert_eq!(runner_id("k8s", Some("east-1"), Some("pod-abc")), "east-1");
    }

    #[test]
    fn the_host_name_gives_a_stable_id() {
        assert_eq!(
            runner_id("k8s", None, Some("runner-7d9f-x2x")),
            "k8s-runner-7d9f-x2x"
        );
        assert_eq!(
            runner_id("docker", Some("  "), Some("build-host")),
            "docker-build-host"
        );
        // Two kinds of runner on one host stay distinct
        assert_ne!(
            runner_id("k8s", None, Some("h")),
            runner_id("docker", None, Some("h"))
        );
    }

    #[test]
    fn with_nothing_to_go_on_the_id_is_random() {
        let (a, b) = (
            runner_id("k8s", None, None),
            runner_id("k8s", Some(""), Some("")),
        );
        assert!(!a.is_empty() && a != b);
    }
}
