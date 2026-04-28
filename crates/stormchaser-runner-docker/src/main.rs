use axum::extract::State;
use serde_json::Value;
use std::collections::HashMap;
use tokio::time::sleep;
mod container_machine;
pub mod traits;

use anyhow::{Context, Result};
use axum::{routing::get, Router};
use bollard::container::ListContainersOptions;
use bollard::volume::ListVolumesOptions;
use bollard::Docker;
use container_machine::{ContainerMetadata, ContainerState, DockerContainerMachine};
use futures::StreamExt;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::time;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

use stormchaser_model::dsl;

pub fn parse_step_from_nats_payload(payload: &Value) -> Result<dsl::Step> {
    if let Some(dsl_val) = payload.get("step_dsl") {
        if !dsl_val.is_null() {
            if let Ok(step) = serde_json::from_value(dsl_val.clone()) {
                return Ok(step);
            }
        }
    }

    let spec =
        serde_json::from_value(payload["spec"].clone()).context("Failed to parse step spec")?;

    Ok(dsl::Step {
        name: payload["step_name"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        r#type: payload["step_type"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        spec,
        params: serde_json::from_value(payload["params"].clone()).unwrap_or_default(),
        condition: None,
        strategy: None,
        aggregation: Vec::new(),
        iterate: None,
        iterate_as: None,
        steps: None,
        next: Vec::new(),
        on_failure: None,
        retry: None,
        timeout: None,
        allow_failure: None,
        start_marker: None,
        end_marker: None,
        outputs: Vec::new(),
        reports: Vec::new(),
        artifacts: None,
    })
}

pub fn parse_step_from_docker_labels(
    container_name: &str,
    raw_step_dsl: Option<&String>,
    is_encrypted: bool,
    encryption_key: Option<&String>,
) -> Result<dsl::Step> {
    if let Some(raw) = raw_step_dsl {
        let dsl_str = if is_encrypted {
            if let Some(key) = encryption_key {
                container_machine::crypto::decrypt_state(raw, key).context(format!(
                    "Failed to decrypt state for container {}",
                    container_name
                ))?
            } else {
                anyhow::bail!(
                    "Container {} is encrypted but no encryption key is configured",
                    container_name
                );
            }
        } else {
            raw.clone()
        };

        if let Ok(step) = serde_json::from_str(&dsl_str) {
            return Ok(step);
        }
    }

    // Fallback
    Ok(dsl::Step {
        name: container_name.to_string(),
        r#type: "RunContainer".to_string(),
        spec: Value::Null,
        params: HashMap::new(),
        condition: None,
        strategy: None,
        aggregation: Vec::new(),
        iterate: None,
        iterate_as: None,
        steps: None,
        next: Vec::new(),
        on_failure: None,
        retry: None,
        timeout: None,
        allow_failure: None,
        start_marker: None,
        end_marker: None,
        outputs: Vec::new(),
        reports: Vec::new(),
        artifacts: None,
    })
}

async fn run_reaper(docker: Docker) -> Result<()> {
    let mut interval = time::interval(Duration::from_secs(3600)); // Check every hour
    loop {
        interval.tick().await;
        info!("Running garbage collection (reaper) on Docker containers and volumes...");

        let mut filters = HashMap::new();
        filters.insert("label", vec!["managed-by=stormchaser"]);

        // 1. Reap old containers
        if let Ok(containers) = docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: filters.clone(),
                ..Default::default()
            }))
            .await
        {
            for container in containers {
                let now = chrono::Utc::now();
                let created_ts = container.created.unwrap_or(0);
                let created = chrono::DateTime::from_timestamp(created_ts, 0)
                    .unwrap_or_else(chrono::Utc::now);

                let age = now - created;
                if age.num_hours() >= 24 {
                    if let Some(id) = container.id {
                        info!("Reaping old container {} (age: {}h)", id, age.num_hours());
                        let _ = docker.stop_container(&id, None).await;
                        let _ = docker.remove_container(&id, None).await;
                    }
                }
            }
        }

        // 2. Reap old volumes
        if let Ok(volumes_res) = docker
            .list_volumes(Some(ListVolumesOptions {
                filters: filters.clone(),
            }))
            .await
        {
            if let Some(volumes) = volumes_res.volumes {
                for vol in volumes {
                    // Docker volumes don't have a reliable creation date in the list output
                    // We'll skip time-based reaping for now and just reap unmounted ones?
                    // Actually, let's just reap all of them that have the label.
                    // To be safe, we could check if they are "in use" but bollard doesn't make it easy to see in list.
                    // For now, let's just log them. SFS Phase 1 cleans up volumes on success anyway.
                    info!("Found managed volume: {}", vol.name);
                }
            }
        }
    }
}

async fn scan_for_orphans(
    docker: Docker,
    nats_client: async_nats::Client,
    runner_id: String,
    encryption_key: Option<String>,
) -> Result<()> {
    info!("Scanning for orphaned Docker containers...");

    let mut filters = HashMap::new();
    filters.insert("label", vec!["managed-by=stormchaser"]);

    let containers = docker
        .list_containers(Some(ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        }))
        .await?;

    for container in containers {
        let container_id = container.id.unwrap_or_default();
        let labels = container.labels.unwrap_or_default();

        let run_id_str = labels.get("stormchaser-run-id");
        let step_id_str = labels.get("stormchaser-step-id");

        if let (Some(run_id_s), Some(step_id_s)) = (run_id_str, step_id_str) {
            let run_id = Uuid::parse_str(run_id_s).unwrap_or_default();
            let step_id = Uuid::parse_str(step_id_s).unwrap_or_default();

            let container_name = container
                .names
                .and_then(|names| names.first().cloned())
                .unwrap_or_else(|| container_id.clone())
                .replace('/', ""); // Docker names often have leading slash

            info!(
                "Found orphaned container {} for step {} (run {})",
                container_name, step_id, run_id
            );

            let received_at = labels
                .get("stormchaser.io/received-at")
                .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(chrono::Utc::now);

            let is_encrypted = labels
                .get("stormchaser.io/state-encrypted")
                .map(|v| v == "true")
                .unwrap_or(false);

            let raw_step_dsl = labels.get("stormchaser.io/step-dsl");

            let step_dsl = match parse_step_from_docker_labels(
                &container_name,
                raw_step_dsl,
                is_encrypted,
                encryption_key.as_ref(),
            ) {
                Ok(dsl) => dsl,
                Err(e) => {
                    error!(
                        "Failed to parse step spec for container {}: {:?}",
                        container_name, e
                    );
                    continue;
                }
            };

            let nats = nats_client.clone();
            let r_id = runner_id.clone();
            let docker_clone = docker.clone();
            let key_clone = encryption_key.clone();

            tokio::spawn(async move {
                // Before adopting, query the orchestrator to see if this step is still relevant
                let query_payload = json!({
                    "step_id": step_id,
                });

                match nats
                    .request("stormchaser.step.query", query_payload.to_string().into())
                    .await
                {
                    Ok(reply) => {
                        let response: Value =
                            serde_json::from_slice(&reply.payload).unwrap_or_default();
                        let status = response["status"].as_str().unwrap_or_default();
                        let exists = response["exists"].as_bool().unwrap_or(false);

                        if !exists || (status != "pending" && status != "running") {
                            info!("Step {} is in status {}, skipping adoption and cleaning up container {}", step_id, status, container_name);
                            let machine = DockerContainerMachine::new(
                                docker_clone,
                                ContainerMetadata {
                                    run_id,
                                    step_id,
                                    step_dsl,
                                    storage: None,
                                    test_report_urls: None,
                                    encryption_key: key_clone,
                                    received_at,
                                },
                                Some(nats.clone()),
                            );
                            let _ = machine.clean_up(&container_name).await;
                            return;
                        }
                    }
                    Err(e) => {
                        error!("Failed to query orchestrator for step {}: {:?}", step_id, e);
                        return;
                    }
                }

                let metadata = ContainerMetadata {
                    run_id,
                    step_id,
                    step_dsl: step_dsl.clone(),
                    storage: None,
                    test_report_urls: None,
                    encryption_key: key_clone,
                    received_at,
                };

                let machine =
                    DockerContainerMachine::new(docker_clone, metadata, Some(nats.clone()));

                match machine.adopt(container_name.clone()).wait().await {
                    Ok(finished_machine) => match finished_machine.into_result() {
                        ContainerState::Succeeded(metrics) => {
                            info!("Adopted step {} completed successfully", step_id);
                            let event = json!({
                                "run_id": run_id,
                                "step_id": step_id,
                                "status": "succeeded",
                                "runner_id": r_id,
                                "exit_code": metrics.exit_code,
                                "outputs": {
                                    "docker exit code": metrics.exit_code,
                                    "run duration": format!("{}ms", metrics.duration_ms),
                                    "run latency": format!("{}ms", metrics.latency_ms),
                                }
                            });
                            let _ = nats
                                .publish("stormchaser.step.completed", event.to_string().into())
                                .await;
                        }
                        ContainerState::Failed(reason, metrics) => {
                            warn!("Adopted step {} failed: {}", step_id, reason);
                            let event = json!({
                                "run_id": run_id,
                                "step_id": step_id,
                                "status": "failed",
                                "error": reason,
                                "runner_id": r_id,
                                "exit_code": metrics.exit_code,
                                "outputs": {
                                    "docker exit code": metrics.exit_code,
                                    "run duration": format!("{}ms", metrics.duration_ms),
                                    "run latency": format!("{}ms", metrics.latency_ms),
                                }
                            });
                            let _ = nats
                                .publish("stormchaser.step.failed", event.to_string().into())
                                .await;
                        }
                    },
                    Err(e) => error!("Error adopting container {}: {:?}", container_name, e),
                }
            });
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
pub struct Config {
    pub nats_url: String,
    pub runner_id: String,
    pub encryption_key: Option<String>,
    pub rust_log: String,
}

impl Config {
    pub fn from_env<I, K, V>(env: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut nats_url = "nats://localhost:4222".to_string();
        let mut runner_id = Uuid::new_v4().to_string();
        let mut encryption_key = None;
        let mut rust_log = "stormchaser_runner_docker=info".to_string();

        for (k, v) in env {
            match k.as_ref() {
                "NATS_URL" => nats_url = v.as_ref().to_string(),
                "RUNNER_ID" => runner_id = v.as_ref().to_string(),
                "STORMCHASER_STATE_ENCRYPTION_KEY" => encryption_key = Some(v.as_ref().to_string()),
                "RUST_LOG" => rust_log = v.as_ref().to_string(),
                _ => {}
            }
        }

        Self {
            nats_url,
            runner_id,
            encryption_key,
            rust_log,
        }
    }
}

struct AppState {
    is_ready: watch::Receiver<bool>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env(std::env::vars());
    run_runner(config).await
}

pub async fn run_runner(config: Config) -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install default crypto provider");

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(&config.rust_log))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let nats_url = config.nats_url;
    let runner_id = config.runner_id;
    let encryption_key = config.encryption_key;

    if encryption_key.is_some() {
        info!("State encryption is enabled");
    } else {
        warn!("State encryption is DISABLED. Sensitive step data in Docker labels will be stored in plaintext.");
    }

    info!("Starting Stormchaser Docker Runner: {}", runner_id);

    // Initialize Docker client
    let docker =
        Docker::connect_with_local_defaults().context("Failed to connect to Docker daemon")?;

    // Watch channel for readiness state
    let (ready_tx, ready_rx) = watch::channel(false);

    // 1. Start Health Check Server
    let health_state = Arc::new(AppState { is_ready: ready_rx });

    let reaper_docker = docker.clone();
    tokio::spawn(async move {
        if let Err(e) = run_reaper(reaper_docker).await {
            error!("Reaper error: {:?}", e);
        }
    });

    let app = Router::new()
        .route("/healthz", get(|| async { "OK" }))
        .route(
            "/readyz",
            get(|state: State<Arc<AppState>>| async move {
                if *state.is_ready.borrow() {
                    axum::http::StatusCode::OK
                } else {
                    axum::http::StatusCode::SERVICE_UNAVAILABLE
                }
            }),
        )
        .with_state(health_state);

    let health_addr = SocketAddr::from(([0, 0, 0, 0], 8081)); // Different port from K8s runner
    let listener = tokio::net::TcpListener::bind(health_addr).await?;
    info!("Health server listening on {}", health_addr);

    let health_server = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            error!("Health server error: {:?}", e);
        }
    });

    // 2. Connect to NATS
    let nats_client = async_nats::connect(nats_url)
        .await
        .context("Failed to connect to NATS")?;

    // 3. Register with the Orchestration Engine
    let nats_subject = format!("stormchaser.runner.docker.{}", runner_id);

    // Generate JSON Schema for our supported step type
    let common_schema = schemars::schema_for!(dsl::CommonContainerSpec);
    let common_schema_json = serde_json::to_value(common_schema)?;

    let registration_payload = json!({
        "runner_id": runner_id,
        "runner_type": "docker",
        "protocol_version": "v1",
        "nats_subject": nats_subject,
        "capabilities": ["docker", "linux", "container"],
        "step_types": [
            {
                "step_type": "RunContainer",
                "schema": common_schema_json,
                "documentation": "Runs a container using Docker with a minimal common set of parameters."
            }
        ]
    });

    nats_client
        .publish(
            "stormchaser.runner.register",
            registration_payload.to_string().into(),
        )
        .await
        .context("Failed to publish registration event")?;

    info!("Runner registered successfully");

    // Scan for orphaned containers
    let _ = scan_for_orphans(
        docker.clone(),
        nats_client.clone(),
        runner_id.clone(),
        encryption_key.clone(),
    )
    .await;

    let _ = ready_tx.send(true);

    // 4. Start heartbeat loop
    let heartbeat_client = nats_client.clone();
    let heartbeat_id = runner_id.clone();
    let mut heartbeat_interval = time::interval(Duration::from_secs(10));

    // 5. Subscribe to task subjects
    let js = async_nats::jetstream::new(nats_client.clone());
    let mut runner_subscriber = nats_client.subscribe(nats_subject.clone()).await?;

    info!("Ensuring JetStream stream 'stormchaser' exists...");
    let stream = js
        .get_or_create_stream(async_nats::jetstream::stream::Config {
            name: "stormchaser".to_string(),
            subjects: vec!["stormchaser.>".to_string()],
            ..Default::default()
        })
        .await
        .context("Failed to ensure JetStream stream")?;

    info!("Creating durable consumer for docker-runner...");
    let consumer = stream
        .get_or_create_consumer(
            "docker-runner",
            async_nats::jetstream::consumer::pull::Config {
                durable_name: Some("docker-runner".to_string()),
                filter_subject: "stormchaser.step.scheduled.runcontainer".to_string(),
                ..Default::default()
            },
        )
        .await
        .context("Failed to create JetStream consumer")?;

    let mut task_messages = consumer
        .messages()
        .await
        .context("Failed to get consumer messages")?;

    info!(
        "Listening for tasks on {} and JetStream subject stormchaser.step.scheduled.runcontainer",
        nats_subject
    );

    // 6. Main event loop
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

    loop {
        tokio::select! {
            _ = sigterm.recv() => {
                info!("SIGTERM received, shutting down...");
                break;
            }
            _ = sigint.recv() => {
                info!("SIGINT received, shutting down...");
                break;
            }
            _ = heartbeat_interval.tick() => {
                let heartbeat_payload = json!({
                    "runner_id": heartbeat_id,
                });

                if let Err(e) = heartbeat_client
                    .publish("stormchaser.runner.heartbeat", heartbeat_payload.to_string().into())
                    .await
                {
                    error!("Failed to publish heartbeat: {:?}", e);
                }
            }
            message = runner_subscriber.next() => {
                if let Some(msg) = message {
                    info!("Received runner-specific message: {:?}", msg.payload);
                }
            }
            message = task_messages.next() => {
                match message {
                    Some(Ok(msg)) => {
                        tokio::spawn(handle_task(
                            msg,
                            docker.clone(),
                            nats_client.clone(),
                            runner_id.clone(),
                            encryption_key.clone(),
                        ));
                    }
                    Some(Err(e)) => {
                        error!("JetStream consumer error: {:?}", e);
                        time::sleep(Duration::from_secs(1)).await;
                    }
                    None => {
                        error!("JetStream consumer closed");
                        break;
                    }
                }
            }
        }
    }

    health_server.abort();
    info!("Runner stopped");

    Ok(())
}

async fn handle_task(
    msg: async_nats::jetstream::message::Message,
    docker: bollard::Docker,
    nats_client: async_nats::Client,
    runner_id: String,
    encryption_key: Option<String>,
) {
    use crate::container_machine::{ContainerMetadata, ContainerState, DockerContainerMachine};
    use serde_json::json;
    use tracing::{error, info};

    let received_at = chrono::Utc::now();
    info!("Received task message: {:?}", msg.subject);

    let payload: Value = serde_json::from_slice(&msg.payload).unwrap_or_default();
    let run_id_str = payload["run_id"].as_str().unwrap_or_default();
    let run_id = Uuid::parse_str(run_id_str).unwrap_or_default();
    let step_id_str = payload["step_id"].as_str().unwrap_or_default();
    let step_id = Uuid::parse_str(step_id_str).unwrap_or_default();

    let step_dsl: dsl::Step = match payload.get("step_dsl").and_then(|v| {
        if !v.is_null() {
            serde_json::from_value(v.clone()).ok()
        } else {
            None
        }
    }) {
        Some(step) => step,
        None => match serde_json::from_value(payload["spec"].clone()) {
            Ok(spec) => dsl::Step {
                name: payload["step_name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                r#type: payload["step_type"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                spec,
                params: serde_json::from_value(payload["params"].clone()).unwrap_or_default(),
                condition: None,
                strategy: None,
                aggregation: Vec::new(),
                iterate: None,
                iterate_as: None,
                steps: None,
                next: Vec::new(),
                on_failure: None,
                retry: None,
                timeout: None,
                allow_failure: None,
                start_marker: None,
                end_marker: None,
                outputs: Vec::new(),
                reports: Vec::new(),
                artifacts: None,
            },
            Err(e) => {
                error!("Failed to parse step spec: {:?}", e);
                return;
            }
        },
    };

    let storage: Option<HashMap<String, Value>> =
        serde_json::from_value(payload["storage"].clone()).ok();
    let test_report_urls: Option<HashMap<String, Value>> =
        serde_json::from_value(payload["test_report_urls"].clone()).ok();

    let in_progress_msg = msg.clone();
    let in_progress_handle = tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(15)).await;
            let _ = in_progress_msg
                .ack_with(async_nats::jetstream::message::AckKind::Progress)
                .await;
        }
    });

    let running_event = json!({
        "run_id": run_id,
        "step_id": step_id,
        "status": "running",
        "runner_id": runner_id,
        "timestamp": chrono::Utc::now(),
    });
    let _ = nats_client
        .publish("stormchaser.step.running", running_event.to_string().into())
        .await;

    let machine = DockerContainerMachine::new(
        docker,
        ContainerMetadata {
            run_id,
            step_id,
            step_dsl,
            storage,
            test_report_urls,
            encryption_key,
            received_at,
        },
        Some(nats_client.clone()),
    );

    let result = match machine.start().await {
        Ok(crate::container_machine::StartResult::Running(running_machine)) => {
            running_machine.wait().await.map(|m| m.into_result())
        }
        Ok(crate::container_machine::StartResult::Failed(finished_machine)) => {
            Ok(finished_machine.into_result())
        }
        Err(e) => Err(e),
    };

    in_progress_handle.abort();
    let _ = msg.double_ack().await;

    match result {
        Ok(ContainerState::Succeeded(metrics)) => {
            info!("Step {} (Run {}) completed successfully", step_id, run_id);
            let event = json!({
                "run_id": run_id,
                "step_id": step_id,
                "status": "succeeded",
                "runner_id": runner_id,
                "exit_code": metrics.exit_code,
                "storage_hashes": metrics.storage_hashes,
                "artifacts": metrics.artifacts,
                "test_reports": metrics.test_reports,
                "outputs": {
                    "docker exit code": metrics.exit_code,
                    "run duration": format!("{}ms", metrics.duration_ms),
                    "run latency": format!("{}ms", metrics.latency_ms),
                }
            });
            let _ = nats_client
                .publish("stormchaser.step.completed", event.to_string().into())
                .await;
        }
        Ok(ContainerState::Failed(reason, metrics)) => {
            error!("Step {} (Run {}) failed: {}", step_id, run_id, reason);
            let event = json!({
                "run_id": run_id,
                "step_id": step_id,
                "status": "failed",
                "error": reason,
                "runner_id": runner_id,
                "exit_code": metrics.exit_code,
                "storage_hashes": metrics.storage_hashes,
                "artifacts": metrics.artifacts,
                "test_reports": metrics.test_reports,
                "outputs": {
                    "docker exit code": metrics.exit_code,
                    "run duration": format!("{}ms", metrics.duration_ms),
                    "run latency": format!("{}ms", metrics.latency_ms),
                }
            });
            let _ = nats_client
                .publish("stormchaser.step.failed", event.to_string().into())
                .await;
        }
        Err(e) => {
            error!("Error running container for step {}: {:?}", step_id, e);
            let event = json!({
                "run_id": run_id,
                "step_id": step_id,
                "status": "failed",
                "error": format!("{:?}", e),
                "runner_id": runner_id,
            });
            let _ = nats_client
                .publish("stormchaser.step.failed", event.to_string().into())
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_step_from_nats_payload() {
        let payload = json!({
            "step_name": "test_step",
            "step_type": "RunContainer",
            "spec": {
                "image": "alpine",
                "command": ["echo"],
                "args": ["hello"]
            },
            "params": {}
        });

        let result = parse_step_from_nats_payload(&payload).unwrap();
        assert_eq!(result.name, "test_step");
        assert_eq!(result.r#type, "RunContainer");
    }

    #[test]
    fn test_parse_step_from_docker_labels_fallback() {
        let result = parse_step_from_docker_labels("my_container", None, false, None).unwrap();
        assert_eq!(result.name, "my_container");
        assert_eq!(result.r#type, "RunContainer");
        assert!(result.spec.is_null());
    }

    #[test]
    fn test_config_from_env_defaults() {
        let env: Vec<(String, String)> = vec![];
        let config = Config::from_env(env);
        assert_eq!(config.nats_url, "nats://localhost:4222");
        assert!(config.encryption_key.is_none());
        assert_eq!(config.rust_log, "stormchaser_runner_docker=info");
        // runner_id is random, just assert it's not empty
        assert!(!config.runner_id.is_empty());
    }

    #[test]
    fn test_config_from_env_custom() {
        let env = vec![
            ("NATS_URL".to_string(), "nats://remote:4222".to_string()),
            ("RUNNER_ID".to_string(), "my-runner".to_string()),
            (
                "STORMCHASER_STATE_ENCRYPTION_KEY".to_string(),
                "my-key".to_string(),
            ),
            ("RUST_LOG".to_string(), "debug".to_string()),
        ];
        let config = Config::from_env(env);
        assert_eq!(config.nats_url, "nats://remote:4222");
        assert_eq!(config.runner_id, "my-runner");
        assert_eq!(config.encryption_key.unwrap(), "my-key");
        assert_eq!(config.rust_log, "debug");
    }
}
