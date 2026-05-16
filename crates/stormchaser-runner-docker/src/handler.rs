use crate::container_machine::{ContainerMetadata, ContainerState, DockerContainerMachine};
use crate::parsing::{parse_step_from_docker_labels, parse_step_from_nats_payload};
use anyhow::Result;
use async_nats::jetstream::message::{AckKind, Message};
use bollard::container::ListContainersOptions;
use bollard::Docker;
use chrono::Utc;
use cloudevents::EventBuilder;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
use stormchaser_model::events::StepCompletedEvent;
use stormchaser_model::events::StepFailedEvent;
use stormchaser_model::events::{
    EventSource, EventType, SchemaVersion, StepEventType, StepRunningEvent,
};
use stormchaser_model::nats::publish_cloudevent;
use stormchaser_model::nats::NatsSubject;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_model::APPLICATION_JSON;
use tokio::time::sleep;
use tracing::{error, info, warn};
use uuid::Uuid;

/// Terminates a JetStream message that cannot be processed, preventing redelivery loops.
/// Logs the provided reason before sending the `Term` acknowledgement.
async fn term_message(msg: &Message, reason: &str) {
    error!("{}", reason);
    if let Err(e) = msg.ack_with(AckKind::Term).await {
        error!("Failed to send Term ack: {:?}", e);
    }
}

/// Builds a serialized CloudEvent payload for use with basic NATS `request`.
fn build_cloudevent_payload(
    event_type: &str,
    source: EventSource,
    data: Value,
) -> anyhow::Result<bytes::Bytes> {
    let event = cloudevents::EventBuilderV10::new()
        .id(Uuid::new_v4().to_string())
        .ty(event_type)
        .source(source.as_str())
        .time(Utc::now())
        .data(APPLICATION_JSON, data)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build CloudEvent: {}", e))?;
    let payload = serde_json::to_string(&event)?;
    Ok(payload.into())
}

fn normalize_test_reports(test_reports: Option<Value>) -> Option<HashMap<String, Value>> {
    test_reports.and_then(|value| {
        value
            .as_object()
            .cloned()
            .map(|obj| obj.into_iter().collect())
    })
}

fn build_container_outputs(
    metrics: &crate::container_machine::ContainerMetrics,
) -> HashMap<String, Value> {
    let mut outputs = HashMap::new();
    outputs.insert(
        "docker exit code".to_string(),
        serde_json::json!(metrics.exit_code),
    );
    outputs.insert(
        "run duration".to_string(),
        serde_json::json!(format!("{}ms", metrics.duration_ms)),
    );
    outputs.insert(
        "run latency".to_string(),
        serde_json::json!(format!("{}ms", metrics.latency_ms)),
    );
    outputs
}

fn build_container_result_event(
    state: ContainerState,
    run_id: Uuid,
    step_id: Uuid,
    fencing_token: i64,
    runner_id: String,
) -> (NatsSubject, EventType, Value) {
    match state {
        ContainerState::Succeeded(metrics) => {
            let outputs = build_container_outputs(&metrics);
            let event_type = EventType::Step(StepEventType::Completed);
            let event = StepCompletedEvent {
                run_id: RunId::new(run_id),
                step_id: StepInstanceId::new(step_id),
                fencing_token,
                event_type: event_type.clone(),
                runner_id: Some(runner_id),
                exit_code: metrics.exit_code.map(|c| c as i32),
                storage_hashes: metrics.storage_hashes.map(|h| {
                    h.into_iter()
                        .map(|(k, v)| (k, serde_json::json!(v)))
                        .collect()
                }),
                artifacts: metrics.artifacts,
                test_reports: normalize_test_reports(metrics.test_reports),
                outputs: Some(outputs),
                timestamp: Utc::now(),
            };
            (
                NatsSubject::StepCompleted,
                event_type,
                serde_json::to_value(event).unwrap(),
            )
        }
        ContainerState::Failed(reason, metrics) => {
            let outputs = build_container_outputs(&metrics);
            let event_type = EventType::Step(StepEventType::Failed);
            let event = StepFailedEvent {
                run_id: RunId::new(run_id),
                step_id: StepInstanceId::new(step_id),
                fencing_token,
                event_type: event_type.clone(),
                error: reason,
                runner_id: Some(runner_id),
                exit_code: metrics.exit_code.map(|c| c as i32),
                storage_hashes: metrics.storage_hashes.map(|h| {
                    h.into_iter()
                        .map(|(k, v)| (k, serde_json::json!(v)))
                        .collect()
                }),
                artifacts: metrics.artifacts,
                test_reports: normalize_test_reports(metrics.test_reports),
                outputs: Some(outputs),
                timestamp: Utc::now(),
            };
            (
                NatsSubject::StepFailed,
                event_type,
                serde_json::to_value(event).unwrap(),
            )
        }
    }
}

fn build_container_execution_error_event(
    run_id: Uuid,
    step_id: Uuid,
    fencing_token: i64,
    runner_id: String,
    error: &anyhow::Error,
) -> Value {
    serde_json::to_value(StepFailedEvent {
        run_id: RunId::new(run_id),
        step_id: StepInstanceId::new(step_id),
        fencing_token,
        event_type: EventType::Step(StepEventType::Failed),
        error: format!("{:?}", error),
        runner_id: Some(runner_id),
        exit_code: None,
        storage_hashes: None,
        artifacts: None,
        test_reports: None,
        outputs: None,
        timestamp: Utc::now(),
    })
    .unwrap()
}

async fn publish_container_result(
    state: ContainerState,
    run_id: Uuid,
    step_id: Uuid,
    fencing_token: i64,
    runner_id: String,
    nats: async_nats::Client,
) {
    match &state {
        ContainerState::Succeeded(_) => info!("Adopted step {} completed successfully", step_id),
        ContainerState::Failed(reason, _) => warn!("Adopted step {} failed: {}", step_id, reason),
    }
    let (subject, event_type, event_value) =
        build_container_result_event(state, run_id, step_id, fencing_token, runner_id);

    let _ = publish_cloudevent(
        &async_nats::jetstream::new(nats),
        subject,
        event_type,
        EventSource::System,
        event_value,
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await;
}

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
                            fencing_token: 0,
                            step_dsl,
                            storage: None,
                            test_report_urls: None,
                            registry_auth: None,
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
            fencing_token: 0,
            step_dsl: step_dsl.clone(),
            storage: None,
            test_report_urls: None,
            registry_auth: None,
            encryption_key: key_clone,
            received_at,
        };

        let machine = DockerContainerMachine::new(docker_clone, metadata, Some(nats.clone()));

        match machine.adopt(container_name.clone()).wait().await {
            Ok(finished_machine) => {
                publish_container_result(
                    finished_machine.into_result(),
                    run_id,
                    step_id,
                    0, // Fencing token
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
/// Handles an incoming task message from NATS, starting and managing the lifecycle
/// of a Docker container for the specified step.
pub async fn handle_task(
    msg: async_nats::jetstream::message::Message,
    docker: bollard::Docker,
    nats_client: async_nats::Client,
    runner_id: String,
    encryption_key: Option<String>,
) {
    let received_at = Utc::now();
    info!("Received task message: {:?}", msg.subject);

    let ce: cloudevents::Event = match serde_json::from_slice(&msg.payload) {
        Ok(event) => event,
        Err(e) => {
            term_message(
                &msg,
                &format!(
                    "Failed to deserialize CloudEvent from task message: {:?}",
                    e
                ),
            )
            .await;
            return;
        }
    };
    let payload: Value = match ce.data() {
        Some(cloudevents::Data::Json(v)) => v.clone(),
        _ => {
            term_message(&msg, "Task message CloudEvent does not contain JSON data").await;
            return;
        }
    };

    let run_id = match payload["run_id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
    {
        Some(id) => id,
        None => {
            term_message(&msg, "Task message missing or invalid run_id").await;
            return;
        }
    };
    let step_id = match payload["step_id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
    {
        Some(id) => id,
        None => {
            term_message(&msg, "Task message missing or invalid step_id").await;
            return;
        }
    };

    let step_dsl = match parse_step_from_nats_payload(&payload) {
        Ok(step) => step,
        Err(e) => {
            term_message(&msg, &format!("Failed to parse step spec: {:?}", e)).await;
            return;
        }
    };

    let storage: Option<HashMap<String, Value>> =
        serde_json::from_value(payload["storage"].clone()).ok();
    let test_report_urls: Option<HashMap<String, Value>> =
        serde_json::from_value(payload["test_report_urls"].clone()).ok();

    let registry_auth: Option<Value> =
        serde_json::from_value(payload["registry_auth"].clone()).ok();
    let fencing_token: i64 = payload["fencing_token"].as_i64().unwrap_or(0);

    let in_progress_msg = msg.clone();
    let in_progress_handle = tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(15)).await;
            let _ = in_progress_msg.ack_with(AckKind::Progress).await;
        }
    });

    let running_event = StepRunningEvent {
        run_id: RunId::new(run_id),
        step_id: StepInstanceId::new(step_id),
        event_type: EventType::Step(StepEventType::Running),
        runner_id: Some(runner_id.clone()),
        timestamp: Utc::now(),
    };
    let _ = publish_cloudevent(
        &async_nats::jetstream::new(nats_client.clone()),
        NatsSubject::StepRunning,
        EventType::Step(StepEventType::Running),
        EventSource::System,
        serde_json::to_value(running_event).unwrap(),
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await;

    let machine = DockerContainerMachine::new(
        docker,
        ContainerMetadata {
            run_id,
            step_id,
            fencing_token: 0,
            step_dsl,
            storage,
            test_report_urls,
            registry_auth,
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
        Ok(state) => {
            match &state {
                ContainerState::Succeeded(_) => {
                    info!("Step {} (Run {}) completed successfully", step_id, run_id)
                }
                ContainerState::Failed(reason, _) => {
                    error!("Step {} (Run {}) failed: {}", step_id, run_id, reason)
                }
            }
            let (subject, event_type, event) = build_container_result_event(
                state,
                run_id,
                step_id,
                fencing_token,
                runner_id.clone(),
            );
            let _ = publish_cloudevent(
                &async_nats::jetstream::new(nats_client.clone()),
                subject,
                event_type,
                EventSource::System,
                event,
                Some(SchemaVersion::new("1.0".to_string())),
                None,
            )
            .await;
        }
        Err(e) => {
            error!("Error running container for step {}: {:?}", step_id, e);
            let _ = publish_cloudevent(
                &async_nats::jetstream::new(nats_client.clone()),
                NatsSubject::StepFailed,
                EventType::Step(StepEventType::Failed),
                EventSource::System,
                build_container_execution_error_event(
                    run_id,
                    step_id,
                    fencing_token,
                    runner_id.clone(),
                    &e,
                ),
                Some(SchemaVersion::new("1.0".to_string())),
                None,
            )
            .await;
        }
    }
}

#[cfg(test)]
mod tests_handler_ext {
    use super::*;
    use crate::container_machine::ContainerMetrics;
    use serde_json::json;
    use stormchaser_model::events::EventSource;
    use stormchaser_model::nats::NatsSubject;
    use uuid::Uuid;

    #[test]
    fn test_build_cloudevent_payload_success() {
        let event_type = "stormchaser.test.event";
        let source = EventSource::System;
        let data = json!({"key": "value"});

        let payload = build_cloudevent_payload(event_type, source, data).unwrap();

        // Deserialize back
        let json_payload: serde_json::Value = serde_json::from_slice(&payload).unwrap();

        assert_eq!(json_payload["type"], "stormchaser.test.event");
        assert_eq!(json_payload["source"], "/stormchaser");
        assert_eq!(json_payload["datacontenttype"], "application/json");
        assert_eq!(json_payload["data"]["key"], "value");
        assert!(json_payload.get("id").is_some());
        assert!(json_payload.get("time").is_some());
    }

    #[test]
    fn test_build_container_result_event_success_normalizes_test_reports() {
        let run_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();
        let metrics = ContainerMetrics {
            exit_code: Some(0),
            duration_ms: 1200,
            latency_ms: 25,
            storage_hashes: Some(HashMap::from([("out".to_string(), "abc".to_string())])),
            artifacts: Some(HashMap::from([("logs".to_string(), json!(["a.txt"]))])),
            test_reports: Some(json!({"junit": {"url": "https://paninfracon.net/report.xml"}})),
        };

        let (subject, event_type, event) = build_container_result_event(
            ContainerState::Succeeded(metrics),
            run_id,
            step_id,
            0,
            "runner-1".to_string(),
        );

        assert_eq!(subject, NatsSubject::StepCompleted);
        assert_eq!(event_type, EventType::Step(StepEventType::Completed));
        assert_eq!(event["run_id"], run_id.to_string());
        assert_eq!(event["step_id"], step_id.to_string());
        assert_eq!(event["runner_id"], "runner-1");
        assert_eq!(
            event["test_reports"]["junit"]["url"],
            "https://paninfracon.net/report.xml"
        );
        assert_eq!(event["outputs"]["run duration"], "1200ms");
    }

    #[test]
    fn test_build_container_result_event_failed_drops_non_object_test_reports() {
        let run_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();
        let metrics = ContainerMetrics {
            exit_code: Some(42),
            duration_ms: 5,
            latency_ms: 1,
            storage_hashes: None,
            artifacts: None,
            test_reports: Some(json!("not-an-object")),
        };

        let (subject, event_type, event) = build_container_result_event(
            ContainerState::Failed("boom".to_string(), metrics),
            run_id,
            step_id,
            0,
            "runner-2".to_string(),
        );

        assert_eq!(subject, NatsSubject::StepFailed);
        assert_eq!(event_type, EventType::Step(StepEventType::Failed));
        assert_eq!(event["error"], "boom");
        assert!(event["test_reports"].is_null());
        assert_eq!(event["outputs"]["run latency"], "1ms");
    }

    #[test]
    fn test_build_container_execution_error_event_sets_empty_metrics() {
        let run_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();
        let event = build_container_execution_error_event(
            run_id,
            step_id,
            0,
            "runner-3".to_string(),
            &anyhow::anyhow!("execution failed"),
        );

        assert_eq!(event["run_id"], run_id.to_string());
        assert_eq!(event["step_id"], step_id.to_string());
        assert_eq!(event["runner_id"], "runner-3");
        assert!(event["exit_code"].is_null());
        assert!(event["outputs"].is_null());
        assert!(event["error"]
            .as_str()
            .unwrap_or_default()
            .contains("execution failed"));
    }

    #[tokio::test]
    #[ignore]
    async fn test_handle_orphaned_container_compiles() {
        let _f = handle_orphaned_container;
    }

    #[tokio::test]
    #[ignore]
    async fn test_publish_container_result_compiles() {
        let _f = publish_container_result;
    }
}
