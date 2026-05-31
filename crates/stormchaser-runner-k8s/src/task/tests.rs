use super::*;

use crate::job_machine::JobMetrics;
use serde_json::json;
use stormchaser_model::nats::NatsSubject;
use uuid::Uuid;

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

#[test]
fn test_fallback_step_defaults_when_fields_missing_or_invalid() {
    let payload = json!({
        "step_name": null,
        "step_type": null,
        "params": "invalid"
    });
    let step = fallback_step(&payload, Value::Null);
    assert_eq!(step.name, "");
    assert_eq!(step.r#type, "");
    assert!(step.params.is_empty());
}

#[test]
fn test_build_job_result_event_success() {
    let run_id = Uuid::new_v4();
    let step_id = Uuid::new_v4();
    let metrics = JobMetrics {
        exit_code: Some(0),
        attempts: 2,
        duration_ms: 500,
        latency_ms: 12,
        storage_hashes: Some(HashMap::from([("out".to_string(), "abc".to_string())])),
        artifacts: Some(HashMap::from([("file".to_string(), json!("artifact.txt"))])),
        test_reports: Some(HashMap::from([(
            "junit".to_string(),
            json!({"url": "https://paninfracon.net/junit.xml"}),
        )])),
    };

    let dispatch = build_job_result_event(
        job_machine::JobState::Succeeded(metrics),
        run_id,
        step_id,
        0,
        "runner-k8s".to_string(),
    );

    assert_eq!(
        dispatch.subject,
        NatsSubject::StepCompleted(Some(stormchaser_model::nats::compute_shard_id(
            &stormchaser_model::RunId::new(run_id)
        )))
    );
    assert_eq!(
        dispatch.event_type,
        EventType::Step(StepEventType::Completed)
    );
    let event = dispatch.payload;
    assert_eq!(event["run_id"], run_id.to_string());
    assert_eq!(event["step_id"], step_id.to_string());
    assert_eq!(event["outputs"]["Number of attempts"], 2);
    assert_eq!(
        event["test_reports"]["junit"]["url"],
        "https://paninfracon.net/junit.xml"
    );
}

#[test]
fn test_build_job_result_event_failed() {
    let run_id = Uuid::new_v4();
    let step_id = Uuid::new_v4();
    let metrics = JobMetrics {
        exit_code: Some(1),
        attempts: 1,
        duration_ms: 10,
        latency_ms: 3,
        storage_hashes: None,
        artifacts: None,
        test_reports: None,
    };

    let dispatch = build_job_result_event(
        job_machine::JobState::Failed("boom".to_string(), metrics),
        run_id,
        step_id,
        0,
        "runner-k8s".to_string(),
    );

    assert_eq!(
        dispatch.subject,
        NatsSubject::StepFailed(Some(stormchaser_model::nats::compute_shard_id(
            &stormchaser_model::RunId::new(run_id)
        )))
    );
    assert_eq!(dispatch.event_type, EventType::Step(StepEventType::Failed));
    let event = dispatch.payload;
    assert_eq!(event["error"], "boom");
    assert_eq!(event["outputs"]["run latency"], "3ms");
}

#[test]
fn test_build_job_error_event() {
    let run_id = Uuid::new_v4();
    let step_id = Uuid::new_v4();
    let event = build_job_error_event(
        run_id,
        step_id,
        0,
        "runner-k8s".to_string(),
        &anyhow::anyhow!("job exploded"),
    );

    assert_eq!(event["run_id"], run_id.to_string());
    assert_eq!(event["step_id"], step_id.to_string());
    assert!(event["exit_code"].is_null());
    assert!(event["outputs"].is_null());
    assert!(event["error"]
        .as_str()
        .unwrap_or_default()
        .contains("job exploded"));
}
