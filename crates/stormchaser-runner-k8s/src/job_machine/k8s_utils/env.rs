use crate::job_machine::JobMetadata;
use k8s_openapi::api::core::v1::EnvVar as K8sEnvVar;

use super::*;
use stormchaser_model::dsl;

pub(crate) fn map_dsl_env_to_k8s(env: Vec<dsl::EnvVar>) -> Vec<K8sEnvVar> {
    env.into_iter()
        .map(|v| K8sEnvVar {
            name: v.name,
            value: Some(v.value),
            ..Default::default()
        })
        .collect()
}

pub(crate) fn build_k8s_env_vars(
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
