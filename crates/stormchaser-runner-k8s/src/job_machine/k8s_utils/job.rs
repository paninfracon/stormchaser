use crate::job_machine::{crypto, JobMetadata};
use anyhow::Result;
use k8s_openapi::api::batch::v1::{Job, JobSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use super::*;

pub fn do_build_job_spec(
    job_name: &str,
    metadata: &JobMetadata,
    agent_image: Option<String>,
    sfs_pvc_name: Option<String>,
) -> Result<Job> {
    let step_spec = parse_step_spec(metadata)?;
    StepPolicy::from_env().check(&step_spec)?;

    let (volumes, volume_mounts) = build_k8s_volumes(
        agent_image.is_some(),
        sfs_pvc_name.as_deref(),
        metadata,
        &step_spec,
    );

    let k8s_env = build_k8s_env_vars(metadata, &step_spec, sfs_pvc_name.as_deref());

    let (container, init_containers) = build_k8s_containers(
        agent_image,
        sfs_pvc_name.as_deref(),
        metadata,
        &step_spec,
        k8s_env,
        volume_mounts,
    );

    let pod_template =
        build_k8s_pod_spec(container, init_containers, volumes, metadata, &step_spec);

    // Job Labels. Author-supplied entries go in first so they can never replace the
    // ones Stormchaser relies on to find, attribute and reap its jobs.
    let mut labels = step_spec.extra_labels.unwrap_or_default();
    labels.extend([
        ("managed-by".to_string(), "stormchaser".to_string()),
        (
            "stormchaser-run-id".to_string(),
            metadata.run_id.to_string(),
        ),
        (
            "stormchaser-step-id".to_string(),
            metadata.step_id.to_string(),
        ),
        (
            "stormchaser-fencing-token".to_string(),
            metadata.fencing_token.to_string(),
        ),
    ]);

    // Job Annotations
    let step_dsl_json = serde_json::to_string(&metadata.step_dsl).unwrap_or_default();
    let step_dsl_val = if let Some(key) = &metadata.encryption_key {
        crypto::encrypt_state(&step_dsl_json, key)?
    } else {
        step_dsl_json
    };

    let mut annotations = step_spec.extra_annotations.unwrap_or_default();
    annotations.extend([
        (
            "stormchaser.v1.io/received-at".to_string(),
            metadata.received_at.to_rfc3339(),
        ),
        ("stormchaser.v1.io/step-dsl".to_string(), step_dsl_val),
    ]);

    if metadata.encryption_key.is_some() {
        annotations.insert(
            "stormchaser.v1.io/state-encrypted".to_string(),
            "true".to_string(),
        );
    } else {
        annotations.remove("stormchaser.v1.io/state-encrypted");
    }

    Ok(Job {
        metadata: ObjectMeta {
            name: Some(job_name.to_string()),
            labels: Some(labels),
            annotations: Some(annotations),
            ..Default::default()
        },
        spec: Some(JobSpec {
            template: pod_template,
            backoff_limit: Some(step_spec.backoff_limit.unwrap_or_else(|| {
                metadata
                    .step_dsl
                    .retry
                    .as_ref()
                    .map(|r| r.count as i32)
                    .unwrap_or(0)
            })),
            completions: Some(step_spec.completions.unwrap_or(1)),
            parallelism: Some(step_spec.parallelism.unwrap_or(1)),
            active_deadline_seconds: step_spec.active_deadline,
            ttl_seconds_after_finished: Some(step_spec.ttl_seconds_after_finished.unwrap_or(3600)),
            ..Default::default()
        }),
        ..Default::default()
    })
}
