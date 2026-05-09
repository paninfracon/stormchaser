use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_model::dsl::Step;
use stormchaser_model::events::StepCompletedEvent;
use stormchaser_model::events::StepFailedEvent;
use stormchaser_model::events::{EventSource, EventType, SchemaVersion, StepEventType};
use stormchaser_model::nats::publish_cloudevent;
use stormchaser_model::nats::NatsSubject;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use tokio::time::sleep;
use uuid::Uuid;

use stormchaser_model::dsl;

use crate::cluster::ClusterPool;
use crate::job_machine;

pub fn fallback_step(payload: &Value, spec: serde_json::Value) -> Step {
    Step {
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
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_job_on_cluster(
    client: kube::Client,
    cluster_version: String,
    run_id: Uuid,
    step_id: Uuid,
    step_dsl: dsl::Step,
    storage: Option<HashMap<String, Value>>,
    test_report_urls: Option<HashMap<String, Value>>,
    runner_id: String,
    nats_client: async_nats::Client,
    encryption_key: Option<String>,
    received_at: chrono::DateTime<chrono::Utc>,
    in_progress_handle: tokio::task::JoinHandle<()>,
    msg: async_nats::jetstream::message::Message,
) {
    let namespace = std::env::var("KUBERNETES_NAMESPACE").unwrap_or_else(|_| "default".to_string());
    let metadata = job_machine::JobMetadata {
        run_id,
        step_id,
        step_dsl,
        namespace,
        received_at,
        cluster_version,
        encryption_key,
        storage,
        test_report_urls,
    };

    let machine = job_machine::K8sJobMachine::new(client.clone(), metadata.clone());

    let result = match machine.start().await {
        Ok(job_machine::StartResult::Running(running_machine)) => running_machine.wait().await,
        Ok(job_machine::StartResult::Failed(finished_machine)) => {
            Ok(finished_machine.into_result())
        }
        Err(e) => Err(e),
    };
    in_progress_handle.abort();
    let _ = msg.double_ack().await;
    publish_job_result(nats_client, run_id, step_id, runner_id, result).await;
}

async fn publish_job_result(
    nats_client: async_nats::Client,
    run_id: Uuid,
    step_id: Uuid,
    runner_id: String,
    result: Result<job_machine::JobState, anyhow::Error>,
) {
    match result {
        Ok(job_machine::JobState::Succeeded(metrics)) => {
            tracing::info!("Step {} (Run {}) completed successfully", step_id, run_id);
            let mut outputs = HashMap::new();
            outputs.insert(
                "k8s exit code".to_string(),
                serde_json::json!(metrics.exit_code),
            );
            outputs.insert(
                "Number of attempts".to_string(),
                serde_json::json!(metrics.attempts),
            );
            outputs.insert(
                "run duration".to_string(),
                serde_json::json!(format!("{}ms", metrics.duration_ms)),
            );
            outputs.insert(
                "run latency".to_string(),
                serde_json::json!(format!("{}ms", metrics.latency_ms)),
            );
            let complete_event = StepCompletedEvent {
                run_id: RunId::new(run_id),
                step_id: StepInstanceId::new(step_id),
                event_type: EventType::Step(StepEventType::Completed),
                runner_id: Some(runner_id.clone()),
                exit_code: metrics.exit_code,
                storage_hashes: metrics.storage_hashes.map(|h| {
                    h.into_iter()
                        .map(|(k, v)| (k, serde_json::json!(v)))
                        .collect()
                }),
                artifacts: metrics.artifacts,
                test_reports: metrics.test_reports,
                outputs: Some(outputs),
                timestamp: chrono::Utc::now(),
            };
            let _ = publish_cloudevent(
                &async_nats::jetstream::new(nats_client.clone()),
                NatsSubject::StepCompleted,
                EventType::Step(StepEventType::Completed),
                EventSource::System,
                serde_json::to_value(complete_event).unwrap(),
                Some(SchemaVersion::new("1.0".to_string())),
                None,
            )
            .await;
        }
        Ok(job_machine::JobState::Failed(reason, metrics)) => {
            tracing::error!("Step {} (Run {}) failed: {}", step_id, run_id, reason);
            let mut outputs = HashMap::new();
            outputs.insert(
                "k8s exit code".to_string(),
                serde_json::json!(metrics.exit_code),
            );
            outputs.insert(
                "Number of attempts".to_string(),
                serde_json::json!(metrics.attempts),
            );
            outputs.insert(
                "run duration".to_string(),
                serde_json::json!(format!("{}ms", metrics.duration_ms)),
            );
            outputs.insert(
                "run latency".to_string(),
                serde_json::json!(format!("{}ms", metrics.latency_ms)),
            );
            let fail_event = StepFailedEvent {
                run_id: RunId::new(run_id),
                step_id: StepInstanceId::new(step_id),
                event_type: EventType::Step(StepEventType::Failed),
                error: reason,
                runner_id: Some(runner_id.clone()),
                exit_code: metrics.exit_code,
                storage_hashes: metrics.storage_hashes.map(|h| {
                    h.into_iter()
                        .map(|(k, v)| (k, serde_json::json!(v)))
                        .collect()
                }),
                artifacts: metrics.artifacts,
                test_reports: metrics.test_reports,
                outputs: Some(outputs),
                timestamp: chrono::Utc::now(),
            };
            let _ = publish_cloudevent(
                &async_nats::jetstream::new(nats_client.clone()),
                NatsSubject::StepFailed,
                EventType::Step(StepEventType::Failed),
                EventSource::System,
                serde_json::to_value(fail_event).unwrap(),
                Some(SchemaVersion::new("1.0".to_string())),
                None,
            )
            .await;
        }
        Err(e) => {
            tracing::error!("Error running K8s job for step {}: {:?}", step_id, e);
            let fail_event = StepFailedEvent {
                run_id: RunId::new(run_id),
                step_id: StepInstanceId::new(step_id),
                event_type: EventType::Step(StepEventType::Failed),
                error: format!("{:?}", e),
                runner_id: Some(runner_id.clone()),
                exit_code: None,
                storage_hashes: None,
                artifacts: None,
                test_reports: None,
                outputs: None,
                timestamp: chrono::Utc::now(),
            };
            let _ = publish_cloudevent(
                &async_nats::jetstream::new(nats_client.clone()),
                NatsSubject::StepFailed,
                EventType::Step(StepEventType::Failed),
                EventSource::System,
                serde_json::to_value(fail_event).unwrap(),
                Some(SchemaVersion::new("1.0".to_string())),
                None,
            )
            .await;
        }
    }
}

pub async fn handle_task(
    msg: async_nats::jetstream::message::Message,
    cluster_pool: Arc<ClusterPool>,
    nats_client: async_nats::Client,
    runner_id: String,
    encryption_key: Option<String>,
) {
    let received_at = chrono::Utc::now();
    tracing::info!("Received task message: {:?}", msg.subject);

    let ce: cloudevents::Event = match serde_json::from_slice(&msg.payload) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Failed to parse CloudEvent payload: {:?}", e);
            if let Err(ack_err) = msg.double_ack().await {
                tracing::warn!("Failed to ack message after parsing error: {:?}", ack_err);
            }
            return;
        }
    };
    let payload: Value = if let Some(cloudevents::Data::Json(v)) = ce.data() {
        v.clone()
    } else {
        tracing::error!("CloudEvent data is not JSON");
        if let Err(ack_err) = msg.ack().await {
            tracing::warn!("Failed to ack unparseable task message: {:?}", ack_err);
        }
        return;
    };

    let run_id_str = payload["run_id"].as_str().unwrap_or_default();
    let run_id = match Uuid::parse_str(run_id_str) {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("Invalid run_id '{}' in task message: {:?}", run_id_str, e);
            if let Err(ack_err) = msg.ack().await {
                tracing::warn!("Failed to ack invalid-run_id task message: {:?}", ack_err);
            }
            return;
        }
    };

    let step_id_str = payload["step_id"].as_str().unwrap_or_default();
    let step_id = match Uuid::parse_str(step_id_str) {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("Invalid step_id '{}' in task message: {:?}", step_id_str, e);
            if let Err(ack_err) = msg.ack().await {
                tracing::warn!("Failed to ack invalid-step_id task message: {:?}", ack_err);
            }
            return;
        }
    };

    let spec = serde_json::from_value(payload["spec"].clone()).unwrap_or(serde_json::Value::Null);

    let step_dsl: dsl::Step = if let Some(dsl_val) = payload.get("step_dsl") {
        if !dsl_val.is_null() {
            if let Ok(mut step) = serde_json::from_value::<dsl::Step>(dsl_val.clone()) {
                step.spec = spec;
                step
            } else {
                fallback_step(&payload, spec)
            }
        } else {
            fallback_step(&payload, spec)
        }
    } else {
        fallback_step(&payload, spec)
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

    // Notify orchestrator that we are starting
    let running_event = stormchaser_model::events::StepRunningEvent {
        run_id: RunId::new(run_id),
        step_id: StepInstanceId::new(step_id),
        event_type: EventType::Step(StepEventType::Running),
        runner_id: Some(runner_id.clone()),
        timestamp: chrono::Utc::now(),
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

    let target_cluster = "local"; // In future, get from affinity/params
    match cluster_pool.get_client(target_cluster).await {
        Ok((client, cluster_version)) => {
            execute_job_on_cluster(
                client,
                cluster_version,
                run_id,
                step_id,
                step_dsl,
                storage,
                test_report_urls,
                runner_id,
                nats_client,
                encryption_key,
                received_at,
                in_progress_handle,
                msg,
            )
            .await;
        }
        Err(e) => {
            in_progress_handle.abort();
            tracing::error!("Failed to acquire K8s client: {:?}", e);
            let fail_event = StepFailedEvent {
                run_id: RunId::new(run_id),
                step_id: StepInstanceId::new(step_id),
                event_type: EventType::Step(StepEventType::Failed),
                error: format!("Failed to acquire K8s client: {:?}", e),
                runner_id: Some(runner_id.clone()),
                exit_code: None,
                storage_hashes: None,
                artifacts: None,
                test_reports: None,
                outputs: None,
                timestamp: chrono::Utc::now(),
            };
            let _ = publish_cloudevent(
                &async_nats::jetstream::new(nats_client.clone()),
                NatsSubject::StepFailed,
                EventType::Step(StepEventType::Failed),
                EventSource::System,
                serde_json::to_value(fail_event).unwrap(),
                Some(SchemaVersion::new("1.0".to_string())),
                None,
            )
            .await;
            if let Err(ack_err) = msg.double_ack().await {
                tracing::warn!(
                    "Failed to ack task message after client acquisition failure: {:?}",
                    ack_err
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {

    #[tokio::test]
    #[ignore]
    async fn test_execute_job_on_cluster_compiles() {
        let _f = execute_job_on_cluster;
    }

    use super::*;
    use serde_json::json;

    #[test]
    fn test_fallback_step() {
        let payload = json!({
            "step_name": "test-step",
            "step_type": "K8sJob",
            "params": {
                "image": "nginx"
            }
        });
        let spec = json!({"parallelism": 2});

        let step = fallback_step(&payload, spec.clone());
        assert_eq!(step.name, "test-step");
        assert_eq!(step.r#type, "K8sJob");
        assert_eq!(step.spec, spec);
        assert_eq!(step.params.get("image").unwrap(), "nginx");
    }
}
