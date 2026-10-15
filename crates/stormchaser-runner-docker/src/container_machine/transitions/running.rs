use crate::container_machine::{
    state, ContainerMetrics, ContainerState, DockerContainerMachine, AGENT_USER,
};
use anyhow::Result;
use bollard::models::ContainerCreateBody;
use bollard::models::HostConfig;
use bollard::query_parameters::{
    CreateContainerOptions, LogsOptions, StartContainerOptions, WaitContainerOptions,
};
use chrono::Utc;
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
use stormchaser_model::dsl::CommonContainerSpec;
use stormchaser_model::nats::{request_artifact_auth, HELPER_REGISTRY_CONFIG_PATH};
use tokio::time::sleep;
use tracing::{error, info};
use uuid::Uuid;

/// What the parking helper handed in: artifacts, storage hashes and test reports, and why
/// it failed if it did.
type ParkingResults = (
    Option<HashMap<String, Value>>,
    Option<HashMap<String, String>>,
    Option<Value>,
    Option<String>,
);

impl DockerContainerMachine<state::Running> {
    /// Waits for the running container to finish executing, collects metrics and artifacts, and cleans it up.
    pub async fn wait(self) -> Result<DockerContainerMachine<state::Finished>> {
        let container_name = self.state.container_name.clone();
        let dispatched_at = self.state.dispatched_at;
        let volumes_to_cleanup = self.state.volumes_to_cleanup.clone();
        let storage_names = self.state.storage_names.clone();
        let mounts = self.state.mounts.clone();

        let mut exit_code = None;
        // How long the step may run: its own timeout within the runner's limits, or the
        // runner's default. Nothing upstream enforces it; the runner does, here.
        let timeout = stormchaser_model::dsl::limits::StepLimits::from_env()
            .timeout(self.metadata.step_dsl.timeout.as_deref())
            .unwrap_or_else(|_| stormchaser_model::dsl::limits::StepLimits::default().max_timeout);
        let mut timed_out = false;

        if let Some(exec_id) = &self.state.exec_id {
            // Poll for exec completion
            let deadline = tokio::time::Instant::now() + timeout;
            loop {
                if tokio::time::Instant::now() > deadline {
                    // The container is shared with other steps and an exec cannot be
                    // killed through the Docker API, so the step is reported as failed
                    // and its process is left to the container's own lifetime.
                    error!("Timed out waiting for exec {} to complete", exec_id);
                    timed_out = true;
                    break;
                }
                match self.docker.inspect_exec(exec_id).await {
                    Ok(res) => {
                        if res.running != Some(true) {
                            exit_code = res.exit_code;
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Error inspecting exec {}: {:?}", exec_id, e);
                        break;
                    }
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        } else {
            let mut wait_stream = self.docker.wait_container(
                &container_name,
                Some(WaitContainerOptions {
                    condition: "not-running".to_string(),
                }),
            );

            let waited = match tokio::time::timeout(timeout, wait_stream.next()).await {
                Ok(waited) => waited,
                Err(_) => {
                    error!(
                        "Container {} ran longer than {}; stopping it",
                        container_name,
                        humantime::format_duration(timeout)
                    );
                    timed_out = true;
                    // SIGTERM, then SIGKILL after Docker's grace period
                    if let Err(e) = self.docker.stop_container(&container_name, None).await {
                        error!("Failed to stop container {}: {:?}", container_name, e);
                    }
                    None
                }
            };
            exit_code = if let Some(wait_result) = waited {
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
        }

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

        let (artifacts, storage_hashes, test_reports, parking_failure) = self
            .collect_metrics(&storage_names, mounts, spec.as_ref(), exit_code)
            .await?;

        if let Some(a) = artifacts {
            metrics.artifacts = Some(a);
        }
        if let Some(h) = storage_hashes {
            metrics.storage_hashes = Some(h);
        }
        if let Some(r) = test_reports {
            metrics.test_reports = Some(r);
        }

        // Cleanup volume and container
        // Wait a bit for log collector (Alloy) to catch the final logs before we delete the container
        sleep(Duration::from_secs(15)).await;
        // Shared containers (exec_id is Some) are reused across affinity steps; only
        // remove the container when it was created exclusively for this step.
        if self.state.exec_id.is_none() {
            let _ = self.docker.remove_container(&container_name, None).await;
        }

        for vol in volumes_to_cleanup {
            info!("Cleaning up volume: {}", vol);
            let _ = self
                .docker
                .remove_volume(&vol, None::<bollard::query_parameters::RemoveVolumeOptions>)
                .await;
        }

        let result = if timed_out {
            let error_msg = format!(
                "Step timed out after {}",
                humantime::format_duration(timeout)
            );
            error!("Container {} failed: {}", container_name, error_msg);
            ContainerState::Failed(error_msg, metrics)
        } else if exit_code == Some(0) {
            match parking_failure {
                None => {
                    info!("Container {} completed successfully", container_name);
                    ContainerState::Succeeded(metrics)
                }
                Some(error_msg) => {
                    error!("Container {} failed: {}", container_name, error_msg);
                    ContainerState::Failed(error_msg, metrics)
                }
            }
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

    async fn collect_metrics(
        &self,
        storage_names: &[String],
        mounts: Vec<bollard::models::Mount>,
        spec: Option<&CommonContainerSpec>,
        exit_code: Option<i64>,
    ) -> Result<ParkingResults> {
        // A step with no storage and no reports has nothing to hand in. Its own output is
        // never searched for results: anything a step prints is the workflow author's to
        // choose, so the only log read for results is the parking helper's.
        if storage_names.is_empty() && self.metadata.step_dsl.reports.is_empty() {
            return Ok((None, None, None, None));
        }
        self.run_parking_agent(storage_names, mounts, spec, exit_code)
            .await
    }

    async fn run_parking_agent(
        &self,
        storage_names: &[String],
        mounts: Vec<bollard::models::Mount>,
        spec: Option<&CommonContainerSpec>,
        exit_code: Option<i64>,
    ) -> Result<ParkingResults> {
        let mut artifacts_out = None;
        let mut hashes_out = None;
        let mut reports_out = None;

        let agent_image = crate::container_machine::agent_image();
        let park_container_name = format!("park-{}", Uuid::new_v4());

        let sfs_host_path = std::env::var("STORMCHASER_SFS_HOST_PATH").ok();
        let (parking_urls, mount_paths, artifact_urls) =
            self.build_parking_payloads(storage_names, spec, sfs_host_path.as_deref());

        let mut agent_args = vec![
            "run".to_string(),
            "--parking-urls".to_string(),
            serde_json::to_string(&parking_urls)?,
            "--mount-paths".to_string(),
            serde_json::to_string(&mount_paths)?,
        ];

        // Registry credentials are fetched now, not at dispatch, so that a short-lived
        // token is still good when the helper pushes.
        let registry_config = self.fetch_artifact_registry_config(&artifact_urls).await;

        if !artifact_urls.is_empty() {
            agent_args.push("--artifact-urls".to_string());
            agent_args.push(serde_json::to_string(&artifact_urls)?);
        }
        if registry_config.is_some() {
            agent_args.push("--registry-config".to_string());
            agent_args.push(HELPER_REGISTRY_CONFIG_PATH.to_string());
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

        let agent_config = ContainerCreateBody {
            image: Some(agent_image.to_string()),
            // Root, whatever the image says: it reads every file the step left behind
            user: Some(AGENT_USER.to_string()),
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
            let _ = stormchaser_model::nats::announce_step_storage(
                nats,
                self.metadata.run_id,
                self.metadata.step_id,
                &self.metadata.runner_id,
                true,
            )
            .await;
        }

        self.docker
            .create_container(
                Some(CreateContainerOptions {
                    name: Some(park_container_name.clone()),
                    ..Default::default()
                }),
                agent_config,
            )
            .await?;

        // The credentials go into the helper's filesystem before it starts. They are not
        // in its arguments or environment, so `docker inspect` does not show them, and no
        // step volume can be mounted over the path.
        if let Some(config) = &registry_config {
            if let Err(e) = place_registry_config(&self.docker, &park_container_name, config).await
            {
                error!(
                    "Could not give the parking helper its registry credentials; OCI artifacts will not be pushed: {:?}",
                    e
                );
            }
        }

        self.docker
            .start_container(&park_container_name, None::<StartContainerOptions>)
            .await?;

        let mut agent_wait_stream = self.docker.wait_container(
            &park_container_name,
            Some(WaitContainerOptions {
                condition: "not-running".to_string(),
            }),
        );

        let parking_exit = match agent_wait_stream.next().await {
            Some(Ok(response)) => Some(response.status_code),
            Some(Err(bollard::errors::Error::DockerContainerWaitError { code, .. })) => Some(code),
            _ => None,
        };
        // A volume that was not parked would hand the next step what it held before
        let parking_failure = (parking_exit != Some(0)).then(|| {
            format!(
                "Parking the step's storage and artifacts failed (helper exit code {:?})",
                parking_exit
            )
        });

        if let Ok(Some(artifacts)) = self.get_artifact_meta(&park_container_name).await {
            artifacts_out = Some(artifacts);
        }
        if let Ok(Some(hashes)) = self.get_storage_hashes(&park_container_name).await {
            hashes_out = Some(hashes);
        }
        if let Ok(Some(reports)) = self.get_test_reports(&park_container_name).await {
            reports_out = Some(reports);
        }

        // Wait for logs
        sleep(Duration::from_secs(15)).await;
        let _ = self
            .docker
            .remove_container(&park_container_name, None)
            .await;

        Ok((artifacts_out, hashes_out, reports_out, parking_failure))
    }

    /// Asks the engine for the registry credentials of this step's OCI artifact pushes,
    /// if it has any that need them.
    async fn fetch_artifact_registry_config(
        &self,
        artifact_urls: &HashMap<String, Value>,
    ) -> Option<Value> {
        let needs_auth = artifact_urls
            .values()
            .any(|artifact| artifact.get("needs_auth").and_then(Value::as_bool) == Some(true));
        if !needs_auth {
            return None;
        }
        let nats = self.nats.as_ref()?;

        match request_artifact_auth(
            nats,
            self.metadata.run_id,
            self.metadata.step_id,
            Some(&self.metadata.runner_id),
        )
        .await
        {
            Ok(config) => Some(config),
            Err(e) => {
                error!(
                    "No registry credentials for step {}; its OCI artifacts will not be pushed: {:#}",
                    self.metadata.step_id, e
                );
                None
            }
        }
    }

    async fn parse_log_json<T: serde::de::DeserializeOwned>(
        &self,
        container_name: &str,
        prefix: &str,
    ) -> Result<Option<T>> {
        let mut logs = self.docker.logs(
            container_name,
            Some(LogsOptions {
                stdout: true,
                stderr: true,
                ..Default::default()
            }),
        );

        while let Some(log_result) = logs.next().await {
            if let Ok(output) = log_result {
                let line = output.to_string();
                if line.contains(prefix) {
                    if let Some(json_start) = line.find('{') {
                        let json_part = &line[json_start..];
                        if let Ok(data) = serde_json::from_str(json_part) {
                            return Ok(Some(data));
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    async fn get_artifact_meta(
        &self,
        container_name: &str,
    ) -> Result<Option<HashMap<String, Value>>> {
        self.parse_log_json(container_name, "Parked artifacts:")
            .await
    }

    async fn get_storage_hashes(
        &self,
        container_name: &str,
    ) -> Result<Option<HashMap<String, String>>> {
        self.parse_log_json(container_name, "Parked storage hashes:")
            .await
    }

    async fn get_test_reports(&self, container_name: &str) -> Result<Option<Value>> {
        self.parse_log_json(container_name, "Collected test reports JSON:")
            .await
    }

    fn build_parking_payloads(
        &self,
        storage_names: &[String],
        spec: Option<&CommonContainerSpec>,
        sfs_host_path: Option<&str>,
    ) -> (
        HashMap<String, Value>,
        HashMap<String, String>,
        HashMap<String, Value>,
    ) {
        let mut parking_urls = HashMap::new();
        let mut mount_paths = HashMap::new();
        let mut artifact_urls = HashMap::new();

        if let Some(storage_data) = &self.metadata.storage {
            for name in storage_names {
                if let Some(urls) = storage_data.get(name) {
                    let mut cloned_urls = urls.clone();
                    if sfs_host_path.is_some() {
                        if let Some(obj) = cloned_urls.as_object_mut() {
                            obj.remove("put_url");
                        }
                    }
                    parking_urls.insert(name.clone(), cloned_urls);
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

        (parking_urls, mount_paths, artifact_urls)
    }
}

/// Writes a Docker client configuration into a created, not yet started, container.
async fn place_registry_config(
    docker: &bollard::Docker,
    container_name: &str,
    config: &Value,
) -> Result<()> {
    let archive = registry_config_archive(config)?;
    docker
        .upload_to_container(
            container_name,
            Some(
                bollard::query_parameters::UploadToContainerOptionsBuilder::new()
                    .path("/")
                    .build(),
            ),
            bollard::body_full(archive.into()),
        )
        .await?;
    Ok(())
}

/// A tar archive that, unpacked at `/`, leaves `config` at [`HELPER_REGISTRY_CONFIG_PATH`],
/// readable by root only.
fn registry_config_archive(config: &Value) -> Result<Vec<u8>> {
    let content = serde_json::to_vec(config)?;
    let file_path = HELPER_REGISTRY_CONFIG_PATH.trim_start_matches('/');
    let mut builder = tar::Builder::new(Vec::new());

    // Parent directories first, so the path exists whatever the helper image contains
    let mut dir = String::new();
    let mut parts: Vec<&str> = file_path.split('/').collect();
    parts.pop();
    for part in parts {
        dir.push_str(part);
        dir.push('/');
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_mode(0o700);
        header.set_size(0);
        builder.append_data(&mut header, &dir, std::io::empty())?;
    }

    let mut header = tar::Header::new_gnu();
    header.set_mode(0o600);
    header.set_size(content.len() as u64);
    builder.append_data(&mut header, file_path, content.as_slice())?;
    Ok(builder.into_inner()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container_machine::ContainerMetadata;

    #[test]
    fn registry_config_archive_unpacks_to_the_helper_path() {
        let config = serde_json::json!({"auths": {"r.example.com": {"registrytoken": "t"}}});
        let bytes = registry_config_archive(&config).unwrap();

        let mut archive = tar::Archive::new(bytes.as_slice());
        let mut seen = Vec::new();
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            let path = entry.path().unwrap().to_string_lossy().into_owned();
            if entry.header().entry_type().is_file() {
                assert_eq!(entry.header().mode().unwrap(), 0o600);
                let mut content = String::new();
                std::io::Read::read_to_string(&mut entry, &mut content).unwrap();
                assert_eq!(serde_json::from_str::<Value>(&content).unwrap(), config);
            }
            seen.push(path);
        }
        assert_eq!(
            seen,
            vec![
                "stormchaser/",
                "stormchaser/auth/",
                "stormchaser/auth/config.json"
            ]
        );
    }
    use bollard::query_parameters::CreateImageOptions;
    use bollard::Docker;
    use stormchaser_model::dsl::Step;

    #[tokio::test]
    async fn registry_config_reaches_the_helper_without_showing_in_its_definition() {
        let Ok(docker) = Docker::connect_with_local_defaults() else {
            return;
        };
        if docker.ping().await.is_err() {
            return;
        }
        let name = format!("regcfg-test-{}", Uuid::new_v4());
        let config =
            serde_json::json!({"auths": {"r.example.com": {"registrytoken": "s3cr3t-token"}}});

        // Any image with a shell, run as root as the runner runs its helpers; it prints
        // the file and its mode. (The agent image is not built where the tests run.)
        let image = "alpine:3";
        let mut pull = docker.create_image(
            Some(CreateImageOptions {
                from_image: Some(image.to_string()),
                ..Default::default()
            }),
            None,
            None,
        );
        while let Some(progress) = pull.next().await {
            progress.unwrap();
        }
        docker
            .create_container(
                Some(CreateContainerOptions {
                    name: Some(name.clone()),
                    ..Default::default()
                }),
                ContainerCreateBody {
                    image: Some(image.to_string()),
                    entrypoint: Some(vec!["/bin/sh".to_string(), "-c".to_string()]),
                    cmd: Some(vec![format!(
                        "stat -c %a {p} && cat {p}",
                        p = HELPER_REGISTRY_CONFIG_PATH
                    )]),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        place_registry_config(&docker, &name, &config)
            .await
            .unwrap();

        let inspect = docker.inspect_container(&name, None).await.unwrap();
        assert!(!format!("{inspect:?}").contains("s3cr3t-token"));

        docker
            .start_container(&name, None::<StartContainerOptions>)
            .await
            .unwrap();
        let _ = docker
            .wait_container(&name, None::<WaitContainerOptions>)
            .next()
            .await;
        let mut output = String::new();
        let mut logs = docker.logs(
            &name,
            Some(LogsOptions {
                stdout: true,
                stderr: true,
                ..Default::default()
            }),
        );
        while let Some(Ok(chunk)) = logs.next().await {
            output.push_str(&chunk.to_string());
        }
        let _ = docker.remove_container(&name, None).await;

        let (mode, content) = output.split_once('\n').expect(&output);
        assert_eq!(mode.trim(), "600");
        assert_eq!(serde_json::from_str::<Value>(content).unwrap(), config);
    }

    fn create_test_metadata() -> ContainerMetadata {
        let spec = serde_json::json!({
            "image": "alpine:latest",
            "command": ["echo", "hello"],
            "storage_mounts": [
                {
                    "name": "workspace",
                    "mount_path": "/workspace"
                }
            ]
        });

        let mut storage = HashMap::new();
        storage.insert(
            "workspace".to_string(),
            serde_json::json!({
                "get_url": "http://s3/get",
                "put_url": "http://s3/put"
            }),
        );

        ContainerMetadata {
            run_id: Uuid::new_v4(),
            step_id: Uuid::new_v4(),
            runner_id: "test_runner".to_string(),
            fencing_token: 0,
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
                aliases: std::collections::HashMap::new(),
                retry: None,
                timeout: None,
                allow_failure: None,
                start_marker: None,
                end_marker: None,
                outputs: vec![],
                reports: vec![],
                artifacts: None,
            },
            received_at: Utc::now(),
            encryption_key: None,
            loki_url: None,
            storage: Some(storage),
            test_report_urls: None,
            registry_auth: None,
        }
    }

    #[tokio::test]
    async fn test_build_parking_payloads_named_volume() {
        let docker = Docker::connect_with_local_defaults().unwrap();
        let metadata = create_test_metadata();
        let init_machine = DockerContainerMachine::new(docker.clone(), metadata, None);
        let machine = init_machine.adopt("test".to_string());

        let spec: CommonContainerSpec =
            serde_json::from_value(machine.metadata.step_dsl.spec.clone()).unwrap();
        let storage_names = vec!["workspace".to_string()];

        let (parking_urls, mount_paths, _) =
            machine.build_parking_payloads(&storage_names, Some(&spec), None);

        assert_eq!(parking_urls.len(), 1);
        assert_eq!(mount_paths.len(), 1);

        assert_eq!(mount_paths.get("workspace").unwrap(), "/workspace");

        let urls = parking_urls.get("workspace").unwrap();
        assert_eq!(
            urls.get("put_url").and_then(|v| v.as_str()),
            Some("http://s3/put")
        );
        assert_eq!(
            urls.get("get_url").and_then(|v| v.as_str()),
            Some("http://s3/get")
        );
    }

    #[tokio::test]
    async fn test_build_parking_payloads_host_bind() {
        let docker = Docker::connect_with_local_defaults().unwrap();
        let metadata = create_test_metadata();
        let init_machine = DockerContainerMachine::new(docker.clone(), metadata, None);
        let machine = init_machine.adopt("test".to_string());

        let spec: CommonContainerSpec =
            serde_json::from_value(machine.metadata.step_dsl.spec.clone()).unwrap();
        let storage_names = vec!["workspace".to_string()];

        let sfs_host_path = Some("/tmp/stormchaser/sfs");
        let (parking_urls, mount_paths, _) =
            machine.build_parking_payloads(&storage_names, Some(&spec), sfs_host_path);

        assert_eq!(parking_urls.len(), 1);
        assert_eq!(mount_paths.len(), 1);

        assert_eq!(mount_paths.get("workspace").unwrap(), "/workspace");

        let urls = parking_urls.get("workspace").unwrap();
        assert_eq!(
            urls.get("get_url").and_then(|v| v.as_str()),
            Some("http://s3/get")
        );
        // Should omit put_url because host bind mounts do not park their state
        assert_eq!(urls.get("put_url"), None);
    }

    /// Runs a real container that would sleep for five minutes, with a two-second
    /// timeout, and checks that the runner stops it and says why.
    #[tokio::test]
    async fn a_container_that_outlives_its_timeout_is_stopped() {
        let Ok(docker) = Docker::connect_with_local_defaults() else {
            return;
        };
        if docker.ping().await.is_err() {
            return;
        }
        let step_dsl: Step = serde_json::from_value(serde_json::json!({
            "name": "outlives-timeout",
            "type": "RunContainer",
            "params": {},
            "spec": { "image": "alpine:3", "command": ["sleep", "300"] },
            "timeout": "2s",
            "aggregation": [],
            "next": [],
            "outputs": [],
            "reports": []
        }))
        .unwrap();
        let metadata = ContainerMetadata {
            run_id: Uuid::new_v4(),
            step_id: Uuid::new_v4(),
            runner_id: "test_runner".to_string(),
            fencing_token: 0,
            step_dsl,
            received_at: chrono::Utc::now(),
            encryption_key: None,
            loki_url: None,
            storage: None,
            test_report_urls: None,
            registry_auth: None,
        };

        let started = std::time::Instant::now();
        let machine = DockerContainerMachine::new(docker.clone(), metadata, None);
        let crate::container_machine::StartResult::Running(running) =
            machine.start().await.unwrap()
        else {
            panic!("the container did not start");
        };
        let container_name = running.state.container_name.clone();
        let finished = running.wait().await.unwrap();

        match finished.state.result {
            ContainerState::Failed(reason, _) => {
                assert!(reason.contains("timed out after 2s"), "{reason}")
            }
            ContainerState::Succeeded(_) => {
                panic!("a step that timed out was reported as a success")
            }
        }
        // Far sooner than the five minutes the command asked for
        assert!(started.elapsed() < std::time::Duration::from_secs(120));
        // And nothing of it is left running
        assert!(docker
            .inspect_container(&container_name, None)
            .await
            .is_err());
    }

    /// Starts two real containers for one run and one for another, cancels the first
    /// run, and checks that its containers are gone, its steps are reported as failed,
    /// and the other run is untouched.
    #[tokio::test]
    async fn cancelling_a_run_stops_its_containers_and_no_others() {
        let Ok(docker) = Docker::connect_with_local_defaults() else {
            return;
        };
        if docker.ping().await.is_err() {
            return;
        }
        let step = |name: &str| -> Step {
            serde_json::from_value(serde_json::json!({
                "name": name,
                "type": "RunContainer",
                "params": {},
                "spec": { "image": "alpine:3", "command": ["sleep", "300"] },
                "aggregation": [],
                "next": [],
                "outputs": [],
                "reports": []
            }))
            .unwrap()
        };
        let metadata = |run_id: Uuid, name: &str| ContainerMetadata {
            run_id,
            step_id: Uuid::new_v4(),
            runner_id: "test_runner".to_string(),
            fencing_token: 0,
            step_dsl: step(name),
            received_at: chrono::Utc::now(),
            encryption_key: None,
            loki_url: None,
            storage: None,
            test_report_urls: None,
            registry_auth: None,
        };
        let cancelled_run = Uuid::new_v4();
        let other_run = Uuid::new_v4();

        let mut running = Vec::new();
        for (run_id, name) in [
            (cancelled_run, "cancel-a"),
            (cancelled_run, "cancel-b"),
            (other_run, "cancel-bystander"),
        ] {
            let machine = DockerContainerMachine::new(docker.clone(), metadata(run_id, name), None);
            let crate::container_machine::StartResult::Running(machine) =
                machine.start().await.unwrap()
            else {
                panic!("the container did not start");
            };
            running.push(machine);
        }
        let bystander = running.pop().unwrap();
        let bystander_name = bystander.state.container_name.clone();

        let started = std::time::Instant::now();
        let waits: Vec<_> = running
            .into_iter()
            .map(|machine| tokio::spawn(machine.wait()))
            .collect();
        let stopped =
            crate::container_machine::cancel::stop_run_containers(&docker, cancelled_run).await;
        assert_eq!(stopped, 2);

        for wait in waits {
            // Each step's own handler notices and reports, rather than waiting out the
            // five minutes or hanging
            let finished = tokio::time::timeout(std::time::Duration::from_secs(60), wait)
                .await
                .expect("a cancelled step never finished")
                .unwrap();
            if let Ok(finished) = finished {
                assert!(
                    matches!(finished.state.result, ContainerState::Failed(..)),
                    "a cancelled step was reported as a success"
                );
            }
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(120));

        // The other run's container is still running
        let inspected = docker
            .inspect_container(&bystander_name, None)
            .await
            .expect("the other run's container was removed");
        assert_eq!(inspected.state.and_then(|state| state.running), Some(true));
        assert_eq!(
            crate::container_machine::cancel::stop_run_containers(&docker, other_run).await,
            1
        );
    }
}
