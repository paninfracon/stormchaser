use crate::container_machine::ContainerState;
use chrono::Utc;
use cloudevents::EventBuilder;
use serde_json::Value;
use std::collections::HashMap;
use stormchaser_model::events::{
    EventSource, EventType, SchemaVersion, StepCompletedEvent, StepEventType, StepFailedEvent,
};
use stormchaser_model::nats::{publish_cloudevent, NatsSubject};
use stormchaser_model::{RunId, StepInstanceId, APPLICATION_JSON};
use tracing::{info, warn};
use uuid::Uuid;

/// Builds a serialized CloudEvent payload for use with basic NATS `request`.
pub(crate) fn build_cloudevent_payload(
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

pub(crate) fn build_container_result_event(
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
                NatsSubject::StepCompleted(Some(stormchaser_model::nats::compute_shard_id(
                    &stormchaser_model::RunId::new(run_id),
                ))),
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
                NatsSubject::StepFailed(Some(stormchaser_model::nats::compute_shard_id(
                    &stormchaser_model::RunId::new(run_id),
                ))),
                event_type,
                serde_json::to_value(event).unwrap(),
            )
        }
    }
}

pub(crate) fn build_container_execution_error_event(
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

pub(crate) async fn publish_container_result(
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

        assert_eq!(
            subject,
            NatsSubject::StepCompleted(Some(stormchaser_model::nats::compute_shard_id(
                &stormchaser_model::RunId::new(run_id)
            )))
        );
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

        assert_eq!(
            subject,
            NatsSubject::StepFailed(Some(stormchaser_model::nats::compute_shard_id(
                &stormchaser_model::RunId::new(run_id)
            )))
        );
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
}
