use crate::container_machine::{state, ContainerMetrics, ContainerState, DockerContainerMachine};
use anyhow::Result;
use bollard::container::{
    Config, CreateContainerOptions, LogsOptions, StartContainerOptions, WaitContainerOptions,
};
use bollard::service::HostConfig;
use chrono::Utc;
use cloudevents::EventBuilder;
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
use stormchaser_model::dsl::CommonContainerSpec;
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
            loop {
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
                    condition: "not-running",
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
            .collect_metrics(
                &container_name,
                &storage_names,
                mounts,
                spec.as_ref(),
                exit_code,
            )
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

    async fn collect_metrics(
        &self,
        container_name: &str,
        storage_names: &[String],
        mounts: Vec<bollard::service::Mount>,
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

        if !storage_names.is_empty() || !self.metadata.step_dsl.reports.is_empty() {
            let (a, h, r) = self
                .run_parking_agent(storage_names, mounts, spec, exit_code)
                .await?;
            artifacts_out = a;
            hashes_out = h;
            reports_out = r;
        } else {
            // For adopted containers without parking, we still want to grab logs if possible
            if let Ok(Some(artifacts)) = self.get_artifact_meta(container_name).await {
                artifacts_out = Some(artifacts);
            }
            if let Ok(Some(hashes)) = self.get_storage_hashes(container_name).await {
                hashes_out = Some(hashes);
            }
            if let Ok(Some(reports)) = self.get_test_reports(container_name).await {
                reports_out = Some(reports);
            }
        }

        Ok((artifacts_out, hashes_out, reports_out))
    }

    async fn run_parking_agent(
        &self,
        storage_names: &[String],
        mounts: Vec<bollard::service::Mount>,
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
    async fn parse_log_json<T: serde::de::DeserializeOwned>(
        &self,
        container_name: &str,
        prefix: &str,
    ) -> Result<Option<T>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container_machine::ContainerMetadata;
    use bollard::Docker;
    use stormchaser_model::dsl::Step;

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
