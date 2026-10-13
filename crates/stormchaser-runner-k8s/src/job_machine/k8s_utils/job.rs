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
) -> Result<(Job, StepSecret)> {
    let step_spec = parse_step_spec(metadata)?;
    let mut secrets = StepSecret::new(job_name);
    StepPolicy::from_env().check(&step_spec)?;

    // Helper containers mount the step's volumes at the paths the step chose
    for mount in &step_spec.storage_mounts {
        stormchaser_model::dsl::ensure_mount_path_clear_of_helpers(&mount.mount_path)?;
    }

    let agent_present = agent_image.is_some();
    let (mut volumes, volume_mounts) = build_k8s_volumes(
        uses_in_step_agent(agent_present, metadata),
        sfs_pvc_name.as_deref(),
        metadata,
        &step_spec,
    );
    if let Some(plan) = plan_sidecar(agent_present, sfs_pvc_name.as_deref(), metadata, &step_spec) {
        volumes.extend(sidecar_volumes(job_name, plan.push_auth));
    }

    let k8s_env = build_k8s_env_vars(metadata, &step_spec, &mut secrets);

    let (containers, init_containers) = build_k8s_containers(
        agent_image,
        sfs_pvc_name.as_deref(),
        metadata,
        &step_spec,
        k8s_env,
        volume_mounts,
        &mut secrets,
    );

    let pod_template =
        build_k8s_pod_spec(containers, init_containers, volumes, metadata, &step_spec);

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

    // Job Annotations. The step's definition is kept so that a restarted runner can pick
    // the step up again. Encrypted, it is kept whole; in clear, it is kept without the
    // values that are secret, which picking a step up does not need.
    let step_dsl_val = match &metadata.encryption_key {
        Some(key) => crypto::encrypt_state(
            &serde_json::to_string(&metadata.step_dsl).unwrap_or_default(),
            key,
        )?,
        None => {
            serde_json::to_string(&metadata.step_dsl.without_secret_values()).unwrap_or_default()
        }
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

    // A Job that depends on Secrets is created suspended, so that no pod exists until
    // the runner has created them (they are owned by the Job, so the Job comes first).
    let suspend = (!secrets.is_empty() || metadata.registry_auth.is_some()).then_some(true);

    let job = Job {
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
            suspend,
            ..Default::default()
        }),
        ..Default::default()
    };
    Ok((job, secrets))
}
