use crate::container_machine::{state, DockerContainerMachine, StartResult};
use anyhow::{Context, Result};
use bollard::container::{
    Config, CreateContainerOptions, StartContainerOptions, WaitContainerOptions,
};
use bollard::image::CreateImageOptions;
use bollard::service::{HostConfig, Mount, MountTypeEnum};
use bollard::volume::CreateVolumeOptions;
use chrono::Utc;
use futures::StreamExt;
use std::path::{Path, PathBuf};
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
                                            "stormchaser.v1.step.unpacking_sfs",
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
                                    None,
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
                                                "stormchaser.v1.step.unpacking_sfs",
                                                unpacking_event.to_string().into(),
                                            )
                                            .await;
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

                                    let resource_type =
                                        prov.get("resource_type").and_then(|r| r.as_str());
                                    let is_extract = resource_type != Some("artifact");
                                    let prov_mode = prov
                                        .get("mode")
                                        .and_then(|m| m.as_str())
                                        .map(str::to_owned);
                                    info!(
                                        "Provisioning resource_type: {:?}, is_extract: {}",
                                        resource_type, is_extract
                                    );

                                    let full_dest_str = full_dest.to_string_lossy().into_owned();
                                    self.unpark_storage(
                                        &volume_name,
                                        &mount.mount_path,
                                        &full_dest_str,
                                        url,
                                        !is_extract,
                                        prov_mode.as_deref(),
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
            let _ = stormchaser_model::nats::publish_cloudevent(
                &async_nats::jetstream::new(nats.clone()),
                "stormchaser.v1.step.running",
                "stormchaser.v1.step.running",
                "/stormchaser",
                serde_json::to_value(running_event).unwrap(),
                Some("1.0"),
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

    async fn unpark_storage(
        &self,
        volume_name: &str,
        volume_mount_path: &str,
        destination_path: &str,
        get_url: &str,
        no_extract: bool,
        mode: Option<&str>,
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
        if let Some(m) = mode {
            cmd.push("--mode".to_string());
            cmd.push(m.to_string());
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
