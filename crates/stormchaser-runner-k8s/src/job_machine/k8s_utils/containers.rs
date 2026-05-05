use crate::job_machine::JobMetadata;
use k8s_openapi::api::core::v1::{Container, EnvVar as K8sEnvVar, VolumeMount};
use serde_json::Value;
use std::path::PathBuf;

use super::*;

pub(crate) fn build_k8s_containers(
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
            image_pull_policy: if agent_img.contains("stormchaser-agent") { Some("IfNotPresent".to_string()) } else { None },
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
                            image_pull_policy: agent_image.as_ref().and_then(|img| if img.contains("stormchaser-agent") { Some("IfNotPresent".to_string()) } else { None }),
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
                            let mut full_dest = PathBuf::from(&mount.mount_path);
                            if dest != "/" && !dest.is_empty() {
                                let relative_dest = dest.trim_start_matches('/').replace('/', "");
                                full_dest.push(relative_dest);
                            }
                            let dest_str = full_dest.to_str().unwrap_or(&mount.mount_path);

                            init_containers.push(Container {
                                name: format!("{}-{}", normalize_resource_name(&mount.name, "prov"), prov_idx),
                                image: Some(agent_image.clone().unwrap_or_else(|| "alpine:latest".to_string())),
                            image_pull_policy: agent_image.as_ref().and_then(|img| if img.contains("stormchaser-agent") { Some("IfNotPresent".to_string()) } else { None }),
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
        image_pull_policy: if final_image.contains("stormchaser-agent") {
            Some("IfNotPresent".to_string())
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

pub(crate) fn wrap_main_command(
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
                    mount_paths.insert(name.clone(), Value::String(mount.mount_path.clone()));
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
