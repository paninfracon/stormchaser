use crate::container_machine::{state, ContainerMetrics, ContainerState, DockerContainerMachine};
use anyhow::Result;
use bollard::container::{
    Config, CreateContainerOptions, LogsOptions, StartContainerOptions, WaitContainerOptions,
};
use bollard::service::HostConfig;
use chrono::Utc;
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
use stormchaser_model::dsl::CommonContainerSpec;
use tokio::time::sleep;
use tracing::{error, info};
use uuid::Uuid;

impl DockerContainerMachine<state::Running> {
    /// Waits for the running container to finish executing, collects metrics and artifacts, and cleans it up.
    pub async fn wait(self) -> Result<DockerContainerMachine<state::Finished>> {
        let container_name = self.state.container_name.clone();
        let dispatched_at = self.state.dispatched_at;
        let volumes_to_cleanup = self.state.volumes_to_cleanup.clone();
        let storage_names = self.state.storage_names.clone();
        let mounts = self.state.mounts.clone();

        let mut wait_stream = self.docker.wait_container(
            &container_name,
            Some(WaitContainerOptions {
                condition: "not-running",
            }),
        );

        let exit_code = if let Some(wait_result) = wait_stream.next().await {
            match wait_result {
                Ok(response) => Some(response.status_code),
                Err(bollard::errors::Error::DockerContainerWaitError { error: _, code }) => {
                    Some(code)
                }
                Err(e) => {
                    error!("Error waiting for container {}: {:?}", container_name, e);
                    None
                }
            }
        } else {
            None
        };

        let latency_ms = (dispatched_at - self.metadata.received_at)
            .num_milliseconds()
            .max(0) as u64;
        let duration_ms = (Utc::now() - dispatched_at).num_milliseconds().max(0) as u64;

        let mut metrics = ContainerMetrics {
            exit_code,
            duration_ms,
            latency_ms,
            ..Default::default()
        };

        // Capture metadata
        let spec: Option<CommonContainerSpec> =
            serde_json::from_value(self.metadata.step_dsl.spec.clone()).ok();

        if !storage_names.is_empty() || !self.metadata.step_dsl.reports.is_empty() {
            let agent_image = "stormchaser-agent:v1";
            let park_container_name = format!("park-{}", Uuid::new_v4());

            let mut parking_urls = HashMap::new();
            let mut mount_paths = HashMap::new();
            let mut artifact_urls = HashMap::new();

            if let Some(storage_data) = &self.metadata.storage {
                for name in &storage_names {
                    if let Some(urls) = storage_data.get(name) {
                        parking_urls.insert(name.clone(), urls.clone());
                        if let Some(artifacts) = urls.get("artifacts").and_then(|a| a.as_object()) {
                            let allowed_artifacts = self.metadata.step_dsl.artifacts.as_ref();
                            for (art_name, art_val) in artifacts {
                                if let Some(allowed) = allowed_artifacts {
                                    if !allowed.contains(art_name) {
                                        continue;
                                    }
                                } else {
                                    // If step.artifacts is not explicitly defined, we assume it publishes NO artifacts
                                    continue;
                                }

                                let mut cloned_art = art_val.clone();
                                if let Some(m) = spec
                                    .as_ref()
                                    .and_then(|s| s.storage_mounts.as_ref())
                                    .and_then(|ms| ms.iter().find(|m| &m.name == name))
                                {
                                    if let Some(p) = art_val.get("path").and_then(|p| p.as_str()) {
                                        let abs_path = std::path::Path::new(&m.mount_path).join(p);
                                        cloned_art["path"] =
                                            Value::String(abs_path.to_string_lossy().to_string());
                                    }
                                }
                                artifact_urls.insert(art_name.clone(), cloned_art);
                            }
                        }
                    }
                    if let Some(m) = spec
                        .as_ref()
                        .and_then(|s| s.storage_mounts.as_ref())
                        .and_then(|ms| ms.iter().find(|m| &m.name == name))
                    {
                        mount_paths.insert(name.clone(), m.mount_path.clone());
                    }
                }
            }

            let mut agent_args = vec![
                "run".to_string(),
                "--parking-urls".to_string(),
                serde_json::to_string(&parking_urls)?,
                "--mount-paths".to_string(),
                serde_json::to_string(&mount_paths)?,
            ];

            if !artifact_urls.is_empty() {
                agent_args.push("--artifact-urls".to_string());
                agent_args.push(serde_json::to_string(&artifact_urls)?);
            }

            if let Some(reports) = &self.metadata.test_report_urls {
                if !reports.is_empty() {
                    agent_args.push("--report-urls".to_string());
                    agent_args.push(serde_json::to_string(reports)?);
                }
            }

            if !self.metadata.step_dsl.reports.is_empty() {
                agent_args.push("--test-reports".to_string());
                agent_args.push(serde_json::to_string(&self.metadata.step_dsl.reports)?);
            }

            if exit_code != Some(0) {
                // If it failed, don't fail parking
                // (This matches original logic but might need review)
            }

            agent_args.push("--".to_string());
            agent_args.push("/bin/true".to_string());

            let agent_config = Config {
                image: Some(agent_image.to_string()),
                cmd: Some(agent_args),
                entrypoint: Some(vec!["/usr/local/bin/stormchaser-agent".to_string()]),
                host_config: Some(HostConfig {
                    mounts: Some(mounts),
                    network_mode: self.get_network_mode().await,
                    ..Default::default()
                }),
                ..Default::default()
            };

            info!("Running agent for parking/reports: {}", park_container_name);
            if let Some(nats) = &self.nats {
                let packing_event = serde_json::json!({
                    "run_id": self.metadata.run_id,
                    "step_id": self.metadata.step_id,
                    "status": "packing_sfs",
                    "timestamp": chrono::Utc::now(),
                });
                let _ = nats
                    .publish(
                        "stormchaser.step.packing_sfs",
                        packing_event.to_string().into(),
                    )
                    .await;
            }

            self.docker
                .create_container(
                    Some(CreateContainerOptions {
                        name: park_container_name.clone(),
                        ..Default::default()
                    }),
                    agent_config,
                )
                .await?;

            self.docker
                .start_container(&park_container_name, None::<StartContainerOptions<String>>)
                .await?;

            let mut agent_wait_stream = self.docker.wait_container(
                &park_container_name,
                Some(WaitContainerOptions {
                    condition: "not-running",
                }),
            );

            let _ = agent_wait_stream.next().await;

            if let Ok(Some(artifacts)) = self.get_artifact_meta(&park_container_name).await {
                metrics.artifacts = Some(artifacts);
            }
            if let Ok(Some(hashes)) = self.get_storage_hashes(&park_container_name).await {
                metrics.storage_hashes = Some(hashes);
            }
            if let Ok(Some(reports)) = self.get_test_reports(&park_container_name).await {
                metrics.test_reports = Some(reports);
            }

            // Wait for logs
            sleep(Duration::from_secs(15)).await;
            let _ = self
                .docker
                .remove_container(&park_container_name, None)
                .await;
        } else {
            // For adopted containers without parking, we still want to grab logs if possible
            if let Ok(Some(artifacts)) = self.get_artifact_meta(&container_name).await {
                metrics.artifacts = Some(artifacts);
            }
            if let Ok(Some(hashes)) = self.get_storage_hashes(&container_name).await {
                metrics.storage_hashes = Some(hashes);
            }
            if let Ok(Some(reports)) = self.get_test_reports(&container_name).await {
                metrics.test_reports = Some(reports);
            }
        }

        // Cleanup volume and container
        // Wait a bit for log collector (Alloy) to catch the final logs before we delete the container
        sleep(Duration::from_secs(15)).await;
        let _ = self.docker.remove_container(&container_name, None).await;

        for vol in volumes_to_cleanup {
            info!("Cleaning up volume: {}", vol);
            let _ = self.docker.remove_volume(&vol, None).await;
        }

        let result = if exit_code == Some(0) {
            info!("Container {} completed successfully", container_name);
            ContainerState::Succeeded(metrics)
        } else {
            let error_msg = format!("Container exited with code {:?}", exit_code);
            error!("Container {} failed: {}", container_name, error_msg);
            ContainerState::Failed(error_msg, metrics)
        };

        Ok(DockerContainerMachine {
            nats: self.nats.clone(),
            docker: self.docker,
            metadata: self.metadata,
            state: state::Finished { result },
        })
    }

    async fn get_artifact_meta(
        &self,
        container_name: &str,
    ) -> Result<Option<HashMap<String, Value>>> {
        let mut logs = self.docker.logs(
            container_name,
            Some(LogsOptions::<String> {
                stdout: true,
                stderr: true,
                ..Default::default()
            }),
        );

        while let Some(log_result) = logs.next().await {
            if let Ok(output) = log_result {
                let line = output.to_string();
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

    async fn get_storage_hashes(
        &self,
        container_name: &str,
    ) -> Result<Option<HashMap<String, String>>> {
        let mut logs = self.docker.logs(
            container_name,
            Some(LogsOptions::<String> {
                stdout: true,
                stderr: true,
                ..Default::default()
            }),
        );

        while let Some(log_result) = logs.next().await {
            if let Ok(output) = log_result {
                let line = output.to_string();
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

    async fn get_test_reports(&self, container_name: &str) -> Result<Option<Value>> {
        let mut logs = self.docker.logs(
            container_name,
            Some(LogsOptions::<String> {
                stdout: true,
                stderr: true,
                ..Default::default()
            }),
        );

        while let Some(log_result) = logs.next().await {
            if let Ok(output) = log_result {
                let line = output.to_string();
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
