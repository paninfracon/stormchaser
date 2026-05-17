use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use cloudevents::EventBuilder;
use k8s_openapi::api::batch::v1::Job;
use kube::{
    api::{Api, ListParams},
    ResourceExt,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use stormchaser_model::events::StepCompletedEvent;
use stormchaser_model::events::StepFailedEvent;
use stormchaser_model::events::{EventSource, EventType, SchemaVersion, StepEventType};
use stormchaser_model::nats::publish_cloudevent;
use stormchaser_model::nats::NatsSubject;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use tracing::{error, info, warn};
use uuid::Uuid;

use stormchaser_model::dsl;

use crate::cluster::ClusterPool;
use crate::job_machine;

/// Builds a serialized CloudEvent payload for use with basic NATS `request`.
fn build_cloudevent_payload(
    event_type: &str,
    source: EventSource,
    data: Value,
) -> anyhow::Result<Vec<u8>> {
    let event = cloudevents::EventBuilderV10::new()
        .id(Uuid::new_v4().to_string())
        .ty(event_type)
        .source(source.as_str())
        .time(chrono::Utc::now())
        .data(stormchaser_model::APPLICATION_JSON, data)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build CloudEvent: {}", e))?;
    Ok(serde_json::to_vec(&event)?)
}

pub fn reconstruct_step(
    job_name: &str,
    is_encrypted: bool,
    encryption_key: Option<&String>,
    raw_step_dsl: Option<&String>,
) -> dsl::Step {
    let fallback = dsl::Step {
        name: job_name.to_string(),
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
        aliases: std::collections::HashMap::new(),
        retry: None,
        timeout: None,
        allow_failure: None,
        start_marker: None,
        end_marker: None,
        outputs: Vec::new(),
        reports: Vec::new(),
        artifacts: None,
    };

    if let Some(raw) = raw_step_dsl {
        let dsl_str = if is_encrypted {
            if let Some(key) = encryption_key {
                match job_machine::crypto::decrypt_state(raw, key) {
                    Ok(decrypted) => decrypted,
                    Err(e) => {
                        error!("Failed to decrypt state for job {}: {:?}", job_name, e);
                        return fallback;
                    }
                }
            } else {
                error!(
                    "Job {} is encrypted but no encryption key is configured",
                    job_name
                );
                return fallback;
            }
        } else {
            raw.clone()
        };

        serde_json::from_str(&dsl_str).unwrap_or(fallback)
    } else {
        fallback
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_orphaned_job(
    run_id_s: &str,
    step_id_s: &str,
    fencing_token: i64,
    job_name: String,
    namespace: String,
    cluster_name: &str,
    job: k8s_openapi::api::batch::v1::Job,
    client: kube::Client,
    cluster_version: String,
    nats_client: async_nats::Client,
    runner_id: String,
    encryption_key: Option<String>,
) {
    let run_id = Uuid::parse_str(run_id_s).unwrap_or_default();
    let step_id = Uuid::parse_str(step_id_s).unwrap_or_default();

    info!(
        "Found orphaned job {} for step {} (run {}) in cluster {}",
        job_name, step_id, run_id, cluster_name
    );

    let annotations = job.metadata.annotations.as_ref();
    let received_at = annotations
        .and_then(|a| a.get("stormchaser.v1.io/received-at"))
        .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(chrono::Utc::now);

    let is_encrypted = annotations
        .and_then(|a| a.get("stormchaser.v1.io/state-encrypted"))
        .map(|v| v == "true")
        .unwrap_or(false);

    let raw_step_dsl = annotations.and_then(|a| a.get("stormchaser.v1.io/step-dsl"));

    let step_dsl = reconstruct_step(
        &job_name,
        is_encrypted,
        encryption_key.as_ref(),
        raw_step_dsl,
    );

    let nats = nats_client.clone();
    let r_id = runner_id.clone();
    let client_clone = client.clone();
    let cv = cluster_version.clone();
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
            .request("stormchaser.v1.step.query", query_ce_payload.into())
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
                        "Step {} is in status {}, skipping adoption and cleaning up job {}",
                        step_id, status, job_name
                    );
                    let machine = job_machine::K8sJobMachine::new(
                        client_clone,
                        job_machine::JobMetadata {
                            run_id,
                            step_id,
                            fencing_token,
                            step_dsl: step_dsl.clone(),
                            namespace,
                            received_at,
                            cluster_version: cv,
                            encryption_key: key_clone,
                            storage: None,
                            test_report_urls: None,
                            registry_auth: None,
                        },
                    );
                    let _ = machine.clean_up(&job_name).await;
                    return;
                }
            }
            Err(e) => {
                error!("Failed to query orchestrator for step {}: {:?}", step_id, e);
                // If we can't reach the orchestrator, we might want to wait or try again later.
                // For now, let's assume it's safer NOT to adopt until we're sure.
                return;
            }
        }

        let metadata = job_machine::JobMetadata {
            run_id,
            step_id,
            fencing_token,
            step_dsl: step_dsl.clone(),
            namespace: namespace.clone(),
            received_at,
            cluster_version: cv.clone(),
            encryption_key: key_clone,
            storage: None,
            test_report_urls: None,
            registry_auth: None,
        };

        let machine = job_machine::K8sJobMachine::new(client_clone, metadata);

        match machine.adopt(job_name.clone()).wait().await {
            Ok(job_state) => match job_state {
                job_machine::JobState::Succeeded(metrics) => {
                    info!("Adopted step {} completed successfully", step_id);
                    let mut outputs = std::collections::HashMap::new();
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
                    let event = StepCompletedEvent {
                        run_id: RunId::new(run_id),
                        step_id: StepInstanceId::new(step_id),
                        fencing_token,
                        event_type: EventType::Step(StepEventType::Completed),
                        runner_id: Some(r_id.clone()),
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
                        &async_nats::jetstream::new(nats.clone()),
                        NatsSubject::StepCompleted(Some(
                            stormchaser_model::nats::compute_shard_id(
                                &stormchaser_model::RunId::new(run_id),
                            ),
                        )),
                        EventType::Step(StepEventType::Completed),
                        EventSource::System,
                        serde_json::to_value(event).unwrap(),
                        Some(SchemaVersion::new("1.0".to_string())),
                        None,
                    )
                    .await;
                }
                job_machine::JobState::Failed(reason, metrics) => {
                    warn!("Adopted step {} failed: {}", step_id, reason);
                    let mut outputs = std::collections::HashMap::new();
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

                    let event = StepFailedEvent {
                        run_id: RunId::new(run_id),
                        step_id: StepInstanceId::new(step_id),
                        fencing_token,
                        event_type: EventType::Step(StepEventType::Failed),
                        error: reason,
                        runner_id: Some(r_id.clone()),
                        exit_code: metrics.exit_code,
                        storage_hashes: None,
                        artifacts: None,
                        test_reports: None,
                        outputs: Some(outputs),
                        timestamp: chrono::Utc::now(),
                    };
                    let _ = publish_cloudevent(
                        &async_nats::jetstream::new(nats.clone()),
                        NatsSubject::StepFailed(Some(stormchaser_model::nats::compute_shard_id(
                            &stormchaser_model::RunId::new(run_id),
                        ))),
                        EventType::Step(StepEventType::Failed),
                        EventSource::System,
                        serde_json::to_value(event).unwrap(),
                        Some(SchemaVersion::new("1.0".to_string())),
                        None,
                    )
                    .await;
                }
            },
            Err(e) => error!("Error adopting job {}: {:?}", job_name, e),
        }
    });
}

pub async fn scan_for_orphans(
    cluster_pool: Arc<ClusterPool>,
    nats_client: async_nats::Client,
    runner_id: String,
    encryption_key: Option<String>,
) -> Result<()> {
    info!("Scanning all clusters for orphaned jobs...");

    for cluster_name in cluster_pool.cluster_names() {
        let (client, cluster_version) = cluster_pool.get_client(&cluster_name).await?;
        let jobs: Api<Job> = Api::all(client.clone());

        let lp = ListParams::default().labels("managed-by=stormchaser");
        let job_list = jobs.list(&lp).await?;

        for job in job_list {
            let job_name = job.name_any();
            let namespace = job.namespace().unwrap_or_else(|| "default".to_string());
            let labels = job.metadata.labels.as_ref().context("Job missing labels")?;

            let run_id_str = labels.get("stormchaser-run-id");
            let step_id_str = labels.get("stormchaser-step-id");
            let fencing_token = labels
                .get("stormchaser-fencing-token")
                .and_then(|token| token.parse::<i64>().ok())
                .unwrap_or(0);

            if let (Some(run_id_s), Some(step_id_s)) = (run_id_str, step_id_str) {
                handle_orphaned_job(
                    run_id_s,
                    step_id_s,
                    fencing_token,
                    job_name,
                    namespace,
                    &cluster_name,
                    job.clone(),
                    client.clone(),
                    cluster_version.clone(),
                    nats_client.clone(),
                    runner_id.clone(),
                    encryption_key.clone(),
                )
                .await;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {

    use super::*;
    use serde_json::json;

    #[test]
    fn test_reconstruct_step_encrypted_no_key() {
        let step = reconstruct_step("test-job", true, None, Some(&"encoded_stuff".to_string()));
        assert_eq!(step.name, "test-job");
    }

    #[test]
    fn test_reconstruct_step_encrypted_invalid_decrypt() {
        let key = "12345678901234567890123456789012".to_string();
        let step = reconstruct_step(
            "test-job",
            true,
            Some(&key),
            Some(&"invalid_base64".to_string()),
        );
        assert_eq!(step.name, "test-job");
    }

    #[test]
    fn test_reconstruct_step_encrypted_valid_decrypt() {
        use crate::job_machine::crypto::encrypt_state;
        let key = "12345678901234567890123456789012".to_string();
        let raw_dsl = json!({
            "name": "test-job",
            "type": "RunContainer",
            "spec": {}
        });
        let encrypted = encrypt_state(&serde_json::to_string(&raw_dsl).unwrap(), &key).unwrap();
        let step = reconstruct_step("test-job", true, Some(&key), Some(&encrypted));
        assert_eq!(step.name, "test-job");
    }

    #[test]
    fn test_build_cloudevent_payload_success() {
        let event_type = "stormchaser.test.event";
        let source = EventSource::System;
        let data = json!({"key": "value"});

        let payload = build_cloudevent_payload(event_type, source, data).unwrap();

        let json_payload: serde_json::Value = serde_json::from_slice(&payload).unwrap();

        assert_eq!(json_payload["type"], "stormchaser.test.event");
        assert_eq!(json_payload["source"], "/stormchaser");
        assert_eq!(json_payload["datacontenttype"], "application/json");
        assert_eq!(json_payload["data"]["key"], "value");
        assert!(json_payload.get("id").is_some());
        assert!(json_payload.get("time").is_some());
    }

    #[test]
    fn test_reconstruct_step_unencrypted_valid_json() {
        let raw_dsl = json!({
            "name": "test-job",
            "type": "RunContainer",
            "spec": {"image": "nginx"},
            "params": {},
            "aggregation": [],
            "next": [],
            "outputs": [],
            "reports": []
        })
        .to_string();

        let step = reconstruct_step("test-job", false, None, Some(&raw_dsl));
        assert_eq!(step.name, "test-job");
        assert_eq!(step.spec, json!({"image": "nginx"}));
    }

    #[test]
    fn test_reconstruct_step_unencrypted_invalid_json() {
        let raw_dsl = "invalid json".to_string();

        let step = reconstruct_step("test-job", false, None, Some(&raw_dsl));
        assert_eq!(step.name, "test-job");
        assert_eq!(step.r#type, "RunContainer");
        assert_eq!(step.spec, Value::Null); // Fallback applied
    }

    #[test]
    fn test_reconstruct_step_unencrypted_invalid_shape() {
        let raw_dsl = json!({"unexpected": "shape"}).to_string();

        let step = reconstruct_step("test-job", false, None, Some(&raw_dsl));
        assert_eq!(step.name, "test-job");
        assert_eq!(step.r#type, "RunContainer");
        assert_eq!(step.spec, Value::Null);
    }

    #[test]
    fn test_reconstruct_step_no_raw_dsl() {
        let step = reconstruct_step("test-job", false, None, None);
        assert_eq!(step.name, "test-job");
        assert_eq!(step.spec, Value::Null);
    }

    #[test]
    fn test_reconstruct_step_encrypted_valid_decrypt_but_invalid_json() {
        use crate::job_machine::crypto::encrypt_state;

        let key = "12345678901234567890123456789012".to_string();
        let encrypted = encrypt_state("not-json", &key).unwrap();
        let step = reconstruct_step("test-job", true, Some(&key), Some(&encrypted));

        assert_eq!(step.name, "test-job");
        assert_eq!(step.r#type, "RunContainer");
        assert_eq!(step.spec, Value::Null);
    }
}
