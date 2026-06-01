use super::events::{build_container_execution_error_event, build_container_result_event};
use crate::container_machine::{ContainerMetadata, ContainerState, DockerContainerMachine};
use crate::parsing::parse_step_from_nats_payload;
use async_nats::jetstream::message::{AckKind, Message};
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
use stormchaser_model::events::{EventSource, EventType, SchemaVersion, StepEventType};
use stormchaser_model::nats::{publish_cloudevent, NatsSubject};
use stormchaser_model::{RunId, StepInstanceId};
use tokio::time::sleep;
use tracing::{error, info};
use uuid::Uuid;

/// Terminates a JetStream message that cannot be processed, preventing redelivery loops.
/// Logs the provided reason before sending the `Term` acknowledgement.
async fn term_message(msg: &Message, reason: &str) {
    error!("{}", reason);
    if let Err(e) = msg.ack_with(AckKind::Term).await {
        error!("Failed to send Term ack: {:?}", e);
    }
}

/// Handles an incoming task message from NATS, starting and managing the lifecycle
/// of a Docker container for the specified step.
pub async fn handle_task(
    msg: async_nats::jetstream::message::Message,
    docker: bollard::Docker,
    nats_client: async_nats::Client,
    runner_id: String,
    encryption_key: Option<String>,
    loki_url: Option<String>,
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

    let initializing_event = stormchaser_model::events::StepInitializingEvent {
        run_id: RunId::new(run_id),
        step_id: StepInstanceId::new(step_id),
        event_type: EventType::Step(StepEventType::Initializing),
        runner_id: Some(runner_id.clone()),
        timestamp: Utc::now(),
    };
    let _ = publish_cloudevent(
        &async_nats::jetstream::new(nats_client.clone()),
        NatsSubject::StepInitializing(Some(stormchaser_model::nats::compute_shard_id(
            &stormchaser_model::RunId::new(run_id),
        ))),
        EventType::Step(StepEventType::Initializing),
        EventSource::System,
        serde_json::to_value(initializing_event).unwrap(),
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await;

    let machine = DockerContainerMachine::new(
        docker,
        ContainerMetadata {
            run_id,
            step_id,
            runner_id: runner_id.clone(),
            fencing_token,
            step_dsl,
            storage,
            test_report_urls,
            registry_auth,
            encryption_key,
            loki_url,
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
            let dispatch = build_container_result_event(
                state,
                run_id,
                step_id,
                fencing_token,
                runner_id.clone(),
            );
            let _ = publish_cloudevent(
                &async_nats::jetstream::new(nats_client.clone()),
                dispatch.subject,
                dispatch.event_type,
                EventSource::System,
                dispatch.payload,
                Some(SchemaVersion::new("1.0".to_string())),
                None,
            )
            .await;
        }
        Err(e) => {
            error!("Error running container for step {}: {:?}", step_id, e);
            let _ = publish_cloudevent(
                &async_nats::jetstream::new(nats_client.clone()),
                NatsSubject::StepFailed(Some(stormchaser_model::nats::compute_shard_id(
                    &stormchaser_model::RunId::new(run_id),
                ))),
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
mod tests {
    use super::*;
    use async_nats::jetstream::stream::Config as StreamConfig;
    use futures::StreamExt;

    #[tokio::test]
    async fn test_handle_task_invalid_event() {
        let docker = bollard::Docker::connect_with_local_defaults().unwrap();
        let nats_client = async_nats::connect("nats://localhost:4222").await;
        if let Ok(nats) = nats_client {
            let js = async_nats::jetstream::new(nats.clone());

            // Create a test stream
            let stream_name = "TEST_TASK_STREAM";
            let subject = "test.task.>";
            let publish_subject = "test.task.foo";
            let _ = js
                .create_stream(StreamConfig {
                    name: stream_name.to_string(),
                    subjects: vec![subject.to_string()],
                    ..Default::default()
                })
                .await;

            // Publish a malformed message
            js.publish(publish_subject, "not a cloud event".into())
                .await
                .unwrap();

            // Get a consumer
            let consumer = js
                .create_consumer_on_stream(
                    async_nats::jetstream::consumer::pull::Config {
                        durable_name: Some("test_task_consumer".to_string()),
                        ..Default::default()
                    },
                    stream_name,
                )
                .await
                .unwrap();

            let mut messages = consumer.messages().await.unwrap();

            if let Some(Ok(msg)) = messages.next().await {
                // Pass it to handle_task
                handle_task(
                    msg,
                    docker.clone(),
                    nats.clone(),
                    "test-runner".to_string(),
                    None,
                    None,
                )
                .await;
            }

            // Clean up
            let _ = js.delete_stream(stream_name).await;
        }
    }
}
