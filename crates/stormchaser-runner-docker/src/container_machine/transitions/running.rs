use crate::container_machine::{state, ContainerMetrics, ContainerState, DockerContainerMachine};
use anyhow::Result;
use bollard::models::ContainerCreateBody;
use bollard::models::HostConfig;
use bollard::query_parameters::{
    CreateContainerOptions, LogsOptions, StartContainerOptions, WaitContainerOptions,
};
use chrono::Utc;
use cloudevents::EventBuilder;
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
use stormchaser_model::dsl::CommonContainerSpec;
use stormchaser_model::nats::{request_artifact_auth, HELPER_REGISTRY_CONFIG_PATH};
use stormchaser_model::step::StepStatus;
use stormchaser_model::APPLICATION_JSON;
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

        let mut exit_code = None;

        if let Some(exec_id) = &self.state.exec_id {
            // Poll for exec completion. Use a 24-hour deadline as a safety guard against
            // an exec that never terminates; any per-step timeout is enforced upstream.
            let deadline =
                tokio::time::Instant::now() + tokio::time::Duration::from_secs(24 * 3600);
            loop {
                if tokio::time::Instant::now() > deadline {
                    error!("Timed out waiting for exec {} to complete", exec_id);
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

            exit_code = if let Some(wait_result) = wait_stream.next().await {
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

        let (artifacts, storage_hashes, test_reports) = self
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

    async fn collect_metrics(
        &self,
        storage_names: &[String],
        mounts: Vec<bollard::models::Mount>,
        spec: Option<&CommonContainerSpec>,
        exit_code: Option<i64>,
    ) -> Result<(
        Option<HashMap<String, Value>>,
        Option<HashMap<String, String>>,
        Option<Value>,
    )> {
        // A step with no storage and no reports has nothing to hand in. Its own output is
        // never searched for results: anything a step prints is the workflow author's to
        // choose, so the only log read for results is the parking helper's.
        if storage_names.is_empty() && self.metadata.step_dsl.reports.is_empty() {
            return Ok((None, None, None));
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
    ) -> Result<(
        Option<HashMap<String, Value>>,
        Option<HashMap<String, String>>,
        Option<Value>,
    )> {
        let mut artifacts_out = None;
        let mut hashes_out = None;
        let mut reports_out = None;

        let agent_image = "stormchaser-agent:v1";
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
                "status": StepStatus::PackingSfs,
                "timestamp": Utc::now(),
            });
            if let Ok(ce) = cloudevents::EventBuilderV10::new()
                .id(uuid::Uuid::new_v4().to_string())
                .ty("stormchaser.v1.step.packing_sfs")
                .source("/stormchaser/runner")
                .time(Utc::now())
                .data(APPLICATION_JSON, packing_event)
                .build()
            {
                if let Ok(payload_bytes) = serde_json::to_vec(&ce) {
                    let _ = nats
                        .publish("stormchaser.v1.step.packing_sfs", payload_bytes.into())
                        .await;
                }
            }
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

        let _ = agent_wait_stream.next().await;

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

        Ok((artifacts_out, hashes_out, reports_out))
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
    use bollard::Docker;
    use stormchaser_model::dsl::Step;

    #[tokio::test]
    async fn registry_config_reaches_the_helper_without_showing_in_its_definition() {
        let docker = Docker::connect_with_local_defaults().unwrap();
        let name = format!("regcfg-test-{}", Uuid::new_v4());
        let config =
            serde_json::json!({"auths": {"r.example.com": {"registrytoken": "s3cr3t-token"}}});

        // The same image the runner's helpers use; it prints the file and its mode
        docker
            .create_container(
                Some(CreateContainerOptions {
                    name: Some(name.clone()),
                    ..Default::default()
                }),
                ContainerCreateBody {
                    image: Some("stormchaser-agent:v1".to_string()),
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
}
