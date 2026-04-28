use super::{crypto, JobMetadata, K8sJobSpec};
use anyhow::{Context, Result};
use k8s_openapi::api::batch::v1::{Job, JobSpec};
use k8s_openapi::api::core::v1::{
    ConfigMapVolumeSource, Container, EnvVar as K8sEnvVar, Pod, PodSpec, PodTemplateSpec,
    ResourceRequirements as K8sResources, SecretVolumeSource, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use std::collections::BTreeMap;

use stormchaser_model::dsl;

pub fn do_extract_pod_metrics_with_reason(pods: Vec<Pod>) -> (Option<i32>, i32, Option<String>) {
    let mut max_restart_count = 0;
    let mut exit_code = None;
    let mut failure_reason = None;

    for pod in pods {
        if let Some(status) = pod.status {
            if let Some(container_statuses) = status.container_statuses {
                for cs in container_statuses {
                    max_restart_count = max_restart_count.max(cs.restart_count);
                    if let Some(state) = cs.state {
                        if let Some(terminated) = state.terminated {
                            exit_code = Some(terminated.exit_code);
                            if terminated.exit_code != 0 {
                                let reason = terminated.reason.as_deref().unwrap_or("Terminated");
                                let message = terminated.message.as_deref().unwrap_or("");
                                failure_reason = Some(if message.is_empty() {
                                    reason.to_string()
                                } else {
                                    format!("{}: {}", reason, message)
                                });
                            }
                        } else if let Some(waiting) = state.waiting {
                            let reason = waiting.reason.as_deref().unwrap_or("Waiting");
                            let message = waiting.message.as_deref().unwrap_or("");
                            failure_reason = Some(if message.is_empty() {
                                reason.to_string()
                            } else {
                                format!("{}: {}", reason, message)
                            });
                        }
                    }
                }
            }
        }
    }

    (exit_code, 1 + max_restart_count, failure_reason)
}

fn normalize_resource_name(name: &str, prefix: &str) -> String {
    format!("{}-{}", prefix, name.to_lowercase().replace('_', "-"))
}

fn map_dsl_env_to_k8s(env: Vec<dsl::EnvVar>) -> Vec<K8sEnvVar> {
    env.into_iter()
        .map(|v| K8sEnvVar {
            name: v.name,
            value: Some(v.value),
            ..Default::default()
        })
        .collect()
}

fn build_k8s_resources(cpu: Option<String>, memory: Option<String>) -> K8sResources {
    let mut requests = BTreeMap::new();
    let mut limits = BTreeMap::new();
    if let Some(cpu) = cpu {
        requests.insert("cpu".to_string(), Quantity(cpu.clone()));
        limits.insert("cpu".to_string(), Quantity(cpu));
    }
    if let Some(mem) = memory {
        requests.insert("memory".to_string(), Quantity(mem.clone()));
        limits.insert("memory".to_string(), Quantity(mem));
    }
    K8sResources {
        requests: Some(requests),
        limits: Some(limits),
        ..Default::default()
    }
}

struct StepSpec {
    image: String,
    command: Option<Vec<String>>,
    args: Option<Vec<String>>,
    env: Vec<K8sEnvVar>,
    resources: K8sResources,
    active_deadline: Option<i64>,
    backoff_limit: Option<i32>,
    completions: Option<i32>,
    parallelism: Option<i32>,
    ttl_seconds_after_finished: Option<i32>,
    privileged: Option<bool>,
    node_selector: Option<BTreeMap<String, String>>,
    service_account_name: Option<String>,
    restart_policy: Option<String>,
    extra_labels: Option<BTreeMap<String, String>>,
    extra_annotations: Option<BTreeMap<String, String>>,
    storage_mounts: Vec<dsl::StorageMount>,
    secret_mounts: Vec<dsl::SecretMount>,
    config_map_mounts: Vec<dsl::ConfigMapMount>,
}

fn parse_step_spec(metadata: &JobMetadata) -> Result<StepSpec> {
    match metadata.step_dsl.r#type.as_str() {
        "RunContainer" => {
            let spec: dsl::CommonContainerSpec =
                serde_json::from_value(metadata.step_dsl.spec.clone())
                    .context("Failed to parse RunContainer spec as CommonContainerSpec")?;

            let env = map_dsl_env_to_k8s(spec.env.unwrap_or_default());

            let resources = build_k8s_resources(spec.cpu, spec.memory);

            Ok(StepSpec {
                image: spec.image,
                command: spec.command,
                args: spec.args,
                env,
                resources,
                active_deadline: None,
                backoff_limit: None,
                completions: None,
                parallelism: None,
                ttl_seconds_after_finished: None,
                privileged: spec.privileged,
                node_selector: None,
                service_account_name: None,
                restart_policy: None,
                extra_labels: None,
                extra_annotations: None,
                storage_mounts: spec.storage_mounts.unwrap_or_default(),
                secret_mounts: Vec::new(),
                config_map_mounts: Vec::new(),
            })
        }
        "RunK8sJob" => {
            let spec: K8sJobSpec = serde_json::from_value(metadata.step_dsl.spec.clone())
                .context("Failed to parse RunK8sJob spec as K8sJobSpec")?;

            let env = map_dsl_env_to_k8s(spec.env.unwrap_or_default());

            Ok(StepSpec {
                image: spec.image,
                command: spec.command,
                args: spec.args,
                env,
                resources: spec.resources.unwrap_or_default(),
                active_deadline: spec.active_deadline_seconds,
                backoff_limit: spec.backoff_limit,
                completions: spec.completions,
                parallelism: spec.parallelism,
                ttl_seconds_after_finished: spec.ttl_seconds_after_finished,
                privileged: spec.privileged,
                node_selector: spec.node_selector,
                service_account_name: spec.service_account_name,
                restart_policy: spec.restart_policy,
                extra_labels: spec.labels,
                extra_annotations: spec.annotations,
                storage_mounts: spec.storage_mounts.unwrap_or_default(),
                secret_mounts: spec.secret_mounts.unwrap_or_default(),
                config_map_mounts: spec.config_map_mounts.unwrap_or_default(),
            })
        }
        _ => {
            let params = &metadata.step_dsl.params;
            let image = params.get("image").map(|s| s.to_string()).context(format!(
                "Missing 'image' for {} step (neither in spec nor params)",
                metadata.step_dsl.r#type
            ))?;

            Ok(StepSpec {
                image,
                command: params.get("command").map(|c| vec![c.to_string()]),
                args: params.get("args").map(|a| vec![a.to_string()]),
                env: Vec::new(),
                resources: K8sResources::default(),
                active_deadline: None,
                backoff_limit: None,
                completions: None,
                parallelism: None,
                ttl_seconds_after_finished: None,
                privileged: None,
                node_selector: None,
                service_account_name: None,
                restart_policy: None,
                extra_labels: None,
                extra_annotations: None,
                storage_mounts: Vec::new(),
                secret_mounts: Vec::new(),
                config_map_mounts: Vec::new(),
            })
        }
    }
}

fn build_k8s_volumes(
    agent_image_present: bool,
    sfs_pvc_name: Option<&str>,
    metadata: &JobMetadata,
    step_spec: &StepSpec,
) -> (Vec<Volume>, Vec<VolumeMount>) {
    let mut volumes = Vec::new();
    let mut volume_mounts = Vec::new();

    // Add empty dir for agent binary
    if agent_image_present {
        volumes.push(Volume {
            name: "storm-agent".to_string(),
            empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource::default()),
            ..Default::default()
        });
        volume_mounts.push(VolumeMount {
            name: "storm-agent".to_string(),
            mount_path: "/stormchaser/agent".to_string(),
            ..Default::default()
        });
    }

    // Handle secret mounts
    for sm in &step_spec.secret_mounts {
        let vol_name = normalize_resource_name(&sm.name, "sec");
        volumes.push(Volume {
            name: vol_name.clone(),
            secret: Some(SecretVolumeSource {
                secret_name: Some(sm.name.clone()),
                ..Default::default()
            }),
            ..Default::default()
        });
        volume_mounts.push(VolumeMount {
            name: vol_name,
            mount_path: sm.mount_path.clone(),
            ..Default::default()
        });
    }

    // Handle config map mounts
    for cm in &step_spec.config_map_mounts {
        let vol_name = normalize_resource_name(&cm.name, "cm");
        volumes.push(Volume {
            name: vol_name.clone(),
            config_map: Some(ConfigMapVolumeSource {
                name: cm.name.clone(),
                ..Default::default()
            }),
            ..Default::default()
        });
        volume_mounts.push(VolumeMount {
            name: vol_name,
            mount_path: cm.mount_path.clone(),
            ..Default::default()
        });
    }

    // Handle storage mounts
    for mount in &step_spec.storage_mounts {
        let vol_name = normalize_resource_name(&mount.name, "sfs");

        let (volume, sub_path) = if let Some(pvc) = sfs_pvc_name {
            (
                Volume {
                    name: vol_name.clone(),
                    persistent_volume_claim: Some(
                        k8s_openapi::api::core::v1::PersistentVolumeClaimVolumeSource {
                            claim_name: pvc.to_string(),
                            ..Default::default()
                        },
                    ),
                    ..Default::default()
                },
                Some(format!("{}/{}", metadata.run_id, mount.name)),
            )
        } else {
            (
                Volume {
                    name: vol_name.clone(),
                    empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource::default()),
                    ..Default::default()
                },
                None,
            )
        };

        volumes.push(volume);
        volume_mounts.push(VolumeMount {
            name: vol_name,
            mount_path: mount.mount_path.clone(),
            read_only: mount.read_only,
            sub_path,
            ..Default::default()
        });
    }

    (volumes, volume_mounts)
}

fn build_k8s_env_vars(
    metadata: &JobMetadata,
    step_spec: &StepSpec,
    sfs_pvc_name: Option<&str>,
) -> Vec<K8sEnvVar> {
    let mut k8s_env = step_spec.env.clone();

    // Storage URLs
    for mount in &step_spec.storage_mounts {
        if let Some(storage_data) = &metadata.storage {
            if let Some(urls) = storage_data.get(&mount.name) {
                if sfs_pvc_name.is_none() {
                    if let Some(put_url) = urls.get("put_url").and_then(|u| u.as_str()) {
                        k8s_env.push(K8sEnvVar {
                            name: format!("STORMCHASER_PUT_URL_{}", mount.name),
                            value: Some(put_url.to_string()),
                            ..Default::default()
                        });
                        k8s_env.push(K8sEnvVar {
                            name: format!("STORMCHASER_MOUNT_PATH_{}", mount.name),
                            value: Some(mount.mount_path.clone()),
                            ..Default::default()
                        });
                    }
                }

                if let Some(arts) = urls.get("artifacts") {
                    k8s_env.push(K8sEnvVar {
                        name: format!("STORMCHASER_ARTIFACTS_{}", mount.name),
                        value: Some(arts.to_string()),
                        ..Default::default()
                    });
                }
            }
        }
    }

    let storage_names: Vec<String> = step_spec
        .storage_mounts
        .iter()
        .map(|m| m.name.clone())
        .collect();
    if !storage_names.is_empty() {
        k8s_env.push(K8sEnvVar {
            name: "STORMCHASER_STORAGES".to_string(),
            value: Some(storage_names.join(" ")),
            ..Default::default()
        });
    }

    if !metadata.step_dsl.reports.is_empty() {
        k8s_env.push(K8sEnvVar {
            name: "STORMCHASER_TEST_REPORTS".to_string(),
            value: Some(serde_json::to_string(&metadata.step_dsl.reports).unwrap_or_default()),
            ..Default::default()
        });
        if let Some(urls) = &metadata.test_report_urls {
            k8s_env.push(K8sEnvVar {
                name: "STORMCHASER_REPORT_URLS".to_string(),
                value: Some(serde_json::to_string(urls).unwrap_or_default()),
                ..Default::default()
            });
        }
    }

    k8s_env
}

fn wrap_main_command(
    step_spec: &StepSpec,
    metadata: &JobMetadata,
    agent_present: bool,
) -> (String, Option<Vec<String>>, Option<Vec<String>>) {
    let mut original_cmd = Vec::new();
    if let Some(cmd) = &step_spec.command {
        original_cmd.extend(cmd.clone());
    }
    if let Some(a) = &step_spec.args {
        original_cmd.extend(a.clone());
    }

    let step_name = &metadata.step_dsl.name;
    let needs_agent = agent_present
        && (!step_spec.storage_mounts.is_empty() || !metadata.step_dsl.reports.is_empty());

    if original_cmd.is_empty() {
        return (
            step_spec.image.clone(),
            step_spec.command.clone(),
            step_spec.args.clone(),
        );
    }

    let wrapped_script = if needs_agent {
        let mut parking_urls = serde_json::Map::new();
        let mut mount_paths = serde_json::Map::new();
        let mut artifact_urls = serde_json::Map::new();

        if let Some(storage_data) = &metadata.storage {
            for (name, urls) in storage_data {
                if urls.get("put_url").is_some() {
                    parking_urls.insert(name.clone(), urls.clone());
                }
                if let Some(mount) = step_spec.storage_mounts.iter().find(|x| x.name == *name) {
                    mount_paths.insert(
                        name.clone(),
                        serde_json::Value::String(mount.mount_path.clone()),
                    );
                }
                if let Some(artifacts) = urls.get("artifacts").and_then(|a| a.as_object()) {
                    for (art_name, art_data) in artifacts {
                        artifact_urls.insert(art_name.clone(), art_data.clone());
                    }
                }
            }
        }

        let parking_urls_json =
            serde_json::to_string(&parking_urls).unwrap_or_else(|_| "{}".to_string());
        let mount_paths_json =
            serde_json::to_string(&mount_paths).unwrap_or_else(|_| "{}".to_string());
        let artifact_urls_json =
            serde_json::to_string(&artifact_urls).unwrap_or_else(|_| "{}".to_string());

        let mut agent_cmd = format!(
            "/stormchaser/agent/stormchaser-agent run --parking-urls '{}' --mount-paths '{}'",
            parking_urls_json, mount_paths_json
        );

        if !artifact_urls.is_empty() {
            agent_cmd.push_str(&format!(" --artifact-urls '{}'", artifact_urls_json));
        }

        format!(
            "echo '========================================'; \
             echo 'Step Metadata: {}'; \
             echo \"Command: $@\"; \
             echo '========================================'; \
             {} -- \"$@\"; \
             RET=$?; \
             echo '========================================'; \
             echo 'Completion Status: '$RET; \
             echo '========================================'; \
             exit $RET",
            step_name, agent_cmd
        )
    } else {
        format!(
            "echo '========================================'; \
             echo 'Step Metadata: {}'; \
             echo \"Command: $@\"; \
             echo '========================================'; \
             \"$@\"; \
             RET=$?; \
             echo '========================================'; \
             echo 'Completion Status: '$RET; \
             echo '========================================'; \
             exit $RET",
            step_name
        )
    };

    let mut new_args = vec!["-c".to_string(), wrapped_script, "--".to_string()];
    new_args.extend(original_cmd);

    (
        step_spec.image.clone(),
        Some(vec!["/bin/sh".to_string()]),
        Some(new_args),
    )
}

fn build_k8s_containers(
    agent_image: Option<String>,
    sfs_pvc_name: Option<&str>,
    metadata: &JobMetadata,
    step_spec: &StepSpec,
    k8s_env: Vec<K8sEnvVar>,
    volume_mounts: Vec<VolumeMount>,
) -> (Container, Vec<Container>) {
    let mut init_containers = Vec::new();

    // Agent injection
    if let Some(agent_img) = &agent_image {
        init_containers.push(Container {
            name: "inject-agent".to_string(),
            image: Some(agent_img.clone()),
            command: Some(vec!["/bin/sh".to_string(), "-c".to_string(), "cp /usr/local/bin/stormchaser-agent /stormchaser/agent/stormchaser-agent && chmod +x /stormchaser/agent/stormchaser-agent".to_string()]),
            volume_mounts: Some(vec![VolumeMount {
                name: "storm-agent".to_string(),
                mount_path: "/stormchaser/agent".to_string(),
                ..Default::default()
            }]),
            ..Default::default()
        });
    }

    // Storage unparking/provisioning
    for mount in &step_spec.storage_mounts {
        let vol_name = normalize_resource_name(&mount.name, "sfs");
        let sub_path = sfs_pvc_name.map(|_| format!("{}/{}", metadata.run_id, mount.name));

        if let Some(storage_data) = &metadata.storage {
            if let Some(urls) = storage_data.get(&mount.name) {
                let has_state = urls.get("expected_hash").and_then(|h| h.as_str()).is_some();

                if has_state && sfs_pvc_name.is_none() {
                    if let Some(get_url) = urls.get("get_url").and_then(|u| u.as_str()) {
                        init_containers.push(Container {
                            name: normalize_resource_name(&mount.name, "unpark"),
                            image: Some(agent_image.clone().unwrap_or_else(|| "alpine:latest".to_string())),
                            command: Some(vec!["/bin/sh".to_string()]),
                            args: Some(vec![
                                "-c".to_string(),
                                format!(
                                    "mkdir -p \"{}\" && curl -sL \"{}\" | tar -xz -C \"{}\" || true",
                                    mount.mount_path, get_url, mount.mount_path
                                ),
                            ]),
                            volume_mounts: Some(vec![VolumeMount {
                                name: vol_name.clone(),
                                mount_path: mount.mount_path.clone(),
                                sub_path: sub_path.clone(),
                                ..Default::default()
                            }]),
                            ..Default::default()
                        });
                    }
                } else if let Some(provision) = urls.get("provision").and_then(|p| p.as_array()) {
                    for (prov_idx, prov) in provision.iter().enumerate() {
                        if let (Some(url), Some(dest)) = (
                            prov.get("url").and_then(|u| u.as_str()),
                            prov.get("destination").and_then(|d| d.as_str()),
                        ) {
                            let mut full_dest = std::path::PathBuf::from(&mount.mount_path);
                            if dest != "/" && !dest.is_empty() {
                                let relative_dest = dest.trim_start_matches('/').replace('/', "");
                                full_dest.push(relative_dest);
                            }
                            let dest_str = full_dest.to_str().unwrap_or(&mount.mount_path);

                            init_containers.push(Container {
                                name: format!("{}-{}", normalize_resource_name(&mount.name, "prov"), prov_idx),
                                image: Some(agent_image.clone().unwrap_or_else(|| "alpine:latest".to_string())),
                                command: Some(vec!["/bin/sh".to_string()]),
                                args: Some(vec![
                                    "-c".to_string(),
                                    format!(
                                        "if [ -z \"$(ls -A \\\"{}\\\" 2>/dev/null)\" ]; then mkdir -p \"{}\" && curl -sL \"{}\" | tar -xz -C \"{}\" || true; fi",
                                        dest_str, dest_str, url, dest_str
                                    ),
                                ]),
                                volume_mounts: Some(vec![VolumeMount {
                                    name: vol_name.clone(),
                                    mount_path: mount.mount_path.clone(),
                                    sub_path: sub_path.clone(),
                                    ..Default::default()
                                }]),
                                ..Default::default()
                            });
                        }
                    }
                }
            }
        }
    }

    // Wrap script
    let (final_image, final_command, final_args) =
        wrap_main_command(step_spec, metadata, agent_image.is_some());

    let mut container_volume_mounts = volume_mounts;
    if agent_image.is_some()
        && !step_spec.storage_mounts.is_empty()
        && !container_volume_mounts
            .iter()
            .any(|m| m.mount_path == "/stormchaser/agent")
    {
        container_volume_mounts.push(VolumeMount {
            name: "storm-agent".to_string(),
            mount_path: "/stormchaser/agent".to_string(),
            ..Default::default()
        });
    }

    let container = Container {
        name: "worker".to_string(),
        image: Some(final_image.clone()),
        image_pull_policy: if final_image == "docker.io/library/stormchaser-agent:v1" {
            Some("Never".to_string())
        } else {
            None
        },
        command: final_command,
        args: final_args,
        env: Some(k8s_env),
        resources: Some(step_spec.resources.clone()),
        volume_mounts: Some(container_volume_mounts),
        security_context: step_spec.privileged.map(|p| {
            k8s_openapi::api::core::v1::SecurityContext {
                privileged: Some(p),
                ..Default::default()
            }
        }),
        ..Default::default()
    };

    (container, init_containers)
}

fn build_k8s_pod_spec(
    container: Container,
    init_containers: Vec<Container>,
    volumes: Vec<Volume>,
    metadata: &JobMetadata,
    step_spec: &StepSpec,
) -> PodTemplateSpec {
    PodTemplateSpec {
        metadata: Some(ObjectMeta {
            labels: Some(BTreeMap::from([
                ("managed-by".to_string(), "stormchaser".to_string()),
                (
                    "stormchaser-run-id".to_string(),
                    metadata.run_id.to_string(),
                ),
                (
                    "stormchaser-step-id".to_string(),
                    metadata.step_id.to_string(),
                ),
            ])),
            ..Default::default()
        }),
        spec: Some(PodSpec {
            containers: vec![container],
            init_containers: Some(init_containers),
            restart_policy: Some(
                step_spec
                    .restart_policy
                    .clone()
                    .unwrap_or_else(|| "OnFailure".to_string()),
            ),
            volumes: Some(volumes),
            node_selector: step_spec
                .node_selector
                .clone()
                .map(|ns| ns.into_iter().collect()),
            service_account_name: step_spec.service_account_name.clone(),
            ..Default::default()
        }),
    }
}

pub fn do_build_job_spec(
    job_name: &str,
    metadata: &JobMetadata,
    agent_image: Option<String>,
    sfs_pvc_name: Option<String>,
) -> Result<Job> {
    let step_spec = parse_step_spec(metadata)?;

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

    // Job Labels
    let mut labels = BTreeMap::from([
        ("managed-by".to_string(), "stormchaser".to_string()),
        (
            "stormchaser-run-id".to_string(),
            metadata.run_id.to_string(),
        ),
        (
            "stormchaser-step-id".to_string(),
            metadata.step_id.to_string(),
        ),
    ]);
    if let Some(el) = step_spec.extra_labels {
        labels.extend(el);
    }

    // Job Annotations
    let step_dsl_json = serde_json::to_string(&metadata.step_dsl).unwrap_or_default();
    let step_dsl_val = if let Some(key) = &metadata.encryption_key {
        crypto::encrypt_state(&step_dsl_json, key)?
    } else {
        step_dsl_json
    };

    let mut annotations = BTreeMap::from([
        (
            "stormchaser.io/received-at".to_string(),
            metadata.received_at.to_rfc3339(),
        ),
        ("stormchaser.io/step-dsl".to_string(), step_dsl_val),
    ]);
    if let Some(ea) = step_spec.extra_annotations {
        annotations.extend(ea);
    }

    if metadata.encryption_key.is_some() {
        annotations.insert(
            "stormchaser.io/state-encrypted".to_string(),
            "true".to_string(),
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use k8s_openapi::api::core::v1::{
        ContainerState, ContainerStateTerminated, ContainerStatus, PodStatus,
    };
    use stormchaser_model::dsl::Step;
    use uuid::Uuid;

    #[test]
    fn test_do_extract_pod_metrics() {
        let pod = Pod {
            status: Some(PodStatus {
                container_statuses: Some(vec![ContainerStatus {
                    restart_count: 2,
                    state: Some(ContainerState {
                        terminated: Some(ContainerStateTerminated {
                            exit_code: 1,
                            reason: Some("Error".to_string()),
                            message: Some("Something went wrong".to_string()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let (exit_code, attempts, reason) = do_extract_pod_metrics_with_reason(vec![pod]);
        assert_eq!(exit_code, Some(1));
        assert_eq!(attempts, 3);
        assert_eq!(reason, Some("Error: Something went wrong".to_string()));
    }

    #[test]
    fn test_do_build_job_spec_basic() {
        let step_dsl = Step {
            name: "test-step".into(),
            r#type: "RunContainer".into(),
            params: std::collections::HashMap::new(),
            spec: serde_json::json!({
                "image": "alpine:latest",
                "command": ["echo"],
                "args": ["hello"]
            }),
            aggregation: vec![],
            next: vec![],
            outputs: vec![],
            reports: vec![],
            condition: None,
            strategy: None,
            iterate: None,
            iterate_as: None,
            steps: None,
            on_failure: None,
            retry: None,
            timeout: None,
            allow_failure: None,
            start_marker: None,
            end_marker: None,
            artifacts: None,
        };

        let metadata = JobMetadata {
            run_id: Uuid::new_v4(),
            step_id: Uuid::new_v4(),
            step_dsl,
            namespace: "default".into(),
            received_at: Utc::now(),
            cluster_version: "v1.28.0".into(),
            encryption_key: None,
            storage: None,
            test_report_urls: None,
        };

        let job = do_build_job_spec("test-job", &metadata, None, None).unwrap();
        assert_eq!(job.metadata.name, Some("test-job".into()));
        let pod_spec = job.spec.unwrap().template.spec.unwrap();
        assert_eq!(pod_spec.containers[0].image, Some("alpine:latest".into()));
    }

    #[test]
    fn test_do_build_job_spec_with_pvc() {
        let run_id = Uuid::new_v4();
        let step_dsl = Step {
            name: "test-pvc-step".into(),
            r#type: "RunContainer".into(),
            params: std::collections::HashMap::new(),
            spec: serde_json::json!({
                "image": "alpine:latest",
                "command": ["echo"],
                "args": ["hello"],
                "storage_mounts": [
                    {
                        "name": "workspace",
                        "mount_path": "/workspace",
                        "read_only": false
                    }
                ]
            }),
            aggregation: vec![],
            next: vec![],
            outputs: vec![],
            reports: vec![],
            condition: None,
            strategy: None,
            iterate: None,
            iterate_as: None,
            steps: None,
            on_failure: None,
            retry: None,
            timeout: None,
            allow_failure: None,
            start_marker: None,
            end_marker: None,
            artifacts: None,
        };

        let mut storage_map = std::collections::HashMap::new();
        storage_map.insert(
            "workspace".to_string(),
            serde_json::json!({
                "put_url": "http://example.com/put",
                "get_url": "http://example.com/get",
                "expected_hash": "abcd",
                "provision": [
                    {
                        "url": "http://example.com/provision.tar.gz",
                        "destination": "/"
                    }
                ]
            }),
        );

        let metadata = JobMetadata {
            run_id,
            step_id: Uuid::new_v4(),
            step_dsl,
            namespace: "default".into(),
            received_at: Utc::now(),
            cluster_version: "v1.28.0".into(),
            encryption_key: None,
            storage: Some(storage_map),
            test_report_urls: None,
        };

        let job = do_build_job_spec(
            "test-job-pvc",
            &metadata,
            None,
            Some("my-shared-pvc".to_string()),
        )
        .unwrap();

        let pod_spec = job.spec.unwrap().template.spec.unwrap();

        // Should have the pvc volume
        let vol = pod_spec
            .volumes
            .as_ref()
            .unwrap()
            .iter()
            .find(|v| v.name == "sfs-workspace")
            .unwrap();
        assert!(vol.persistent_volume_claim.is_some());
        assert_eq!(
            vol.persistent_volume_claim.as_ref().unwrap().claim_name,
            "my-shared-pvc"
        );

        // Main container should have the volume mount with sub_path
        let main_container = &pod_spec.containers[0];
        let mount = main_container
            .volume_mounts
            .as_ref()
            .unwrap()
            .iter()
            .find(|m| m.name == "sfs-workspace")
            .unwrap();
        assert_eq!(mount.sub_path, Some(format!("{}/workspace", run_id)));

        // Because we are using PVC, it should NOT inject STORMCHASER_PUT_URL_workspace for S3
        let env = main_container.env.as_ref().unwrap();
        assert!(!env
            .iter()
            .any(|e| e.name == "STORMCHASER_PUT_URL_workspace"));

        // But because of provision, it SHOULD have a provision init container
        assert!(pod_spec.init_containers.is_some());
        let init_containers = pod_spec.init_containers.as_ref().unwrap();
        let prov_container = init_containers
            .iter()
            .find(|c| c.name == "prov-workspace-0");
        assert!(prov_container.is_some());

        // The unpark container (from get_url/expected_hash) should NOT be present
        let unpark_container = init_containers
            .iter()
            .find(|c| c.name == "unpark-workspace");
        assert!(unpark_container.is_none());
    }

    #[test]
    fn test_normalize_resource_name() {
        assert_eq!(
            normalize_resource_name("My_Resource", "prefix"),
            "prefix-my-resource"
        );
        assert_eq!(
            normalize_resource_name("Another-Test", "job"),
            "job-another-test"
        );
    }

    #[test]
    fn test_map_dsl_env_to_k8s() {
        let dsl_env = vec![
            dsl::EnvVar {
                name: "KEY1".to_string(),
                value: "VAL1".to_string(),
            },
            dsl::EnvVar {
                name: "KEY2".to_string(),
                value: "VAL2".to_string(),
            },
        ];
        let k8s_env = map_dsl_env_to_k8s(dsl_env);
        assert_eq!(k8s_env.len(), 2);
        assert_eq!(k8s_env[0].name, "KEY1");
        assert_eq!(k8s_env[0].value, Some("VAL1".to_string()));
    }

    #[test]
    fn test_build_k8s_resources() {
        let res = build_k8s_resources(Some("500m".to_string()), Some("1Gi".to_string()));
        let requests = res.requests.unwrap();
        let limits = res.limits.unwrap();
        assert_eq!(requests["cpu"].0, "500m");
        assert_eq!(requests["memory"].0, "1Gi");
        assert_eq!(limits["cpu"].0, "500m");
        assert_eq!(limits["memory"].0, "1Gi");
    }
}
