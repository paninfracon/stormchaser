use super::{k8s_utils, state, JobMetrics, JobState, K8sJobMachine, StartResult};
use anyhow::{Context, Result};
use chrono::Utc;
use futures::StreamExt;
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{
    Api, DeleteParams, ListParams, PostParams, PropagationPolicy, WatchEvent, WatchParams,
};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};

impl K8sJobMachine<state::Initialized> {
    pub fn adopt(self, job_name: String) -> K8sJobMachine<state::Running> {
        info!(
            "Adopting orphaned K8s job {} in namespace {}",
            job_name, self.metadata.namespace
        );
        K8sJobMachine {
            client: self.client,
            metadata: self.metadata.clone(),
            state: state::Running {
                job_name,
                dispatched_at: self.metadata.received_at,
            },
        }
    }

    #[allow(dead_code)]
    pub async fn clean_up(self, job_name: &str) -> Result<()> {
        info!(
            "Cleaning up K8s job {} in namespace {}",
            job_name, self.metadata.namespace
        );
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.metadata.namespace);
        let dp = DeleteParams {
            propagation_policy: Some(PropagationPolicy::Background),
            ..Default::default()
        };
        jobs.delete(job_name, &dp).await?;
        Ok(())
    }

    pub async fn start(self) -> Result<StartResult> {
        let job_name = format!(
            "storm-{}-{}",
            self.metadata.step_dsl.name.to_lowercase().replace('_', "-"),
            &self.metadata.step_id.to_string()[..8]
        );

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

        let agent_image = std::env::var("STORMCHASER_AGENT_IMAGE").ok();
        let sfs_pvc_name = std::env::var("STORMCHASER_SFS_PVC_NAME").ok();
        let job =
            k8s_utils::do_build_job_spec(&job_name, &self.metadata, agent_image, sfs_pvc_name)?;

        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.metadata.namespace);

        match jobs.create(&PostParams::default(), &job).await {
            Ok(_) => Ok(StartResult::Running(K8sJobMachine {
                client: self.client,
                metadata: self.metadata,
                state: state::Running {
                    job_name,
                    dispatched_at: Utc::now(),
                },
            })),
            Err(kube::Error::Api(ref err)) if err.code == 409 => {
                tracing::warn!("Job {} already exists, re-attaching", job_name);
                Ok(StartResult::Running(K8sJobMachine {
                    client: self.client,
                    metadata: self.metadata,
                    state: state::Running {
                        job_name,
                        dispatched_at: Utc::now(),
                    },
                }))
            }
            Err(e) => {
                error!("Failed to create K8s job {}: {:?}", job_name, e);
                Ok(StartResult::Failed(K8sJobMachine {
                    client: self.client,
                    metadata: self.metadata,
                    state: state::Finished {
                        result: JobState::Failed(
                            format!("K8s API error: {:?}", e),
                            JobMetrics::default(),
                        ),
                    },
                }))
            }
        }
    }
}

impl K8sJobMachine<state::Running> {
    pub async fn wait(self) -> Result<JobState> {
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
                let (exit_code, attempts, _) =
                    self.get_pod_metrics(job_name)
                        .await
                        .unwrap_or((Some(0), 1, None));
                let storage_hashes = self.get_storage_hashes(job_name).await.unwrap_or(None);
                let artifacts = self.get_artifact_meta(job_name).await.unwrap_or(None);
                let test_reports = self.get_test_reports(job_name).await.unwrap_or(None);

                return Ok(Some(JobState::Succeeded(JobMetrics {
                    exit_code,
                    attempts,
                    duration_ms,
                    latency_ms,
                    storage_hashes,
                    artifacts,
                    test_reports,
                })));
            }
            if status.failed.unwrap_or(0) > 0 {
                let (exit_code, attempts, pod_reason) = self
                    .get_pod_metrics(job_name)
                    .await
                    .unwrap_or((None, 1, None));
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

                    let reason = pod_reason.unwrap_or(job_reason);
                    error!("Job {} failed: {}", job_name, reason);
                    return Ok(Some(JobState::Failed(
                        reason,
                        JobMetrics {
                            exit_code,
                            attempts,
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

    async fn get_pod_metrics(&self, job_name: &str) -> Result<(Option<i32>, i32, Option<String>)> {
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
                        container: Some("worker".to_string()),
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
                        container: Some("worker".to_string()),
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
    pub fn into_result(self) -> JobState {
        self.state.result
    }
}

impl<S> K8sJobMachine<S> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_do_parse_version() {
        assert_eq!(
            K8sJobMachine::<state::Initialized>::do_parse_version("1.2.3").unwrap(),
            vec![1, 2, 3]
        );
        assert_eq!(
            K8sJobMachine::<state::Initialized>::do_parse_version("v1.2.3").unwrap(),
            vec![1, 2, 3]
        );
        assert_eq!(
            K8sJobMachine::<state::Initialized>::do_parse_version("v1.2.3-alpha").unwrap(),
            vec![1, 2, 3]
        );
        assert_eq!(
            K8sJobMachine::<state::Initialized>::do_parse_version("1.25.0+xyz").unwrap(),
            vec![1, 25, 0]
        );
        assert!(K8sJobMachine::<state::Initialized>::do_parse_version("invalid").is_err());
    }

    #[test]
    fn test_do_check_version() {
        assert!(K8sJobMachine::<state::Initialized>::do_check_version("1.25.0", "1.24.0").is_ok());
        assert!(K8sJobMachine::<state::Initialized>::do_check_version("1.25.0", "1.25.0").is_ok());
        assert!(K8sJobMachine::<state::Initialized>::do_check_version("1.25.1", "1.25.0").is_ok());

        let err =
            K8sJobMachine::<state::Initialized>::do_check_version("1.24.0", "1.25.0").unwrap_err();
        assert!(err.to_string().contains("less than required version"));
    }
}
