use super::events::{build_cloudevent_payload, publish_container_result};
use crate::container_machine::{ContainerMetadata, DockerContainerMachine};
use crate::parsing::parse_step_from_docker_labels;
use anyhow::Result;
use bollard::container::ListContainersOptions;
use bollard::Docker;
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;
use stormchaser_model::events::EventSource;
use stormchaser_model::StepInstanceId;
use tracing::{error, info};
use uuid::Uuid;

/// Scans Docker for containers labeled as managed by Stormchaser but not actively tracked.
/// If a step is no longer relevant to the orchestrator, it is cleaned up.
/// Otherwise, it attempts to adopt the running container and wait for its completion.
#[allow(clippy::too_many_arguments)]
async fn handle_orphaned_container(
    run_id_s: &str,
    step_id_s: &str,
    container_id: String,
    container: bollard::models::ContainerSummary,
    docker: Docker,
    nats_client: async_nats::Client,
    runner_id: String,
    encryption_key: Option<String>,
) {
    let run_id = match Uuid::parse_str(run_id_s) {
        Ok(id) => id,
        Err(e) => {
            error!(
                "Invalid run_id label '{}' on container {}: {}",
                run_id_s, container_id, e
            );
            return;
        }
    };
    let step_id = match Uuid::parse_str(step_id_s) {
        Ok(id) => id,
        Err(e) => {
            error!(
                "Invalid step_id label '{}' on container {}: {}",
                step_id_s, container_id, e
            );
            return;
        }
    };

    let container_name = container
        .names
        .and_then(|names| names.first().cloned())
        .unwrap_or_else(|| container_id.clone())
        .replace('/', ""); // Docker names often have leading slash

    info!(
        "Found orphaned container {} for step {} (run {})",
        container_name, step_id, run_id
    );

    let labels = container.labels.unwrap_or_default();

    let received_at = labels
        .get("stormchaser.v1.io/received-at")
        .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(Utc::now);

    let is_encrypted = labels
        .get("stormchaser.v1.io/state-encrypted")
        .map(|v| v == "true")
        .unwrap_or(false);

    let raw_step_dsl = labels.get("stormchaser.v1.io/step-dsl");
    let fencing_token = labels
        .get("stormchaser-fencing-token")
        .and_then(|token| token.parse::<i64>().ok())
        .unwrap_or(0);

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
            return;
        }
    };

    let nats = nats_client.clone();
    let r_id = runner_id.clone();
    let docker_clone = docker.clone();
    let key_clone = encryption_key.clone();

    tokio::spawn(async move {
        // Before adopting, query the orchestrator to see if this step is still relevant
        let query_data = serde_json::to_value(stormchaser_model::events::StepQueryEvent {
            step_id: StepInstanceId::new(step_id),
        })
        .unwrap();
        let query_ce_payload = match build_cloudevent_payload(
            "stormchaser.v1.step.query",
            EventSource::System,
            query_data,
        ) {
            Ok(p) => p,
            Err(e) => {
                error!("Failed to build step query CloudEvent: {:?}", e);
                return;
            }
        };

        match nats
            .request("stormchaser.v1.step.query", query_ce_payload)
            .await
        {
            Ok(reply) => {
                let ce: cloudevents::Event = match serde_json::from_slice(&reply.payload) {
                    Ok(e) => e,
                    Err(e) => {
                        error!("Failed to parse step query response as CloudEvent: {:?}", e);
                        return;
                    }
                };
                let response: Value = match ce.data() {
                    Some(cloudevents::Data::Json(v)) => v.clone(),
                    _ => {
                        error!("Step query response CloudEvent has no JSON data");
                        return;
                    }
                };
                let status = response["status"].as_str().unwrap_or_default();
                let exists = response["exists"].as_bool().unwrap_or(false);

                if !exists || (status != "pending" && status != "running") {
                    info!(
                        "Step {} is in status {}, skipping adoption and cleaning up container {}",
                        step_id, status, container_name
                    );
                    let machine = DockerContainerMachine::new(
                        docker_clone,
                        ContainerMetadata {
                            run_id,
                            step_id,
                            fencing_token,
                            step_dsl,
                            storage: None,
                            test_report_urls: None,
                            registry_auth: None,
                            encryption_key: key_clone,
                            loki_url: None,
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
            fencing_token,
            step_dsl: step_dsl.clone(),
            storage: None,
            test_report_urls: None,
            registry_auth: None,
            encryption_key: key_clone,
            loki_url: None,
            received_at,
        };

        let machine = DockerContainerMachine::new(docker_clone, metadata, Some(nats.clone()));

        match machine.adopt(container_name.clone()).wait().await {
            Ok(finished_machine) => {
                publish_container_result(
                    finished_machine.into_result(),
                    run_id,
                    step_id,
                    fencing_token,
                    r_id,
                    nats.clone(),
                )
                .await;
            }
            Err(e) => error!("Error adopting container {}: {:?}", container_name, e),
        }
    });
}

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
        let container_id = container.id.clone().unwrap_or_default();
        let labels = container.labels.clone().unwrap_or_default();

        let run_id_str = labels.get("stormchaser-run-id");
        let step_id_str = labels.get("stormchaser-step-id");

        if let (Some(run_id_s), Some(step_id_s)) = (run_id_str, step_id_str) {
            handle_orphaned_container(
                run_id_s,
                step_id_s,
                container_id,
                container,
                docker.clone(),
                nats_client.clone(),
                runner_id.clone(),
                encryption_key.clone(),
            )
            .await;
        }
    }

    Ok(())
}
