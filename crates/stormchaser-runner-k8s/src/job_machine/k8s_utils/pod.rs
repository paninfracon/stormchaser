use crate::job_machine::{JobMetadata, K8sJobSpec};
use anyhow::{Context, Result};
use k8s_openapi::api::core::v1::{
    Container, EnvVar as K8sEnvVar, PodSpec, PodTemplateSpec, ResourceRequirements as K8sResources,
    Volume,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use std::collections::BTreeMap;

use super::*;
use stormchaser_model::dsl;

pub(crate) struct StepSpec {
    pub(crate) image: String,
    pub(crate) command: Option<Vec<String>>,
    pub(crate) args: Option<Vec<String>>,
    pub(crate) env: Vec<K8sEnvVar>,
    pub(crate) resources: K8sResources,
    pub(crate) active_deadline: Option<i64>,
    pub(crate) backoff_limit: Option<i32>,
    pub(crate) completions: Option<i32>,
    pub(crate) parallelism: Option<i32>,
    pub(crate) ttl_seconds_after_finished: Option<i32>,
    pub(crate) privileged: Option<bool>,
    pub(crate) node_selector: Option<BTreeMap<String, String>>,
    pub(crate) service_account_name: Option<String>,
    pub(crate) restart_policy: Option<String>,
    pub(crate) extra_labels: Option<BTreeMap<String, String>>,
    pub(crate) extra_annotations: Option<BTreeMap<String, String>>,
    pub(crate) storage_mounts: Vec<dsl::StorageMount>,
    pub(crate) secret_mounts: Vec<dsl::SecretMount>,
    pub(crate) config_map_mounts: Vec<dsl::ConfigMapMount>,
}

pub(crate) fn build_k8s_pod_spec(
    container: Container,
    init_containers: Vec<Container>,
    volumes: Vec<Volume>,
    metadata: &JobMetadata,
    step_spec: &StepSpec,
) -> PodTemplateSpec {
    let image_pull_secrets = if metadata.registry_auth.is_some() {
        let secret_name = format!(
            "storm-{}-{}-auth",
            metadata.step_dsl.name.to_lowercase().replace('_', "-"),
            &metadata.step_id.to_string()[..8]
        );
        Some(vec![k8s_openapi::api::core::v1::LocalObjectReference {
            name: secret_name,
        }])
    } else {
        None
    };

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
            image_pull_secrets,
            ..Default::default()
        }),
    }
}

pub(crate) fn parse_step_spec(metadata: &JobMetadata) -> Result<StepSpec> {
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

pub(crate) fn normalize_resource_name(name: &str, prefix: &str) -> String {
    format!("{}-{}", prefix, name.to_lowercase().replace('_', "-"))
}
