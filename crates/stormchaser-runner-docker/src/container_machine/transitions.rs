use super::{state, ContainerMetrics, ContainerState, DockerContainerMachine, StartResult};
use anyhow::{Context, Result};
use bollard::container::{
    Config, CreateContainerOptions, LogsOptions, StartContainerOptions, WaitContainerOptions,
};
use bollard::image::CreateImageOptions;
use bollard::service::{HostConfig, Mount, MountTypeEnum};
use bollard::volume::CreateVolumeOptions;
use chrono::Utc;
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use stormchaser_model::dsl::CommonContainerSpec;
use tokio::time::sleep;
use tracing::{error, info};
use uuid::Uuid;

impl DockerContainerMachine<state::Initialized> {
    /// Adopts an already-running container by name, transitioning the state machine to `Running`.
    pub fn adopt(self, container_name: String) -> DockerContainerMachine<state::Running> {
        info!("Adopting orphaned Docker container {}", container_name);
        DockerContainerMachine {
            nats: self.nats.clone(),
            docker: self.docker,
            metadata: self.metadata,
            state: state::Running {
                container_name,
                dispatched_at: Utc::now(),
                volumes_to_cleanup: Vec::new(),
                storage_names: Vec::new(),
                mounts: Vec::new(),
            },
        }
    }

    /// Cleans up an orphaned container without running it through the state machine.
    pub async fn clean_up(self, container_name: &str) -> Result<()> {
        info!("Cleaning up orphaned Docker container {}", container_name);
        let _ = self.docker.stop_container(container_name, None).await;
        let _ = self.docker.remove_container(container_name, None).await;
        Ok(())
    }

    /// Starts a new Docker container, pulling images and unparking storage as necessary.
    pub async fn start(self) -> Result<StartResult> {
        let container_name = format!(
            "storm-{}-{}",
            self.metadata.step_dsl.name.to_lowercase().replace('_', "-"),
            &self.metadata.step_id.to_string()[..8]
        );

        let spec: CommonContainerSpec = serde_json::from_value(self.metadata.step_dsl.spec.clone())
            .context("Failed to parse RunContainer spec as CommonContainerSpec")?;

        let mut mounts = Vec::new();
        let mut storage_names = Vec::new();
        let mut volumes_to_cleanup = Vec::new();

        if let Some(storage_mounts) = &spec.storage_mounts {
            for mount in storage_mounts {
                storage_names.push(mount.name.clone());
                let volume_name = format!(
                    "sfs-{}-{}",
                    mount.name.to_lowercase().replace('_', "-"),
                    &self.metadata.run_id.to_string()[..8]
                );

                info!("Ensuring Docker volume exists: {}", volume_name);
                self.docker
                    .create_volume(CreateVolumeOptions {
                        name: volume_name.clone(),
                        ..Default::default()
                    })
                    .await?;

                volumes_to_cleanup.push(volume_name.clone());

                mounts.push(Mount {
                    target: Some(mount.mount_path.clone()),
                    source: Some(volume_name.clone()),
                    typ: Some(MountTypeEnum::VOLUME),
                    read_only: mount.read_only,
                    ..Default::default()
                });

                if let Some(storage_data) = &self.metadata.storage {
                    if let Some(urls) = storage_data.get(&mount.name) {
                        let has_state =
                            urls.get("expected_hash").and_then(|h| h.as_str()).is_some();

                        if has_state {
                            if let Some(get_url) = urls.get("get_url").and_then(|u| u.as_str()) {
                                info!(
                                    "Unparking storage '{}' (resume state) for volume '{}'",
                                    mount.name, volume_name
                                );
                                if let Some(nats) = &self.nats {
                                    let unpacking_event = serde_json::json!({
                                        "run_id": self.metadata.run_id,
                                        "step_id": self.metadata.step_id,
                                        "status": "unpacking_sfs",
                                        "timestamp": chrono::Utc::now(),
                                    });
                                    let _ = nats
                                        .publish(
                                            "stormchaser.step.unpacking_sfs",
                                            unpacking_event.to_string().into(),
                                        )
                                        .await;
                                }
                                self.unpark_storage(
                                    &volume_name,
                                    &mount.mount_path,
                                    &mount.mount_path,
                                    get_url,
                                    false,
                                )
                                .await?;
                            }
                        } else if let Some(provision) =
                            urls.get("provision").and_then(|p| p.as_array())
                        {
                            for prov in provision {
                                if let (Some(url), Some(dest)) = (
                                    prov.get("url").and_then(|u| u.as_str()),
                                    prov.get("destination").and_then(|d| d.as_str()),
                                ) {
                                    info!(
                                        "Provisioning storage '{}' from URL '{}' into destination '{}'",
                                        mount.name, url, dest
                                    );
                                    if let Some(nats) = &self.nats {
                                        let unpacking_event = serde_json::json!({
                                            "run_id": self.metadata.run_id,
                                            "step_id": self.metadata.step_id,
                                            "status": "unpacking_sfs",
                                            "timestamp": chrono::Utc::now(),
                                        });
                                        let _ = nats
                                            .publish(
                                                "stormchaser.step.unpacking_sfs",
                                                unpacking_event.to_string().into(),
                                            )
                                            .await;
                                    }
                                    let mut full_dest = PathBuf::from(&mount.mount_path);
                                    if dest != "/" && !dest.is_empty() {
                                        let relative_dest = dest.trim_start_matches('/');
                                        full_dest.push(relative_dest);
                                    }

                                    let resource_type =
                                        prov.get("resource_type").and_then(|r| r.as_str());
                                    let is_extract = resource_type != Some("artifact");
                                    info!(
                                        "Provisioning resource_type: {:?}, is_extract: {}",
                                        resource_type, is_extract
                                    );

                                    self.unpark_storage(
                                        &volume_name,
                                        &mount.mount_path,
                                        &full_dest.to_string_lossy(),
                                        url,
                                        !is_extract,
                                    )
                                    .await?;
                                }
                            }
                        }
                    }
                }
            }
        }

        info!("Pulling image {}", spec.image);
        self.pull_image(&spec.image).await?;

        let network_mode = self.get_network_mode().await;
        let config =
            self.build_container_config(&spec, mounts.clone(), network_mode, &storage_names)?;

        info!("Creating container {}", container_name);
        let dispatched_at = Utc::now();
        self.docker
            .create_container(
                Some(CreateContainerOptions {
                    name: container_name.clone(),
                    ..Default::default()
                }),
                config,
            )
            .await?;

        info!("Starting container {}", container_name);
        self.docker
            .start_container(&container_name, None::<StartContainerOptions<String>>)
            .await?;

        if let Some(nats) = &self.nats {
            let running_event = serde_json::json!({
                "run_id": self.metadata.run_id,
                "step_id": self.metadata.step_id,
                "status": "running",
                "timestamp": chrono::Utc::now(),
            });
            let _ = nats
                .publish("stormchaser.step.running", running_event.to_string().into())
                .await;
        }

        Ok(StartResult::Running(DockerContainerMachine {
            nats: self.nats.clone(),
            docker: self.docker,
            metadata: self.metadata,
            state: state::Running {
                container_name,
                dispatched_at,
                volumes_to_cleanup,
                storage_names,
                mounts,
            },
        }))
    }

    async fn unpark_storage(
        &self,
        volume_name: &str,
        volume_mount_path: &str,
        destination_path: &str,
        get_url: &str,
        no_extract: bool,
    ) -> Result<()> {
        let agent_image = "stormchaser-agent:v1";
        let unpark_container_name = format!("unpark-{}", Uuid::new_v4());

        let mut cmd = vec![
            "/usr/local/bin/stormchaser-agent".to_string(),
            "unpark".to_string(),
            "--url".to_string(),
            get_url.to_string(),
            "--destination".to_string(),
            destination_path.to_string(),
        ];
        if no_extract {
            cmd.push("--no-extract".to_string());
        }

        let config = Config {
            image: Some(agent_image.to_string()),
            cmd: Some(cmd),
            host_config: Some(HostConfig {
                mounts: Some(vec![Mount {
                    target: Some(volume_mount_path.to_string()),
                    source: Some(volume_name.to_string()),
                    typ: Some(MountTypeEnum::VOLUME),
                    ..Default::default()
                }]),
                network_mode: self.get_network_mode().await,
                ..Default::default()
            }),
            ..Default::default()
        };

        self.docker
            .create_container(
                Some(CreateContainerOptions {
                    name: unpark_container_name.clone(),
                    ..Default::default()
                }),
                config,
            )
            .await?;

        self.docker
            .start_container(
                &unpark_container_name,
                None::<StartContainerOptions<String>>,
            )
            .await?;

        let mut wait_stream = self.docker.wait_container(
            &unpark_container_name,
            Some(WaitContainerOptions {
                condition: "not-running",
            }),
        );

        if let Some(wait_result) = wait_stream.next().await {
            match wait_result {
                Ok(res) if res.status_code == 0 => {
                    info!("Unpark successful for {}", volume_name);
                    // Wait for logs
                    sleep(Duration::from_secs(15)).await;
                    let _ = self
                        .docker
                        .remove_container(&unpark_container_name, None)
                        .await;
                    Ok(())
                }
                res => {
                    error!("Unpark failed for {}: {:?}", volume_name, res);
                    Err(anyhow::anyhow!(
                        "Unpark failed for {}: {:?}",
                        volume_name,
                        res
                    ))
                }
            }
        } else {
            Err(anyhow::anyhow!("Wait stream ended unexpectedly"))
        }
    }

    async fn pull_image(&self, image: &str) -> Result<()> {
        let mut pull_stream = self.docker.create_image(
            Some(CreateImageOptions {
                from_image: image.to_string(),
                ..Default::default()
            }),
            None,
            None,
        );
        while let Some(pull_result) = pull_stream.next().await {
            if let Err(e) = pull_result {
                error!("Error pulling image {}: {:?}", image, e);
                anyhow::bail!("Failed to pull image {}: {}", image, e);
            }
        }
        Ok(())
    }
}

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

impl DockerContainerMachine<state::Finished> {
    /// Consumes the state machine, returning the final `ContainerState` result.
    pub fn into_result(self) -> ContainerState {
        self.state.result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container_machine::ContainerMetadata;
    use bollard::Docker;
    use stormchaser_model::dsl::Step;
    use uuid::Uuid;

    async fn get_docker() -> Docker {
        Docker::connect_with_local_defaults().unwrap()
    }

    fn create_test_metadata(image: &str, cmd: Vec<&str>) -> ContainerMetadata {
        let spec = serde_json::json!({
            "image": image,
            "command": cmd,
            "cpu_limit": null,
            "memory_limit": null,
            "env": null,
            "ports": null,
            "storage_mounts": null
        });

        ContainerMetadata {
            run_id: Uuid::new_v4(),
            step_id: Uuid::new_v4(),
            step_dsl: Step {
                name: "test_step".to_string(),
                r#type: "RunContainer".to_string(),
                spec,
                condition: None,
                params: HashMap::new(),
                strategy: None,
                aggregation: vec![],
                iterate: None,
                iterate_as: None,
                steps: None,
                next: vec![],
                on_failure: None,
                retry: None,
                timeout: None,
                allow_failure: None,
                start_marker: None,
                end_marker: None,
                outputs: vec![],
                reports: vec![],
                artifacts: None,
            },
            received_at: chrono::Utc::now(),
            encryption_key: None,
            storage: None,
            test_report_urls: None,
        }
    }

    #[tokio::test]
    async fn test_container_lifecycle_success() {
        let docker = get_docker().await;
        let metadata = create_test_metadata("alpine:latest", vec!["echo", "hello"]);

        let machine = DockerContainerMachine::new(docker.clone(), metadata, None);

        // Test Start
        let start_res = machine.start().await.expect("Failed to start container");
        let running_machine = match start_res {
            StartResult::Running(m) => m,
            StartResult::Failed(_) => panic!("Container failed to start"),
        };

        let container_name = running_machine.state.container_name.clone();

        // Test Wait
        let finished_machine = running_machine
            .wait()
            .await
            .expect("Failed to wait on container");
        let state = finished_machine.into_result();

        match state {
            ContainerState::Succeeded(metrics) => {
                assert_eq!(metrics.exit_code, Some(0));
            }
            ContainerState::Failed(msg, _) => panic!("Container failed: {}", msg),
        }

        // Ensure container is cleaned up
        let inspect = docker.inspect_container(&container_name, None).await;
        assert!(inspect.is_err(), "Container should be removed");
    }

    #[tokio::test]
    async fn test_container_lifecycle_failure() {
        let docker = get_docker().await;
        let metadata = create_test_metadata("alpine:latest", vec!["false"]);

        let machine = DockerContainerMachine::new(docker.clone(), metadata, None);

        let start_res = machine.start().await.expect("Failed to start container");
        let running_machine = match start_res {
            StartResult::Running(m) => m,
            StartResult::Failed(_) => panic!("Container failed to start"),
        };

        let container_name = running_machine.state.container_name.clone();

        let finished_machine = running_machine
            .wait()
            .await
            .expect("Failed to wait on container");
        let state = finished_machine.into_result();

        match state {
            ContainerState::Failed(msg, metrics) => {
                assert!(
                    msg.contains("Some(1)"),
                    "Unexpected failure message: {}",
                    msg
                );
                assert_eq!(metrics.exit_code, Some(1));
            }
            ContainerState::Succeeded(_) => panic!("Container should have failed"),
        }

        // Ensure container is cleaned up
        let inspect = docker.inspect_container(&container_name, None).await;
        assert!(inspect.is_err(), "Container should be removed");
    }

    #[tokio::test]
    async fn test_adopt_container() {
        let docker = get_docker().await;
        let metadata = create_test_metadata("alpine:latest", vec!["echo", "hello"]);
        let machine = DockerContainerMachine::new(docker.clone(), metadata, None);

        let running_machine = machine.adopt("my-adopted-container".to_string());
        assert_eq!(running_machine.state.container_name, "my-adopted-container");
        assert!(running_machine.state.volumes_to_cleanup.is_empty());
    }

    #[tokio::test]
    async fn test_clean_up_orphaned_container() {
        let docker = get_docker().await;
        let container_name = format!("test-cleanup-{}", Uuid::new_v4());

        // Create a real container to clean up
        use futures::StreamExt;
        let _ = docker
            .create_image(
                Some(bollard::image::CreateImageOptions {
                    from_image: "alpine:latest",
                    ..Default::default()
                }),
                None,
                None,
            )
            .collect::<Vec<_>>()
            .await;

        let _ = docker
            .create_container(
                Some(bollard::container::CreateContainerOptions {
                    name: container_name.as_str(),
                    ..Default::default()
                }),
                bollard::container::Config {
                    image: Some("alpine:latest".to_string()),
                    cmd: Some(vec!["sleep".to_string(), "1000".to_string()]),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let metadata = create_test_metadata("alpine:latest", vec!["sleep", "1000"]);
        let machine = DockerContainerMachine::new(docker.clone(), metadata, None);

        machine.clean_up(&container_name).await.unwrap();

        // Verify it is gone
        let inspect = docker.inspect_container(&container_name, None).await;
        assert!(inspect.is_err(), "Container should be cleaned up");
    }

    #[tokio::test]
    async fn test_into_result() {
        use super::super::{state, ContainerMetrics, ContainerState, DockerContainerMachine};
        let docker = get_docker().await;
        let metadata = create_test_metadata("alpine:latest", vec!["echo", "hello"]);
        let machine = DockerContainerMachine {
            nats: None,
            docker,
            metadata,
            state: state::Finished {
                result: ContainerState::Succeeded(ContainerMetrics::default()),
            },
        };
        let state = machine.into_result();
        match state {
            ContainerState::Succeeded(_) => {}
            _ => panic!("Expected Succeeded"),
        }
    }
}
