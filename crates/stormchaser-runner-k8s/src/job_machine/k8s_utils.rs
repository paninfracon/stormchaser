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

pub fn do_build_job_spec(
    job_name: &str,
    metadata: &JobMetadata,
    agent_image: Option<String>,
    sfs_pvc_name: Option<String>,
) -> Result<Job> {
    let mut volume_mounts = Vec::new();
    let mut volumes = Vec::new();
    let mut k8s_env = Vec::new();
    let mut k8s_resources = K8sResources::default();

    // Add empty dir for agent binary
    if agent_image.is_some() {
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

    let (
        image,
        command,
        args,
        active_deadline,
        backoff_limit,
        completions,
        parallelism,
        ttl_seconds_after_finished,
        privileged,
        node_selector,
        service_account_name,
        restart_policy,
        extra_labels,
        extra_annotations,
        storage_mounts,
    ) = match metadata.step_dsl.r#type.as_str() {
        "RunContainer" => {
            let spec: stormchaser_model::dsl::CommonContainerSpec =
                serde_json::from_value(metadata.step_dsl.spec.clone())
                    .context("Failed to parse RunContainer spec as CommonContainerSpec")?;

            // Map common fields
            if let Some(env) = spec.env {
                k8s_env = env
                    .into_iter()
                    .map(|v| K8sEnvVar {
                        name: v.name,
                        value: Some(v.value),
                        ..Default::default()
                    })
                    .collect();
            }

            let mut requests = BTreeMap::new();
            let mut limits = BTreeMap::new();
            if let Some(cpu) = spec.cpu {
                requests.insert("cpu".to_string(), Quantity(cpu.clone()));
                limits.insert("cpu".to_string(), Quantity(cpu));
            }
            if let Some(mem) = spec.memory {
                requests.insert("memory".to_string(), Quantity(mem.clone()));
                limits.insert("memory".to_string(), Quantity(mem));
            }
            k8s_resources = K8sResources {
                requests: Some(requests),
                limits: Some(limits),
                ..Default::default()
            };

            (
                spec.image,
                spec.command,
                spec.args,
                None,
                None,
                None,
                None,
                None,
                spec.privileged,
                None,
                None,
                None,
                None,
                None,
                spec.storage_mounts,
            )
        }
        "RunK8sJob" => {
            let spec: K8sJobSpec = serde_json::from_value(metadata.step_dsl.spec.clone())
                .context("Failed to parse RunK8sJob spec as K8sJobSpec")?;

            // Handle secret mounts
            if let Some(mounts) = spec.secret_mounts {
                for sm in mounts {
                    let vol_name = format!("sec-{}", sm.name.to_lowercase().replace('_', "-"));
                    volumes.push(Volume {
                        name: vol_name.clone(),
                        secret: Some(SecretVolumeSource {
                            secret_name: Some(sm.name),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                    volume_mounts.push(VolumeMount {
                        name: vol_name,
                        mount_path: sm.mount_path,
                        ..Default::default()
                    });
                }
            }

            // Handle config map mounts
            if let Some(mounts) = spec.config_map_mounts {
                for cm in mounts {
                    let vol_name = format!("cm-{}", cm.name.to_lowercase().replace('_', "-"));
                    volumes.push(Volume {
                        name: vol_name.clone(),
                        config_map: Some(ConfigMapVolumeSource {
                            name: cm.name,
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                    volume_mounts.push(VolumeMount {
                        name: vol_name,
                        mount_path: cm.mount_path,
                        ..Default::default()
                    });
                }
            }

            // Convert env
            if let Some(env) = spec.env {
                k8s_env = env
                    .into_iter()
                    .map(|v| K8sEnvVar {
                        name: v.name,
                        value: Some(v.value),
                        ..Default::default()
                    })
                    .collect();
            }

            // Convert resources
            if let Some(r) = spec.resources {
                k8s_resources = r;
            }

            (
                spec.image,
                spec.command,
                spec.args,
                spec.active_deadline_seconds,
                spec.backoff_limit,
                spec.completions,
                spec.parallelism,
                spec.ttl_seconds_after_finished,
                spec.privileged,
                spec.node_selector,
                spec.service_account_name,
                spec.restart_policy,
                spec.labels,
                spec.annotations,
                spec.storage_mounts,
            )
        }
        _ => {
            // Fallback for unknown types or legacy RunContainer behavior
            let params = &metadata.step_dsl.params;
            let image = params.get("image").map(|s| s.to_string()).context(format!(
                "Missing 'image' for {} step (neither in spec nor params)",
                metadata.step_dsl.r#type
            ))?;

            (
                image,
                params.get("command").map(|c| vec![c.to_string()]),
                params.get("args").map(|a| vec![a.to_string()]),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
        }
    };

    // Handle storage mounts
    tracing::info!("do_build_job_spec agent_image: {:?}", agent_image);
    let mut init_containers = Vec::new();
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
    let mut storage_names = Vec::new();

    if let Some(ref mounts) = storage_mounts {
        for mount in mounts {
            storage_names.push(mount.name.clone());
            let vol_name = format!("sfs-{}", mount.name.to_lowercase().replace('_', "-"));

            let (volume_source, sub_path) = if let Some(pvc) = &sfs_pvc_name {
                (
                    Volume {
                        name: vol_name.clone(),
                        persistent_volume_claim: Some(
                            k8s_openapi::api::core::v1::PersistentVolumeClaimVolumeSource {
                                claim_name: pvc.clone(),
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

            volumes.push(volume_source);

            volume_mounts.push(VolumeMount {
                name: vol_name.clone(),
                mount_path: mount.mount_path.clone(),
                read_only: mount.read_only,
                sub_path: sub_path.clone(),
                ..Default::default()
            });

            // Pass URLs via env
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

                    let has_state = urls.get("expected_hash").and_then(|h| h.as_str()).is_some();

                    if has_state && sfs_pvc_name.is_none() {
                        if let Some(get_url) = urls.get("get_url").and_then(|u| u.as_str()) {
                            init_containers.push(Container {
                                name: format!("unpark-{}", mount.name.to_lowercase().replace('_', "-")),
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
                    } else if let Some(provision) = urls.get("provision").and_then(|p| p.as_array())
                    {
                        let mut prov_idx = 0;
                        for prov in provision {
                            if let (Some(url), Some(dest)) = (
                                prov.get("url").and_then(|u| u.as_str()),
                                prov.get("destination").and_then(|d| d.as_str()),
                            ) {
                                let mut full_dest = std::path::PathBuf::from(&mount.mount_path);
                                if dest != "/" && !dest.is_empty() {
                                    let relative_dest =
                                        dest.trim_start_matches('/').replace('/', "");
                                    full_dest.push(relative_dest);
                                }
                                let dest_str = full_dest.to_str().unwrap_or(&mount.mount_path);

                                init_containers.push(Container {
                                    name: format!("prov-{}-{}", mount.name.to_lowercase().replace('_', "-"), prov_idx),
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
                                prov_idx += 1;
                            }
                        }
                    }
                }
            }
        }
    }

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

    let (final_image, final_command, final_args) = {
        let mut original_cmd = Vec::new();
        if let Some(cmd) = &command {
            original_cmd.extend(cmd.clone());
        }
        if let Some(a) = &args {
            original_cmd.extend(a.clone());
        }

        let step_name = &metadata.step_dsl.name;

        let needs_agent = agent_image.is_some()
            && (!storage_names.is_empty() || !metadata.step_dsl.reports.is_empty());

        let (cmd_to_use, args_to_use) = if !original_cmd.is_empty() {
            let wrapped_script = if needs_agent {
                let mut parking_urls = serde_json::Map::new();
                let mut mount_paths = serde_json::Map::new();
                let mut artifact_urls = serde_json::Map::new();

                if let Some(storage_data) = &metadata.storage {
                    for (name, urls) in storage_data {
                        if urls.get("put_url").is_some() {
                            parking_urls.insert(name.clone(), urls.clone());
                        }
                        if let Some(mount) = storage_mounts
                            .as_ref()
                            .and_then(|m| m.iter().find(|x| x.name == *name))
                        {
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

            (Some(vec!["/bin/sh".to_string()]), Some(new_args))
        } else {
            (command, args)
        };

        (image, cmd_to_use, args_to_use)
    };

    let mut container_volume_mounts = volume_mounts.clone();
    if agent_image.is_some() && !storage_names.is_empty() {
        // Check if already mounted (might have been added via spec)
        if !container_volume_mounts
            .iter()
            .any(|m| m.mount_path == "/stormchaser/agent")
        {
            container_volume_mounts.push(VolumeMount {
                name: "storm-agent".to_string(),
                mount_path: "/stormchaser/agent".to_string(),
                ..Default::default()
            });
        }
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
        resources: Some(k8s_resources),
        volume_mounts: Some(container_volume_mounts),
        security_context: privileged.map(|p| k8s_openapi::api::core::v1::SecurityContext {
            privileged: Some(p),
            ..Default::default()
        }),
        ..Default::default()
    };

    let pod_template = PodTemplateSpec {
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
            restart_policy: Some(restart_policy.unwrap_or_else(|| "OnFailure".to_string())),
            volumes: Some(volumes),
            node_selector: node_selector.map(|ns| ns.into_iter().collect()),
            service_account_name,
            ..Default::default()
        }),
    };

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
    if let Some(el) = extra_labels {
        labels.extend(el);
    }

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
    if let Some(ea) = extra_annotations {
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
            backoff_limit: Some(backoff_limit.unwrap_or_else(|| {
                metadata
                    .step_dsl
                    .retry
                    .as_ref()
                    .map(|r| r.count as i32)
                    .unwrap_or(0)
            })),
            completions: Some(completions.unwrap_or(1)),
            parallelism: Some(parallelism.unwrap_or(1)),
            active_deadline_seconds: active_deadline,
            ttl_seconds_after_finished: Some(ttl_seconds_after_finished.unwrap_or(3600)),
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
}
