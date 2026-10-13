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

pub(crate) fn build_k8s_env_vars(metadata: &JobMetadata, step_spec: &StepSpec) -> Vec<K8sEnvVar> {
    let mut k8s_env = step_spec.env.clone();

    // Upload URLs and artifact instructions are not here: they go to the agent sidecar,
    // which the step's own code cannot read.
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
