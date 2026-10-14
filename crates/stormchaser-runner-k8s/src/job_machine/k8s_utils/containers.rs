use crate::job_machine::JobMetadata;
use k8s_openapi::api::core::v1::{Container, EnvVar as K8sEnvVar, VolumeMount};
use std::path::PathBuf;

use super::*;

/// True when the agent binary is copied into the step's own container to collect the
/// test reports the step declares. Reports are files the step wrote anywhere in its own
/// filesystem, so only something inside that container can read them; what it uploads is
/// the author's own data either way.
pub(crate) fn uses_in_step_agent(agent_present: bool, metadata: &JobMetadata) -> bool {
    agent_present && !metadata.step_dsl.reports.is_empty()
}

/// Environment variable through which a helper init container is given the presigned URL
/// it downloads from.
const FETCH_URL_ENV: &str = "STORMCHASER_FETCH_URL";

/// Pull policy for the image a step names: `Always` (the default), `IfNotPresent` or
/// `Never`.
pub const STEP_IMAGE_PULL_POLICY_ENV: &str = "STORMCHASER_STEP_IMAGE_PULL_POLICY";

/// The pull policy for a step's own image.
///
/// With `IfNotPresent` (Kubernetes' default for a tag other than `latest`) a node starts
/// an image it already holds without asking the registry, so a step without credentials
/// for a private image could run it once another user's run had pulled it onto the same
/// node. `Always` makes the kubelet check with the registry, using this step's own pull
/// secrets, every time; layers already on the node are not downloaded again. An operator
/// who preloads step images onto nodes can choose `IfNotPresent` or `Never`.
pub(crate) fn step_image_pull_policy(configured: Option<&str>) -> String {
    match configured.map(str::trim) {
        None | Some("") | Some("Always") => "Always".to_string(),
        Some(policy @ ("IfNotPresent" | "Never")) => policy.to_string(),
        Some(other) => {
            tracing::warn!(
                "Ignoring {}={:?}: expected Always, IfNotPresent or Never",
                STEP_IMAGE_PULL_POLICY_ENV,
                other
            );
            "Always".to_string()
        }
    }
}

/// Builds the pod's containers (the step's own first, then the agent sidecar if there is
/// storage to park) and its init containers.
pub(crate) fn build_k8s_containers(
    agent_image: Option<String>,
    sfs_pvc_name: Option<&str>,
    metadata: &JobMetadata,
    step_spec: &StepSpec,
    k8s_env: Vec<K8sEnvVar>,
    volume_mounts: Vec<VolumeMount>,
    secrets: &mut StepSecret,
) -> (Vec<Container>, Vec<Container>) {
    let mut init_containers = Vec::new();
    let in_step_agent = uses_in_step_agent(agent_image.is_some(), metadata);

    // Agent injection
    if let Some(agent_img) = agent_image.as_ref().filter(|_| in_step_agent) {
        init_containers.push(Container {
            name: "inject-agent".to_string(),
            image: Some(agent_img.clone()),
            image_pull_policy: if agent_img.contains("stormchaser-agent") { Some("IfNotPresent".to_string()) } else { None },
            command: Some(vec!["/bin/sh".to_string(), "-c".to_string(), "cp /usr/local/bin/stormchaser-agent /stormchaser/agent/stormchaser-agent && chmod +x /stormchaser/agent/stormchaser-agent".to_string()]),
            security_context: Some(agent_security_context()),
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
                            security_context: Some(agent_security_context()),
                            image_pull_policy: agent_image.as_ref().and_then(|img| if img.contains("stormchaser-agent") { Some("IfNotPresent".to_string()) } else { None }),
                            command: Some(vec!["/bin/sh".to_string()]),
                            // The presigned URL is read from the step's Secret, not
                            // written into the pod
                            args: Some(vec![
                                "-c".to_string(),
                                format!(
                                    "mkdir -p \"{}\" && curl -sL \"${}\" | tar -xz -C \"{}\" || true",
                                    mount.mount_path, FETCH_URL_ENV, mount.mount_path
                                ),
                            ]),
                            env: Some(vec![secrets.env_var(FETCH_URL_ENV, get_url.to_string())]),
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
                            security_context: Some(agent_security_context()),
                            image_pull_policy: agent_image.as_ref().and_then(|img| if img.contains("stormchaser-agent") { Some("IfNotPresent".to_string()) } else { None }),
                                command: Some(vec!["/bin/sh".to_string()]),
                                args: Some(vec![
                                    "-c".to_string(),
                                    format!(
                                        "if [ -z \"$(ls -A \\\"{}\\\" 2>/dev/null)\" ]; then mkdir -p \"{}\" && curl -sL \"${}\" | tar -xz -C \"{}\" || true; fi",
                                        dest_str, dest_str, FETCH_URL_ENV, dest_str
                                    ),
                                ]),
                                env: Some(vec![secrets.env_var(FETCH_URL_ENV, url.to_string())]),
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

    let cmd_info = wrap_main_command(step_spec, metadata, agent_image.is_some());
    let final_image = cmd_info.image;
    let final_command = cmd_info.command;
    let final_args = cmd_info.args;

    let container_volume_mounts = volume_mounts;

    let container = Container {
        name: WORKER_CONTAINER.to_string(),
        image: Some(final_image.clone()),
        image_pull_policy: Some(step_image_pull_policy(
            std::env::var(STEP_IMAGE_PULL_POLICY_ENV).ok().as_deref(),
        )),
        command: final_command,
        args: final_args,
        env: Some(k8s_env),
        resources: Some(step_spec.resources.clone()),
        volume_mounts: Some(container_volume_mounts),
        security_context: Some(step_security_context(step_spec.privileged)),
        ..Default::default()
    };

    let mut containers = vec![container];
    if let (Some(agent_img), Some(plan)) = (
        agent_image.as_ref(),
        plan_sidecar(agent_image.is_some(), sfs_pvc_name, metadata, step_spec),
    ) {
        containers.push(build_agent_sidecar(
            agent_img,
            plan,
            storage_volume_mounts(sfs_pvc_name, metadata, step_spec),
            secrets,
        ));
    }

    (containers, init_containers)
}

/// How a step's own container runs. A privileged step (which the runner has already
/// checked it may run) is left as it asked. Any other gets the runtime's default seccomp
/// profile, cannot gain privileges through setuid binaries, and loses `NET_RAW`, which
/// lets it forge packets on the pod network. It keeps the image's user and the other
/// default capabilities, which ordinary images that run as root (package installs,
/// `chown`) need. These fit the `baseline` Pod Security level the chart sets on the
/// steps namespace.
pub(crate) fn step_security_context(
    privileged: Option<bool>,
) -> k8s_openapi::api::core::v1::SecurityContext {
    use k8s_openapi::api::core::v1::{Capabilities, SeccompProfile, SecurityContext};
    if privileged == Some(true) {
        return SecurityContext {
            privileged: Some(true),
            ..Default::default()
        };
    }
    SecurityContext {
        privileged: Some(false),
        allow_privilege_escalation: Some(false),
        capabilities: Some(Capabilities {
            drop: Some(vec!["NET_RAW".to_string()]),
            ..Default::default()
        }),
        seccomp_profile: Some(SeccompProfile {
            type_: "RuntimeDefault".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub(crate) struct ContainerCommandInfo {
    pub image: String,
    pub command: Option<Vec<String>>,
    pub args: Option<Vec<String>>,
}

pub(crate) fn wrap_main_command(
    step_spec: &StepSpec,
    metadata: &JobMetadata,
    agent_present: bool,
) -> ContainerCommandInfo {
    let mut original_cmd = Vec::new();
    if let Some(cmd) = &step_spec.command {
        original_cmd.extend(cmd.clone());
    }
    if let Some(a) = &step_spec.args {
        original_cmd.extend(a.clone());
    }

    let step_name = &metadata.step_dsl.name;
    let needs_agent = uses_in_step_agent(agent_present, metadata);

    if original_cmd.is_empty() {
        return ContainerCommandInfo {
            image: step_spec.image.clone(),
            command: step_spec.command.clone(),
            args: step_spec.args.clone(),
        };
    }

    let wrapped_script = if needs_agent {
        // Here the agent only collects test reports (named in its environment). Storage
        // and artifacts are parked by the sidecar, so it is given nothing to upload them
        // with.
        let agent_cmd =
            "/stormchaser/agent/stormchaser-agent run --parking-urls '{}' --mount-paths '{}'";

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

    ContainerCommandInfo {
        image: step_spec.image.clone(),
        command: Some(vec!["/bin/sh".to_string()]),
        args: Some(new_args),
    }
}

#[cfg(test)]
mod tests {
    use super::{step_image_pull_policy, step_security_context};

    #[test]
    fn a_step_image_is_checked_with_the_registry_unless_the_operator_says_otherwise() {
        assert_eq!(step_image_pull_policy(None), "Always");
        assert_eq!(step_image_pull_policy(Some("")), "Always");
        assert_eq!(step_image_pull_policy(Some("IfNotPresent")), "IfNotPresent");
        assert_eq!(step_image_pull_policy(Some("Never")), "Never");
        assert_eq!(step_image_pull_policy(Some("sometimes")), "Always");
    }

    #[test]
    fn steps_cannot_gain_privileges_or_forge_packets() {
        let context = step_security_context(None);
        assert_eq!(context.allow_privilege_escalation, Some(false));
        assert_eq!(context.privileged, Some(false));
        assert_eq!(
            context.capabilities.unwrap().drop,
            Some(vec!["NET_RAW".to_string()])
        );
        assert_eq!(context.seccomp_profile.unwrap().type_, "RuntimeDefault");
    }

    #[test]
    fn a_privileged_step_is_left_as_it_asked() {
        let context = step_security_context(Some(true));
        assert_eq!(context.privileged, Some(true));
        assert!(context.allow_privilege_escalation.is_none());
    }
}
