//! The per-step Secret that carries what a step's pod must not show.
//!
//! A step's environment holds resolved secrets and injected connection credentials, and
//! its helper containers are given presigned URLs that grant access to the run's
//! storage. Written into the pod as literal values they could be read by anyone allowed
//! to look at pods or Jobs. Instead each value goes into one Secret per step, and the pod
//! refers to it by key, so the pod and the Job show names and never values. The Secret is
//! owned by the step's Job and is deleted with it.

use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::{EnvVar as K8sEnvVar, EnvVarSource, Secret, SecretKeySelector};
use std::collections::BTreeMap;

/// Name of the Secret that holds a step's environment values and helper URLs.
pub fn step_secret_name(job_name: &str) -> String {
    format!("{}-env", job_name)
}

/// The values collected for one step's Secret while its pod is being built.
#[derive(Debug, Clone)]
pub struct StepSecret {
    name: String,
    data: BTreeMap<String, String>,
}

impl StepSecret {
    /// Starts an empty Secret for the Job `job_name`.
    pub fn new(job_name: &str) -> Self {
        Self {
            name: step_secret_name(job_name),
            data: BTreeMap::new(),
        }
    }

    /// Stores `value` and returns an environment variable `name` that reads it from the
    /// Secret. Keys are generated: an environment variable's name need not be a valid
    /// Secret key.
    pub fn env_var(&mut self, name: &str, value: String) -> K8sEnvVar {
        let key = format!("v{}", self.data.len());
        self.data.insert(key.clone(), value);
        K8sEnvVar {
            name: name.to_string(),
            value: None,
            value_from: Some(EnvVarSource {
                secret_key_ref: Some(SecretKeySelector {
                    name: self.name.clone(),
                    key,
                    optional: Some(false),
                }),
                ..Default::default()
            }),
        }
    }

    /// True when the step has nothing to keep out of its pod, and needs no Secret.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// The Secret to create, owned by the step's Job so that Kubernetes deletes the two
    /// together.
    pub fn into_secret(self, job_name: &str, owner: &Job) -> Secret {
        let mut secret = Secret::default();
        secret.metadata.name = Some(self.name);
        secret.metadata.labels = Some(BTreeMap::from([(
            "managed-by".to_string(),
            "stormchaser".to_string(),
        )]));
        secret.metadata.owner_references =
            crate::job_machine::transitions::job_owner_reference(job_name, owner);
        secret.type_ = Some("Opaque".to_string());
        secret.string_data = Some(self.data);
        secret
    }

    #[cfg(test)]
    pub(crate) fn values(&self) -> Vec<&str> {
        self.data.values().map(String::as_str).collect()
    }
}
