use crate::container_machine::{ContainerMetadata, ContainerState, DockerContainerMachine};
use crate::parsing::{parse_step_from_docker_labels, parse_step_from_nats_payload};
use anyhow::Result;
use bollard::container::ListContainersOptions;
use bollard::Docker;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};
use uuid::Uuid;

/// Scans Docker for containers labeled as managed by Stormchaser but not actively tracked.
/// If a step is no longer relevant to the orchestrator, it is cleaned up.
/// Otherwise, it attempts to adopt the running container and wait for its completion.
pub async fn scan_for_orphans(
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

/// Handles an incoming task message from NATS, starting and managing the lifecycle
/// of a Docker container for the specified step.
pub async fn handle_task(
    msg: async_nats::jetstream::message::Message,
    docker: bollard::Docker,
    nats_client: async_nats::Client,
    runner_id: String,
    encryption_key: Option<String>,
) {
    let received_at = chrono::Utc::now();
    info!("Received task message: {:?}", msg.subject);

    let payload: Value = serde_json::from_slice(&msg.payload).unwrap_or_default();
    let run_id_str = payload["run_id"].as_str().unwrap_or_default();
    let run_id = Uuid::parse_str(run_id_str).unwrap_or_default();
    let step_id_str = payload["step_id"].as_str().unwrap_or_default();
    let step_id = Uuid::parse_str(step_id_str).unwrap_or_default();

    let step_dsl = match parse_step_from_nats_payload(&payload) {
        Ok(step) => step,
        Err(e) => {
            error!("Failed to parse step spec: {:?}", e);
            return;
        }
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
