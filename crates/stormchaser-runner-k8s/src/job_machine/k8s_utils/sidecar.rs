//! The agent sidecar that parks a step's storage and publishes its artifacts.
//!
//! The step's own container runs nothing but the author's command and holds no upload
//! URLs or registry credentials. A second container in the pod, which shares only the
//! storage volumes with it, waits until the runner reports that the step's container has
//! finished and then does the parking. What it logs is what the runner trusts, because
//! the author's code cannot write to another container's output.

use crate::job_machine::JobMetadata;
use k8s_openapi::api::core::v1::{
    Container, DownwardAPIVolumeFile, DownwardAPIVolumeSource, ObjectFieldSelector,
    ResourceRequirements, SecretVolumeSource, SecurityContext, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use stormchaser_model::nats::HELPER_REGISTRY_CONFIG_PATH;

use super::*;

/// Name of the container that runs the workflow author's command.
pub const WORKER_CONTAINER: &str = "worker";
/// Name of the agent sidecar container.
pub const AGENT_SIDECAR_CONTAINER: &str = "stormchaser-agent";
/// Pod annotation the runner sets to the worker container's exit code once it has
/// finished. The sidecar sees it through a downward API volume.
pub const WORKER_EXIT_ANNOTATION: &str = "stormchaser.v1.io/worker-exit";

const PARKING_URLS_ENV: &str = "STORMCHASER_PARKING_URLS";
const ARTIFACT_URLS_ENV: &str = "STORMCHASER_ARTIFACT_URLS";
const CONTROL_VOLUME: &str = "storm-control";
const CONTROL_DIR: &str = "/stormchaser/control";
const WORKER_EXIT_FILE: &str = "worker-exit";
const PUSH_AUTH_VOLUME: &str = "storm-push-auth";
/// Key of the Docker client configuration inside the push credentials Secret.
pub const PUSH_AUTH_SECRET_KEY: &str = "config.json";

/// Name of the Secret the runner creates with a step's registry push credentials.
pub fn push_auth_secret_name(job_name: &str) -> String {
    format!("{}-push-auth", job_name)
}

/// What a pod's agent sidecar has to do once the step's container has finished.
pub(crate) struct SidecarPlan {
    /// Upload instructions for each storage volume to park, by name.
    parking_urls: serde_json::Map<String, Value>,
    /// Where each storage volume is mounted, by name.
    mount_paths: serde_json::Map<String, Value>,
    /// The artifacts to publish, by name, with paths made absolute inside their volume.
    artifacts: serde_json::Map<String, Value>,
    /// True when an artifact goes to a registry that needs credentials, which the runner
    /// fetches from the engine once the step's container has finished.
    pub(crate) push_auth: bool,
}

/// Decides whether the pod gets an agent sidecar, and what for.
///
/// It gets one when an agent image is configured and the step has storage to park or
/// artifacts to publish. With a shared PVC the volumes persist by themselves and are not
/// parked.
pub(crate) fn plan_sidecar(
    agent_present: bool,
    sfs_pvc_name: Option<&str>,
    metadata: &JobMetadata,
    step_spec: &StepSpec,
) -> Option<SidecarPlan> {
    if !agent_present {
        return None;
    }
    let storage_data = metadata.storage.as_ref()?;

    let mut parking_urls = serde_json::Map::new();
    let mut mount_paths = serde_json::Map::new();
    let mut artifacts = serde_json::Map::new();
    for mount in &step_spec.storage_mounts {
        let Some(urls) = storage_data.get(&mount.name) else {
            continue;
        };
        mount_paths.insert(mount.name.clone(), Value::String(mount.mount_path.clone()));
        if sfs_pvc_name.is_none() && urls.get("put_url").is_some_and(|u| !u.is_null()) {
            parking_urls.insert(mount.name.clone(), urls.clone());
        }
        let declared = urls.get("artifacts").and_then(|a| a.as_object());
        for (name, artifact) in declared.into_iter().flatten() {
            let mut artifact = artifact.clone();
            if let Some(path) = artifact.get("path").and_then(|p| p.as_str()) {
                let absolute = Path::new(&mount.mount_path).join(path);
                artifact["path"] = Value::String(absolute.to_string_lossy().into_owned());
            }
            artifacts.insert(name.clone(), artifact);
        }
    }

    if parking_urls.is_empty() && artifacts.is_empty() {
        return None;
    }
    let push_auth = artifacts
        .values()
        .any(|artifact| artifact.get("needs_auth").and_then(Value::as_bool) == Some(true));
    Some(SidecarPlan {
        parking_urls,
        mount_paths,
        artifacts,
        push_auth,
    })
}

/// Volumes only the sidecar mounts: the runner's signal, and its registry credentials.
pub(crate) fn sidecar_volumes(job_name: &str, push_auth: bool) -> Vec<Volume> {
    let mut volumes = vec![Volume {
        name: CONTROL_VOLUME.to_string(),
        downward_api: Some(DownwardAPIVolumeSource {
            items: Some(vec![DownwardAPIVolumeFile {
                path: WORKER_EXIT_FILE.to_string(),
                field_ref: Some(ObjectFieldSelector {
                    field_path: format!("metadata.annotations['{}']", WORKER_EXIT_ANNOTATION),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    }];
    if push_auth {
        // The Secret does not exist when the pod starts: the runner creates it when the
        // step's container has finished, so that a short-lived token is still fresh.
        volumes.push(Volume {
            name: PUSH_AUTH_VOLUME.to_string(),
            secret: Some(SecretVolumeSource {
                secret_name: Some(push_auth_secret_name(job_name)),
                optional: Some(true),
                default_mode: Some(0o400),
                ..Default::default()
            }),
            ..Default::default()
        });
    }
    volumes
}

/// Builds the agent sidecar container.
pub(crate) fn build_agent_sidecar(
    agent_image: &str,
    plan: SidecarPlan,
    storage_mounts: Vec<VolumeMount>,
    secrets: &mut StepSecret,
) -> Container {
    let push_auth = plan.push_auth;

    // The upload instructions hold presigned URLs. They reach the agent through its
    // environment, from the step's Secret; `$(NAME)` in an argument is filled in by the
    // kubelet when it starts the container and is never part of the pod.
    let mut env = vec![secrets.env_var(
        PARKING_URLS_ENV,
        Value::Object(plan.parking_urls).to_string(),
    )];
    let mut args = vec![
        "park".to_string(),
        "--wait-for".to_string(),
        format!("{}/{}", CONTROL_DIR, WORKER_EXIT_FILE),
        "--parking-urls".to_string(),
        format!("$({})", PARKING_URLS_ENV),
        "--mount-paths".to_string(),
        Value::Object(plan.mount_paths).to_string(),
    ];
    if !plan.artifacts.is_empty() {
        env.push(secrets.env_var(ARTIFACT_URLS_ENV, Value::Object(plan.artifacts).to_string()));
        args.push("--artifact-urls".to_string());
        args.push(format!("$({})", ARTIFACT_URLS_ENV));
    }
    if push_auth {
        args.push("--registry-config".to_string());
        args.push(HELPER_REGISTRY_CONFIG_PATH.to_string());
    }

    let mut volume_mounts = storage_mounts;
    volume_mounts.push(VolumeMount {
        name: CONTROL_VOLUME.to_string(),
        mount_path: CONTROL_DIR.to_string(),
        read_only: Some(true),
        ..Default::default()
    });
    if push_auth {
        let auth_dir = Path::new(HELPER_REGISTRY_CONFIG_PATH)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        volume_mounts.push(VolumeMount {
            name: PUSH_AUTH_VOLUME.to_string(),
            mount_path: auth_dir,
            read_only: Some(true),
            ..Default::default()
        });
    }

    Container {
        name: AGENT_SIDECAR_CONTAINER.to_string(),
        image: Some(agent_image.to_string()),
        image_pull_policy: agent_image
            .contains("stormchaser-agent")
            .then(|| "IfNotPresent".to_string()),
        command: Some(vec!["/usr/local/bin/stormchaser-agent".to_string()]),
        args: Some(args),
        env: Some(env),
        volume_mounts: Some(volume_mounts),
        // Enough to be scheduled under a quota; parking is not capped
        resources: Some(ResourceRequirements {
            requests: Some(BTreeMap::from([
                ("cpu".to_string(), Quantity("10m".to_string())),
                ("memory".to_string(), Quantity("32Mi".to_string())),
            ])),
            ..Default::default()
        }),
        security_context: Some(agent_security_context()),
        ..Default::default()
    }
}

/// How containers from the agent image run. The image's own user is unprivileged; the
/// agent reads and writes files a step created as any user, so it runs as root, but
/// cannot gain more than that.
pub(crate) fn agent_security_context() -> SecurityContext {
    SecurityContext {
        run_as_user: Some(0),
        run_as_group: Some(0),
        allow_privilege_escalation: Some(false),
        ..Default::default()
    }
}
