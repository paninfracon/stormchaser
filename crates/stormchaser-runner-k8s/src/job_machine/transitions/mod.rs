use super::{k8s_utils, state, JobMetrics, JobState, K8sJobMachine, StartResult};
use anyhow::{Context, Result};
use chrono::Utc;
use futures::StreamExt;
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::{Pod, Secret};
use kube::api::{
    Api, DeleteParams, ListParams, PostParams, PropagationPolicy, WatchEvent, WatchParams,
};
use serde_json::Value;
use std::collections::HashMap;
use std::env::var;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};

impl K8sJobMachine<state::Initialized> {
    /// Adopt.
    pub fn adopt(self, job_name: String) -> K8sJobMachine<state::Running> {
        info!(
            "Adopting orphaned K8s job {} in namespace {}",
            job_name, self.metadata.namespace
        );
        K8sJobMachine {
            client: self.client,
            nats: self.nats,
            metadata: self.metadata.clone(),
            state: state::Running {
                job_name,
                dispatched_at: self.metadata.received_at,
            },
        }
    }

    #[allow(dead_code)]
    /// Clean up.
    pub async fn clean_up(self, job_name: &str) -> Result<()> {
        info!(
            "Cleaning up K8s job {} in namespace {}",
            job_name, self.metadata.namespace
        );
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.metadata.namespace);
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &self.metadata.namespace);
        let dp = DeleteParams {
            propagation_policy: Some(PropagationPolicy::Background),
            ..Default::default()
        };
        jobs.delete(job_name, &dp).await?;
        // Owned by the Job and so collected with it, but there is no reason to wait
        for secret_name in [
            format!("{}-auth", job_name),
            k8s_utils::step_secret_name(job_name),
        ] {
            if let Err(e) = secrets.delete(&secret_name, &dp).await {
                if !matches!(e, kube::Error::Api(ref err) if err.code == 404) {
                    return Err(e.into());
                }
            }
        }
        Ok(())
    }

    /// Starts the Kubernetes job, transitioning to either Running or Failed state.
    pub async fn start(self) -> Result<StartResult> {
        let job_name = k8s_utils::job_name(&self.metadata.step_dsl.name, self.metadata.step_id);

        info!(
            "Starting K8s job {} in namespace {}",
            job_name, self.metadata.namespace
        );

        // Version check if specified
        if self.metadata.step_dsl.r#type == "RunK8sJob" {
            if let Ok(spec) =
                serde_json::from_value::<super::K8sJobSpec>(self.metadata.step_dsl.spec.clone())
            {
                if let Some(min_v) = spec.minimum_version {
                    if let Err(e) = Self::do_check_version(&self.metadata.cluster_version, &min_v) {
                        warn!("Version check failed for {}: {}", job_name, e);
                        return Ok(StartResult::Failed(K8sJobMachine {
                            client: self.client,
                            nats: self.nats,
                            metadata: self.metadata,
                            state: state::Finished {
                                result: JobState::Failed(
                                    format!("Version mismatch: {}", e),
                                    JobMetrics::default(),
                                ),
                            },
                        }));
                    }
                }
            }
        }

        let agent_image = var("STORMCHASER_AGENT_IMAGE").ok();
        let sfs_pvc_name = var("STORMCHASER_SFS_PVC_NAME").ok();
        let (job, step_secret) =
            k8s_utils::do_build_job_spec(&job_name, &self.metadata, agent_image, sfs_pvc_name)?;
        // Created suspended when it depends on Secrets that have to be owned by it
        let gated = job.spec.as_ref().and_then(|spec| spec.suspend) == Some(true);

        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.metadata.namespace);

        let created = match jobs.create(&PostParams::default(), &job).await {
            Ok(created) => created,
            Err(kube::Error::Api(ref err)) if err.code == 409 => {
                tracing::warn!("Job {} already exists, re-attaching", job_name);
                match jobs.get(&job_name).await {
                    Ok(existing) => existing,
                    Err(e) => {
                        return Ok(self.failed(format!(
                            "Failed to read existing K8s job {}: {}",
                            job_name, e
                        )))
                    }
                }
            }
            Err(e) => {
                error!("Failed to create K8s job {}: {:?}", job_name, e);
                return Ok(self.failed(format!("K8s API error: {:?}", e)));
            }
        };

        // The Job's Secrets are created after it so that they can be owned by it: deleting
        // the Job, by the reaper or its TTL, then removes the step's environment values
        // and registry credentials too. The Job is suspended until they exist, so its pod
        // never starts without them. Each step is safe to repeat: after a crash the task
        // is redelivered and this runs again against the Job that is already there.
        let mut secrets_to_create = Vec::new();
        if !step_secret.is_empty() {
            secrets_to_create.push(step_secret.into_secret(&job_name, &created));
        }
        if let Some(auth) = &self.metadata.registry_auth {
            secrets_to_create.push(build_pull_secret(&job_name, auth, &created));
        }
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &self.metadata.namespace);
        for secret in &secrets_to_create {
            let name = secret.metadata.name.clone().unwrap_or_default();
            if let Err(e) = secrets.create(&PostParams::default(), secret).await {
                if !matches!(e, kube::Error::Api(ref err) if err.code == 409) {
                    self.delete_unstarted_job(&jobs, &job_name).await;
                    return Ok(self.failed(format!("Failed to create secret {}: {}", name, e)));
                }
            }
        }

        if gated {
            let resume = serde_json::json!({ "spec": { "suspend": false } });
            if let Err(e) = jobs
                .patch(
                    &job_name,
                    &kube::api::PatchParams::default(),
                    &kube::api::Patch::Merge(&resume),
                )
                .await
            {
                self.delete_unstarted_job(&jobs, &job_name).await;
                return Ok(self.failed(format!("Failed to start K8s job {}: {}", job_name, e)));
            }
        }

        Ok(StartResult::Running(K8sJobMachine {
            client: self.client,
            nats: self.nats,
            metadata: self.metadata,
            state: state::Running {
                job_name,
                dispatched_at: Utc::now(),
            },
        }))
    }

    /// Removes a Job that could not be given what it needs to run. Its Secrets, being
    /// owned by it, go with it.
    async fn delete_unstarted_job(&self, jobs: &Api<Job>, job_name: &str) {
        let dp = DeleteParams {
            propagation_policy: Some(PropagationPolicy::Background),
            ..Default::default()
        };
        if let Err(e) = jobs.delete(job_name, &dp).await {
            error!(
                "Failed to delete K8s job {} after it could not be started: {:?}",
                job_name, e
            );
        }
    }

    fn failed(self, reason: String) -> StartResult {
        StartResult::Failed(K8sJobMachine {
            client: self.client,
            nats: self.nats,
            metadata: self.metadata,
            state: state::Finished {
                result: JobState::Failed(reason, JobMetrics::default()),
            },
        })
    }
}

/// An owner reference that makes Kubernetes delete an object together with a step's Job.
pub(crate) fn job_owner_reference(
    job_name: &str,
    owner: &Job,
) -> Option<Vec<k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference>> {
    let uid = owner.metadata.uid.as_ref()?;
    Some(vec![
        k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference {
            api_version: "batch/v1".to_string(),
            kind: "Job".to_string(),
            name: job_name.to_string(),
            uid: uid.clone(),
            controller: Some(false),
            block_owner_deletion: Some(false),
        },
    ])
}

/// Builds the per-step image pull secret, owned by the step's Job so that Kubernetes
/// deletes it with the Job.
pub(crate) fn build_pull_secret(job_name: &str, auth: &serde_json::Value, owner: &Job) -> Secret {
    use base64::{engine::general_purpose, Engine as _};
    use std::collections::BTreeMap;

    let username = auth.get("username").and_then(|v| v.as_str()).unwrap_or("");
    let password = auth.get("password").and_then(|v| v.as_str()).unwrap_or("");
    let url = auth
        .get("url")
        .and_then(|v| v.as_str())
        .filter(|url| !url.is_empty())
        .unwrap_or("https://index.docker.io/v1/");

    let auth_base64 = general_purpose::STANDARD.encode(format!("{}:{}", username, password));

    let dockerconfigjson = serde_json::json!({
        "auths": {
            url: {
                "username": username,
                "password": password,
                "auth": auth_base64
            }
        }
    });

    let mut secret = Secret::default();
    secret.metadata.name = Some(format!("{}-auth", job_name));
    secret.metadata.labels = Some(BTreeMap::from([(
        "managed-by".to_string(),
        "stormchaser".to_string(),
    )]));
    secret.metadata.owner_references = job_owner_reference(job_name, owner);
    secret.type_ = Some("kubernetes.io/dockerconfigjson".to_string());
    secret.data = Some(BTreeMap::from([(
        ".dockerconfigjson".to_string(),
        k8s_openapi::ByteString(
            serde_json::to_vec(&dockerconfigjson).expect("a JSON value always serializes"),
        ),
    )]));
    secret
}

impl K8sJobMachine<state::Running> {
    /// Waits for the job to finish, telling its agent sidecars along the way when the
    /// step's own container is done.
    pub async fn wait(self) -> Result<JobState> {
        tokio::select! {
            result = self.wait_for_job() => result,
            never = self.signal_sidecars() => match never {},
        }
    }

    async fn wait_for_job(&self) -> Result<JobState> {
        let job_name = &self.state.job_name;
        let dispatched_at = self.state.dispatched_at;
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.metadata.namespace);

        info!("Watching K8s job {} until completion...", job_name);

        let mut watch_failures = 0;
        let max_watch_failures = 3;

        loop {
            let wp = WatchParams::default()
                .fields(&format!("metadata.name={}", job_name))
                .timeout(60);
            let mut stream = jobs.watch(&wp, "0").await?.boxed();

            while let Some(status) = stream.next().await {
                match status {
                    Ok(event) => {
                        watch_failures = 0;
                        match event {
                            WatchEvent::Modified(job) | WatchEvent::Added(job) => {
                                if let Some(state) = self
                                    .process_job_result(job, job_name, dispatched_at)
                                    .await?
                                {
                                    return Ok(state);
                                }
                            }
                            WatchEvent::Deleted(job) => {
                                return Err(anyhow::anyhow!(
                                    "Job {} was deleted during watch",
                                    job.metadata.name.unwrap_or_default()
                                ));
                            }
                            _ => {}
                        }
                    }
                    Err(e) => {
                        error!("Error from watch stream for {}: {:?}", job_name, e);
                        watch_failures += 1;
                        break; // Exit inner loop to restart or fallback
                    }
                }
            }

            if watch_failures >= max_watch_failures {
                warn!(
                    "Watch stream for {} failed {} times, falling back to polling",
                    job_name, watch_failures
                );
                return self
                    .poll_job_until_completion(jobs, job_name, dispatched_at)
                    .await;
            }

            sleep(Duration::from_secs(2)).await;

            if let Some(job) = jobs.get_opt(job_name).await? {
                if let Some(state) = self
                    .process_job_result(job, job_name, dispatched_at)
                    .await?
                {
                    return Ok(state);
                }
            } else {
                return Err(anyhow::anyhow!(
                    "Job {} vanished during watch reconnection",
                    job_name
                ));
            }
        }
    }

    async fn poll_job_until_completion(
        &self,
        jobs: Api<Job>,
        job_name: &str,
        dispatched_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<JobState> {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            interval.tick().await;
            info!("Polling K8s job {} status...", job_name);

            if let Some(job) = jobs.get_opt(job_name).await? {
                if let Some(state) = self
                    .process_job_result(job, job_name, dispatched_at)
                    .await?
                {
                    return Ok(state);
                }
            } else {
                return Err(anyhow::anyhow!("Job {} vanished during polling", job_name));
            }
        }
    }

    async fn process_job_result(
        &self,
        job: Job,
        job_name: &str,
        dispatched_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<JobState>> {
        if let Some(status) = job.status {
            let latency_ms = (dispatched_at - self.metadata.received_at)
                .num_milliseconds()
                .max(0) as u64;
            let duration_ms = (Utc::now() - dispatched_at).num_milliseconds().max(0) as u64;

            if status.succeeded.unwrap_or(0) > 0 {
                info!("Job {} completed successfully", job_name);
                let metrics =
                    self.get_pod_metrics(job_name)
                        .await
                        .unwrap_or(k8s_utils::PodMetrics {
                            exit_code: Some(0),
                            attempts: 1,
                            failure_reason: None,
                        });
                let storage_hashes = self.get_storage_hashes(job_name).await.unwrap_or(None);
                let artifacts = self.get_artifact_meta(job_name).await.unwrap_or(None);
                let test_reports = self.get_test_reports(job_name).await.unwrap_or(None);

                return Ok(Some(JobState::Succeeded(JobMetrics {
                    exit_code: metrics.exit_code,
                    attempts: metrics.attempts,
                    duration_ms,
                    latency_ms,
                    storage_hashes,
                    artifacts,
                    test_reports,
                })));
            }
            if status.failed.unwrap_or(0) > 0 {
                let metrics =
                    self.get_pod_metrics(job_name)
                        .await
                        .unwrap_or(k8s_utils::PodMetrics {
                            exit_code: None,
                            attempts: 1,
                            failure_reason: None,
                        });
                let storage_hashes = self.get_storage_hashes(job_name).await.unwrap_or(None);
                let artifacts = self.get_artifact_meta(job_name).await.unwrap_or(None);
                let test_reports = self.get_test_reports(job_name).await.unwrap_or(None);

                let conditions = status.conditions.unwrap_or_default();
                if conditions.iter().any(|c| c.type_ == "Failed") {
                    let job_reason = conditions
                        .iter()
                        .find(|c| c.type_ == "Failed")
                        .and_then(|c| c.message.clone())
                        .unwrap_or_else(|| "Job failed".to_string());

                    let reason = metrics.failure_reason.unwrap_or(job_reason);
                    error!("Job {} failed: {}", job_name, reason);
                    return Ok(Some(JobState::Failed(
                        reason,
                        JobMetrics {
                            exit_code: metrics.exit_code,
                            attempts: metrics.attempts,
                            duration_ms,
                            latency_ms,
                            storage_hashes,
                            artifacts,
                            test_reports,
                        },
                    )));
                }
            }
        }
        Ok(None)
    }

    async fn get_pod_metrics(&self, job_name: &str) -> Result<k8s_utils::PodMetrics> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.metadata.namespace);
        let lp = ListParams::default().labels(&format!("job-name={}", job_name));
        let pod_list = pods.list(&lp).await?;

        Ok(k8s_utils::do_extract_pod_metrics_with_reason(
            pod_list.items,
        ))
    }

    async fn get_storage_hashes(&self, job_name: &str) -> Result<Option<HashMap<String, String>>> {
        if self.metadata.storage.is_none() {
            return Ok(None);
        }

        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.metadata.namespace);
        let lp = ListParams::default().labels(&format!("job-name={}", job_name));
        let pod_list = pods.list(&lp).await?;

        if let Some(pod) = pod_list.items.first() {
            let pod_name = pod.metadata.name.as_ref().context("Pod missing name")?;
            let logs = pods
                .logs(
                    pod_name,
                    &kube::api::LogParams {
                        // Only the agent sidecar's output: the step cannot write to it
                        container: Some(k8s_utils::AGENT_SIDECAR_CONTAINER.to_string()),
                        ..Default::default()
                    },
                )
                .await?;

            for line in logs.lines() {
                if line.contains("Parked storage hashes:") {
                    if let Some(json_start) = line.find('{') {
                        let json_part = &line[json_start..];
                        if let Ok(hashes) = serde_json::from_str(json_part) {
                            return Ok(Some(hashes));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    async fn get_artifact_meta(&self, job_name: &str) -> Result<Option<HashMap<String, Value>>> {
        if self.metadata.storage.is_none() {
            return Ok(None);
        }

        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.metadata.namespace);
        let lp = ListParams::default().labels(&format!("job-name={}", job_name));
        let pod_list = pods.list(&lp).await?;

        if let Some(pod) = pod_list.items.first() {
            let pod_name = pod.metadata.name.as_ref().context("Pod missing name")?;
            let logs = pods
                .logs(
                    pod_name,
                    &kube::api::LogParams {
                        // Only the agent sidecar's output: the step cannot write to it
                        container: Some(k8s_utils::AGENT_SIDECAR_CONTAINER.to_string()),
                        ..Default::default()
                    },
                )
                .await?;

            for line in logs.lines() {
                if line.contains("Parked artifacts:") {
                    if let Some(json_start) = line.find('{') {
                        let json_part = &line[json_start..];
                        if let Ok(meta) = serde_json::from_str(json_part) {
                            return Ok(Some(meta));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    async fn get_test_reports(&self, job_name: &str) -> Result<Option<HashMap<String, Value>>> {
        if self.metadata.step_dsl.reports.is_empty() {
            return Ok(None);
        }

        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.metadata.namespace);
        let lp = ListParams::default().labels(&format!("job-name={}", job_name));
        let pod_list = pods.list(&lp).await?;

        if let Some(pod) = pod_list.items.first() {
            let pod_name = pod.metadata.name.as_ref().context("Pod missing name")?;
            let logs = pods
                .logs(
                    pod_name,
                    &kube::api::LogParams {
                        container: Some("worker".to_string()),
                        ..Default::default()
                    },
                )
                .await?;

            for line in logs.lines() {
                if line.contains("Collected test reports JSON:") {
                    if let Some(json_start) = line.find('{') {
                        let json_part = &line[json_start..];
                        if let Ok(reports) = serde_json::from_str(json_part) {
                            return Ok(Some(reports));
                        }
                    }
                }
            }
        }
        Ok(None)
    }
}

impl K8sJobMachine<state::Finished> {
    /// Into result.
    pub fn into_result(self) -> JobState {
        self.state.result
    }
}

impl<S> K8sJobMachine<S> {
    /// Do check version.
    pub fn do_check_version(cluster_version: &str, min_version: &str) -> Result<()> {
        let mut current = Self::do_parse_version(cluster_version)?;
        let mut required = Self::do_parse_version(min_version)?;

        let max_len = current.len().max(required.len());
        while current.len() < max_len {
            current.push(0);
        }
        while required.len() < max_len {
            required.push(0);
        }

        if current < required {
            return Err(anyhow::anyhow!(
                "Cluster version {} is less than required version {}",
                cluster_version,
                min_version
            ));
        }
        Ok(())
    }

    /// Do parse version.
    pub fn do_parse_version(v: &str) -> Result<Vec<i32>> {
        let v = v.trim_start_matches('v').split('-').next().unwrap_or(v);
        v.split('.')
            .map(|s| {
                let s_clean: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
                s_clean.parse::<i32>().map_err(|e| {
                    anyhow::anyhow!("Failed to parse version component '{}': {}", s, e)
                })
            })
            .collect()
    }
}

mod sidecar_signal;

#[cfg(test)]
mod tests;
