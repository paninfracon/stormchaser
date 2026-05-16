use crate::container_machine::{state, DockerContainerMachine, StartResult};
use anyhow::{Context, Result};
use bollard::container::{
    Config, CreateContainerOptions, StartContainerOptions, WaitContainerOptions,
};
use bollard::image::CreateImageOptions;
use bollard::service::{HostConfig, Mount, MountTypeEnum};
use bollard::volume::CreateVolumeOptions;
use chrono::Utc;
use cloudevents::EventBuilder;
use futures::StreamExt;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use stormchaser_model::dsl::{CommonContainerSpec, StorageMount};
use stormchaser_model::events::StepRunningEvent;
use stormchaser_model::events::{EventSource, EventType, SchemaVersion, StepEventType};
use stormchaser_model::nats::publish_cloudevent;
use stormchaser_model::nats::NatsSubject;
use stormchaser_model::step::StepStatus;
use stormchaser_model::{RunId, StepInstanceId, APPLICATION_JSON};
use tokio::{fs, time::sleep};
use tracing::{error, info};
use uuid::Uuid;

struct UnparkParams<'a> {
    source_name: &'a str,
    volume_mount_path: &'a str,
    destination_path: &'a str,
    get_url: &'a str,
    no_extract: bool,
    mode: Option<&'a str>,
    is_bind_mount: bool,
}

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

        let sfs_host_path = std::env::var("STORMCHASER_SFS_HOST_PATH").ok();
        let (mounts, storage_names, volumes_to_cleanup) = self
            .setup_storage_mounts(&spec, sfs_host_path.as_deref())
            .await?;

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
            let running_event = StepRunningEvent {
                run_id: RunId::new(self.metadata.run_id),
                step_id: StepInstanceId::new(self.metadata.step_id),
                event_type: EventType::Step(StepEventType::Running),
                runner_id: None,
                timestamp: Utc::now(),
            };
            let _ = publish_cloudevent(
                &async_nats::jetstream::new(nats.clone()),
                NatsSubject::StepRunning(Some(stormchaser_model::nats::compute_shard_id(
                    &RunId::new(self.metadata.run_id),
                ))),
                EventType::Step(StepEventType::Running),
                EventSource::System,
                serde_json::to_value(running_event).unwrap(),
                Some(SchemaVersion::new("1.0".to_string())),
                None,
            )
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

    async fn unpark_storage(&self, params: UnparkParams<'_>) -> Result<()> {
        let volume_name = params.source_name;
        let agent_image = "stormchaser-agent:v1";
        let unpark_container_name = format!("unpark-{}", Uuid::new_v4());

        let mut cmd = vec![
            "/usr/local/bin/stormchaser-agent".to_string(),
            "unpark".to_string(),
            "--url".to_string(),
            params.get_url.to_string(),
            "--destination".to_string(),
            params.destination_path.to_string(),
        ];
        if params.no_extract {
            cmd.push("--no-extract".to_string());
        }
        if let Some(m) = params.mode {
            cmd.push("--mode".to_string());
            cmd.push(m.to_string());
        }

        let config = Config {
            image: Some(agent_image.to_string()),
            cmd: Some(cmd),
            host_config: Some(HostConfig {
                mounts: Some(vec![Mount {
                    target: Some(params.volume_mount_path.to_string()),
                    source: Some(params.source_name.to_string()),
                    typ: Some(if params.is_bind_mount {
                        MountTypeEnum::BIND
                    } else {
                        MountTypeEnum::VOLUME
                    }),
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
                    if let Err(e) = self
                        .docker
                        .remove_container(&unpark_container_name, None)
                        .await
                    {
                        error!(
                            "Failed to remove unpark container {}: {:?}",
                            unpark_container_name, e
                        );
                    }
                    Ok(())
                }
                res => {
                    error!("Unpark failed for {}: {:?}", volume_name, res);
                    if let Err(e) = self
                        .docker
                        .remove_container(&unpark_container_name, None)
                        .await
                    {
                        error!(
                            "Failed to remove unpark container {}: {:?}",
                            unpark_container_name, e
                        );
                    }
                    Err(anyhow::anyhow!(
                        "Unpark failed for {}: {:?}",
                        volume_name,
                        res
                    ))
                }
            }
        } else {
            if let Err(e) = self
                .docker
                .remove_container(&unpark_container_name, None)
                .await
            {
                error!(
                    "Failed to remove unpark container {}: {:?}",
                    unpark_container_name, e
                );
            }
            Err(anyhow::anyhow!("Wait stream ended unexpectedly"))
        }
    }

    async fn setup_single_mount(
        &self,
        mount: &StorageMount,
        sfs_host_path: Option<&str>,
        mounts: &mut Vec<Mount>,
        storage_names: &mut Vec<String>,
        volumes_to_cleanup: &mut Vec<String>,
    ) -> Result<()> {
        storage_names.push(mount.name.clone());

        let (source_name, is_bind_mount) = if let Some(host_path) = &sfs_host_path {
            let source_path = self.build_bind_mount_source(host_path, &mount.name).await?;
            (source_path, true)
        } else {
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
            (volume_name, false)
        };

        mounts.push(Mount {
            target: Some(mount.mount_path.clone()),
            source: Some(source_name.clone()),
            typ: Some(if is_bind_mount {
                MountTypeEnum::BIND
            } else {
                MountTypeEnum::VOLUME
            }),
            read_only: mount.read_only,
            ..Default::default()
        });

        if let Some(storage_data) = &self.metadata.storage {
            if let Some(urls) = storage_data.get(&mount.name) {
                let has_state = urls.get("expected_hash").and_then(|h| h.as_str()).is_some();

                if has_state {
                    if is_bind_mount {
                        info!(
                            "Skipping unpark for storage '{}' due to STORMCHASER_SFS_HOST_PATH",
                            mount.name
                        );
                    } else if let Some(get_url) = urls.get("get_url").and_then(|u| u.as_str()) {
                        info!(
                            "Unparking storage '{}' (resume state) for volume '{}'",
                            mount.name, source_name
                        );
                        if let Some(nats) = &self.nats {
                            let unpacking_event = serde_json::json!({
                                "run_id": self.metadata.run_id,
                                "step_id": self.metadata.step_id,
                                "status": StepStatus::UnpackingSfs,
                                "timestamp": Utc::now(),
                            });
                            if let Ok(ce) = cloudevents::EventBuilderV10::new()
                                .id(Uuid::new_v4().to_string())
                                .ty("stormchaser.v1.step.unpacking_sfs")
                                .source("/stormchaser/runner")
                                .time(Utc::now())
                                .data(APPLICATION_JSON, unpacking_event)
                                .build()
                            {
                                if let Ok(payload_bytes) = serde_json::to_vec(&ce) {
                                    let _ = nats
                                        .publish(
                                            "stormchaser.v1.step.unpacking_sfs",
                                            payload_bytes.into(),
                                        )
                                        .await;
                                }
                            }
                        }
                        self.unpark_storage(UnparkParams {
                            source_name: &source_name,
                            volume_mount_path: &mount.mount_path,
                            destination_path: &mount.mount_path,
                            get_url,
                            no_extract: false,
                            mode: None,
                            is_bind_mount,
                        })
                        .await?;
                    }
                } else if let Some(provision) = urls.get("provision").and_then(|p| p.as_array()) {
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
                                    "status": StepStatus::UnpackingSfs,
                                    "timestamp": Utc::now(),
                                });
                                if let Ok(ce) = cloudevents::EventBuilderV10::new()
                                    .id(Uuid::new_v4().to_string())
                                    .ty("stormchaser.v1.step.unpacking_sfs")
                                    .source("/stormchaser/runner")
                                    .time(Utc::now())
                                    .data(APPLICATION_JSON, unpacking_event)
                                    .build()
                                {
                                    if let Ok(payload_bytes) = serde_json::to_vec(&ce) {
                                        let _ = nats
                                            .publish(
                                                "stormchaser.v1.step.unpacking_sfs",
                                                payload_bytes.into(),
                                            )
                                            .await;
                                    }
                                }
                            }
                            let mut full_dest = PathBuf::from(&mount.mount_path);
                            if dest != "/" && !dest.is_empty() {
                                let relative_dest = dest.trim_start_matches('/');
                                // Reject path traversal and Windows-style drive prefixes.
                                for component in Path::new(relative_dest).components() {
                                    match component {
                                        std::path::Component::ParentDir => {
                                            anyhow::bail!(
                                                        "Provision destination '{}' contains illegal path traversal (..)",
                                                        dest
                                                    );
                                        }
                                        std::path::Component::Prefix(_) => {
                                            anyhow::bail!(
                                                        "Provision destination '{}' contains an illegal absolute path prefix",
                                                        dest
                                                    );
                                        }
                                        _ => {}
                                    }
                                }
                                full_dest.push(relative_dest);
                            }

                            let resource_type = prov.get("resource_type").and_then(|r| r.as_str());
                            let is_extract = resource_type != Some("artifact");
                            let prov_mode =
                                prov.get("mode").and_then(|m| m.as_str()).map(str::to_owned);
                            info!(
                                "Provisioning resource_type: {:?}, is_extract: {}",
                                resource_type, is_extract
                            );

                            let full_dest_str = full_dest.to_string_lossy().into_owned();
                            self.unpark_storage(UnparkParams {
                                source_name: &source_name,
                                volume_mount_path: &mount.mount_path,
                                destination_path: &full_dest_str,
                                get_url: url,
                                no_extract: !is_extract,
                                mode: prov_mode.as_deref(),
                                is_bind_mount,
                            })
                            .await?;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn validate_bind_mount_name(mount_name: &str) -> Result<()> {
        if mount_name.is_empty() || mount_name.contains('/') || mount_name.contains('\\') {
            anyhow::bail!(
                "Storage mount name '{}' is not valid for bind mounts",
                mount_name
            );
        }

        let mut components = Path::new(mount_name).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(_)), None) => Ok(()),
            _ => anyhow::bail!(
                "Storage mount name '{}' contains illegal path components",
                mount_name
            ),
        }
    }

    async fn build_bind_mount_source(&self, host_path: &str, mount_name: &str) -> Result<String> {
        Self::validate_bind_mount_name(mount_name)?;

        let run_dir = PathBuf::from(host_path).join(self.metadata.run_id.to_string());
        let source_path = run_dir.join(mount_name);
        if !source_path.starts_with(&run_dir) {
            anyhow::bail!(
                "Storage mount name '{}' escapes bind mount root '{}'",
                mount_name,
                run_dir.display()
            );
        }

        fs::create_dir_all(&source_path).await.with_context(|| {
            format!(
                "Failed to create bind mount directory '{}' for storage '{}'",
                source_path.display(),
                mount_name
            )
        })?;

        Ok(source_path.to_string_lossy().into_owned())
    }

    async fn setup_storage_mounts(
        &self,
        spec: &CommonContainerSpec,
        sfs_host_path: Option<&str>,
    ) -> Result<(Vec<Mount>, Vec<String>, Vec<String>)> {
        let mut mounts = Vec::new();
        let mut storage_names = Vec::new();
        let mut volumes_to_cleanup = Vec::new();

        if let Some(storage_mounts) = &spec.storage_mounts {
            for mount in storage_mounts {
                self.setup_single_mount(
                    mount,
                    sfs_host_path,
                    &mut mounts,
                    &mut storage_names,
                    &mut volumes_to_cleanup,
                )
                .await?;
            }
        }
        Ok((mounts, storage_names, volumes_to_cleanup))
    }
    async fn pull_image(&self, image: &str) -> Result<()> {
        // Parse image reference robustly:
        // - Digest-pinned: `repo@sha256:...` → from_image=full ref, tag=""
        // - Tagged:        `repo:tag`         → from_image=repo, tag=tag
        // - Bare:          `repo`             → from_image=repo, tag="latest"
        let (from_image, tag) = if image.contains('@') {
            // Digest reference — pass the full string and let Docker handle it
            (image, "")
        } else {
            match image.rsplit_once(':') {
                // Only treat it as a tag if the part after `:` contains no `/`
                // (to avoid splitting registry hosts like `registry.example.com:5000/repo`)
                Some((repo, t)) if !t.contains('/') => (repo, t),
                _ => (image, "latest"),
            }
        };

        let credentials =
            self.metadata
                .registry_auth
                .as_ref()
                .map(|auth| bollard::auth::DockerCredentials {
                    username: auth
                        .get("username")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    password: auth
                        .get("password")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    serveraddress: auth
                        .get("url")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    ..Default::default()
                });

        let mut pull_stream = self.docker.create_image(
            Some(CreateImageOptions {
                from_image: from_image.to_string(),
                tag: tag.to_string(),
                ..Default::default()
            }),
            None,
            credentials,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container_machine::ContainerMetadata;
    use bollard::Docker;
    use std::collections::HashMap;
    use stormchaser_model::dsl::Step;
    use tempfile::tempdir;

    fn create_test_metadata_with_mounts() -> ContainerMetadata {
        create_test_metadata_with_mount_name("workspace")
    }

    fn create_test_metadata_with_mount_name(mount_name: &str) -> ContainerMetadata {
        let spec = serde_json::json!({
            "image": "alpine:latest",
            "command": ["echo", "hello"],
            "storage_mounts": [
                {
                    "name": mount_name,
                    "mount_path": "/workspace"
                }
            ]
        });

        ContainerMetadata {
            run_id: Uuid::new_v4(),
            step_id: Uuid::new_v4(),
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
            storage: None,
            test_report_urls: None,
            registry_auth: None,
        }
    }

    #[tokio::test]
    async fn test_setup_storage_mounts_named_volume() {
        let docker = Docker::connect_with_local_defaults().unwrap();
        let metadata = create_test_metadata_with_mounts();
        let run_id = metadata.run_id;
        let machine = DockerContainerMachine::new(docker.clone(), metadata, None);

        let spec: CommonContainerSpec =
            serde_json::from_value(machine.metadata.step_dsl.spec.clone()).unwrap();

        let (mounts, storage_names, volumes_to_cleanup) =
            machine.setup_storage_mounts(&spec, None).await.unwrap();

        assert_eq!(mounts.len(), 1);
        assert_eq!(storage_names.len(), 1);
        assert_eq!(volumes_to_cleanup.len(), 1);

        assert_eq!(storage_names[0], "workspace");

        let mount = &mounts[0];
        assert_eq!(mount.target.as_deref(), Some("/workspace"));
        assert_eq!(mount.typ, Some(MountTypeEnum::VOLUME));

        let source = mount.source.as_ref().unwrap();
        assert!(source.starts_with("sfs-workspace-"));
        assert!(source.contains(&run_id.to_string()[..8]));

        // Cleanup
        for vol in volumes_to_cleanup {
            let _ = docker.remove_volume(&vol, None).await;
        }
    }

    #[tokio::test]
    async fn test_setup_storage_mounts_host_bind() {
        let docker = Docker::connect_with_local_defaults().unwrap();
        let metadata = create_test_metadata_with_mounts();
        let run_id = metadata.run_id;
        let machine = DockerContainerMachine::new(docker.clone(), metadata, None);

        let spec: CommonContainerSpec =
            serde_json::from_value(machine.metadata.step_dsl.spec.clone()).unwrap();

        let host_path = tempdir().unwrap();
        let sfs_host_path = Some(host_path.path().to_str().unwrap());
        let (mounts, storage_names, volumes_to_cleanup) = machine
            .setup_storage_mounts(&spec, sfs_host_path)
            .await
            .unwrap();

        assert_eq!(mounts.len(), 1);
        assert_eq!(storage_names.len(), 1);
        assert_eq!(volumes_to_cleanup.len(), 0); // No volumes to clean up for bind mounts

        assert_eq!(storage_names[0], "workspace");

        let mount = &mounts[0];
        assert_eq!(mount.target.as_deref(), Some("/workspace"));
        assert_eq!(mount.typ, Some(MountTypeEnum::BIND));

        let expected_source = host_path.path().join(run_id.to_string()).join("workspace");
        assert!(expected_source.is_dir());
        assert_eq!(mount.source.as_deref(), expected_source.to_str());
    }

    #[tokio::test]
    async fn test_setup_storage_mounts_host_bind_rejects_invalid_mount_name() {
        let docker = Docker::connect_with_local_defaults().unwrap();
        let metadata = create_test_metadata_with_mount_name("../escape");
        let machine = DockerContainerMachine::new(docker, metadata, None);
        let spec: CommonContainerSpec =
            serde_json::from_value(machine.metadata.step_dsl.spec.clone()).unwrap();
        let host_path = tempdir().unwrap();

        let err = machine
            .setup_storage_mounts(&spec, host_path.path().to_str())
            .await
            .unwrap_err();

        assert!(
            err.to_string().contains("Storage mount name '../escape'"),
            "unexpected error: {err}"
        );
    }
}
