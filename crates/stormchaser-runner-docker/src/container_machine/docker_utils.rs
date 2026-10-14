use super::{crypto, DockerContainerMachine};
use anyhow::Result;
use bollard::models::ContainerCreateBody;
use bollard::models::HostConfigLogConfig;
use bollard::models::{HostConfig, Mount};
use std::collections::HashMap;
use stormchaser_model::dsl::{self, CommonContainerSpec};
use tracing::info;

/// Environment variable naming the Docker network step containers are attached to.
pub const STEP_NETWORK_ENV: &str = "STORMCHASER_STEP_NETWORK";

impl<S> DockerContainerMachine<S> {
    /// Returns the Docker network step containers should join.
    ///
    /// `STORMCHASER_STEP_NETWORK` names a network set aside for steps, so that they cannot
    /// reach the control plane (NATS, Postgres) that shares the runner's own network.
    /// Without it, steps fall back to the runner's network.
    pub async fn get_network_mode(&self) -> Option<String> {
        if let Some(network) = std::env::var(STEP_NETWORK_ENV)
            .ok()
            .filter(|n| !n.trim().is_empty())
        {
            return Some(network);
        }
        tracing::warn!(
            "{} is not set; step containers will share the runner's network and can reach whatever it can",
            STEP_NETWORK_ENV
        );
        if let Ok(hostname) = std::env::var("HOSTNAME") {
            if let Ok(inspect) = self.docker.inspect_container(&hostname, None).await {
                if let Some(networks) = inspect.network_settings.and_then(|ns| ns.networks) {
                    if let Some(network_name) = networks.keys().next() {
                        info!("Detected runner network: {}", network_name);
                        return Some(network_name.clone());
                    }
                }
            }
        }
        info!("Could not detect runner network, using default");
        None
    }

    /// Builds the Docker container configuration based on the provided specifications.
    pub fn build_container_config(
        &self,
        spec: &CommonContainerSpec,
        mounts: Vec<Mount>,
        network_mode: Option<String>,
        storage_names: &[String],
        container_name: &str,
        loki_url: Option<&str>,
    ) -> Result<ContainerCreateBody> {
        dsl::ensure_privileged_permitted(spec.privileged, dsl::privileged_steps_allowed())?;

        let mut env: Vec<String> = spec
            .env
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|e| format!("{}={}", e.name, e.value))
            .collect();

        if !storage_names.is_empty() {
            env.push(format!("STORMCHASER_STORAGES={}", storage_names.join(" ")));
        }

        let final_image = spec.image.clone();

        let mut original_full_cmd = Vec::new();
        if let Some(cmd) = &spec.command {
            original_full_cmd.extend(cmd.clone());
        }
        if let Some(args) = &spec.args {
            original_full_cmd.extend(args.clone());
        }

        let (final_command, final_args) = if !original_full_cmd.is_empty() {
            let step_name = &self.metadata.step_dsl.name;
            let wrapped_script = format!(
                "echo '========================================'; \
                 echo 'Step Metadata: {}'; \
                 echo \"Command: $@\"; \
                 echo '========================================'; \
                 \"$@\"; \
                 RET=$?; \
                 echo '========================================'; \
                 echo 'Completion Status: '$RET; \
                 echo '========================================'; \
                 exit $RET",
                step_name
            );
            let mut new_args = vec!["-c".to_string(), wrapped_script, "--".to_string()];
            new_args.extend(original_full_cmd);
            (Some(vec!["/bin/sh".to_string()]), Some(new_args))
        } else {
            (spec.command.clone(), spec.args.clone())
        };

        // The step's definition is kept on the container so that a restarted runner can
        // pick the step up again. Encrypted, it is kept whole; in clear, it is kept
        // without the values that are secret, which picking a step up does not need.
        let step_dsl_val = match &self.metadata.encryption_key {
            Some(key) => crypto::encrypt_state(
                &serde_json::to_string(&self.metadata.step_dsl).unwrap_or_default(),
                key,
            )?,
            None => serde_json::to_string(&self.metadata.step_dsl.without_secret_values())
                .unwrap_or_default(),
        };

        let mut labels = HashMap::new();
        labels.insert("managed-by".to_string(), "stormchaser".to_string());
        labels.insert(
            "stormchaser-run-id".to_string(),
            self.metadata.run_id.to_string(),
        );
        labels.insert(
            "stormchaser-step-id".to_string(),
            self.metadata.step_id.to_string(),
        );
        labels.insert(
            "stormchaser-fencing-token".to_string(),
            self.metadata.fencing_token.to_string(),
        );
        labels.insert(
            "stormchaser.v1.io/received-at".to_string(),
            self.metadata.received_at.to_rfc3339(),
        );
        labels.insert("stormchaser.v1.io/step-dsl".to_string(), step_dsl_val);

        if self.metadata.encryption_key.is_some() {
            labels.insert(
                "stormchaser.v1.io/state-encrypted".to_string(),
                "true".to_string(),
            );
        }

        let mut log_config = None;
        if let Some(url) = loki_url {
            let external_labels = format!(
                "job_name={},run_id={},step_id={}",
                container_name, self.metadata.run_id, self.metadata.step_id
            );
            log_config = Some(HostConfigLogConfig {
                typ: Some("loki".to_string()),
                config: Some(HashMap::from([
                    ("loki-url".to_string(), url.to_string()),
                    ("loki-external-labels".to_string(), external_labels),
                    ("loki-retries".to_string(), "2".to_string()),
                    ("loki-batch-size".to_string(), "1".to_string()),
                    ("loki-batch-wait".to_string(), "10ms".to_string()),
                ])),
            });
        }

        tracing::info!(
            "Loki config is: {:?}, loki_url was: {:?}",
            log_config,
            loki_url
        );
        // What the runner allows any one step, whatever its workflow asks for. A step
        // that asks for nothing gets the defaults, not the whole host.
        let limits = stormchaser_model::dsl::limits::StepLimits::from_env();
        let resources = limits.resources(spec.cpu.as_deref(), spec.memory.as_deref())?;
        // Refused here, before anything is started, if the step's timeout is out of range
        limits.timeout(self.metadata.step_dsl.timeout.as_deref())?;

        Ok(ContainerCreateBody {
            image: Some(final_image),
            cmd: final_args,
            entrypoint: final_command,
            env: Some(env),
            labels: Some(labels),
            host_config: Some(HostConfig {
                // Quota against Docker's default period of 100 ms
                cpu_quota: Some(resources.cpu_millis as i64 * 100),
                memory: Some(resources.memory_bytes as i64),
                // No swap on top of the memory limit
                memory_swap: Some(resources.memory_bytes as i64),
                // Bounds a fork bomb to the container
                pids_limit: Some(limits.pids_limit),
                privileged: spec.privileged,
                mounts: Some(mounts),
                network_mode,
                log_config,
                ..Default::default()
            }),
            ..Default::default()
        })
    }

    /// Parses a CPU string limit to Docker CPU quota representation.
    pub fn parse_cpu_to_quota(&self, cpu: &str) -> Option<i64> {
        if let Some(m_idx) = cpu.find('m') {
            if let Ok(m_cores) = cpu[..m_idx].parse::<i64>() {
                return Some(m_cores * 100);
            }
        } else if let Ok(cores) = cpu.parse::<f64>() {
            return Some((cores * 100_000.0) as i64);
        }
        None
    }

    /// Parses a memory string limit into bytes.
    pub fn parse_memory_to_bytes(&self, memory: &str) -> Option<i64> {
        let mem = memory.to_lowercase();
        if let Some(idx) = mem.find(|c: char| c.is_alphabetic()) {
            let val = mem[..idx].trim().parse::<i64>().ok()?;
            let suffix = &mem[idx..];
            match suffix {
                "gi" | "g" => Some(val * 1024 * 1024 * 1024),
                "mi" | "m" => Some(val * 1024 * 1024),
                "ki" | "k" => Some(val * 1024),
                _ => Some(val),
            }
        } else {
            memory.parse::<i64>().ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{state, ContainerMetadata, DockerContainerMachine};
    use super::*;
    use stormchaser_model::dsl::Step;
    use uuid::Uuid;

    fn get_dummy_machine() -> DockerContainerMachine<state::Initialized> {
        let docker = bollard::Docker::connect_with_local_defaults().unwrap();
        let step_dsl: Step = serde_json::from_str(
            r#"{
            "name": "test",
            "type": "RunContainer",
            "params": {},
            "spec": {},
            "aggregation": [],
            "next": [],
            "outputs": [],
            "reports": []
        }"#,
        )
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
        DockerContainerMachine::new(docker, metadata, None)
    }

    #[test]
    fn test_parse_cpu_to_quota() {
        let machine = get_dummy_machine();
        assert_eq!(machine.parse_cpu_to_quota("500m"), Some(50000));
        assert_eq!(machine.parse_cpu_to_quota("1.0"), Some(100000));
    }

    #[test]
    fn test_parse_memory_to_bytes() {
        let machine = get_dummy_machine();
        assert_eq!(
            machine.parse_memory_to_bytes("512Mi"),
            Some(512 * 1024 * 1024)
        );
        assert_eq!(
            machine.parse_memory_to_bytes("1Gi"),
            Some(1024 * 1024 * 1024)
        );
    }

    #[test]
    fn test_build_container_config() {
        let machine = get_dummy_machine();
        let spec = CommonContainerSpec {
            image: "alpine:latest".into(),
            registry_connection: None,
            connections: None,
            command: Some(vec!["echo".into()]),
            args: Some(vec!["hello".into()]),
            env: None,
            cpu: Some("500m".into()),
            memory: Some("512Mi".into()),
            privileged: Some(false),
            storage_mounts: None,
        };

        let config = machine
            .build_container_config(&spec, vec![], None, &[], "test-container", None)
            .unwrap();
        assert_eq!(config.image, Some("alpine:latest".into()));
    }

    fn container_spec(cpu: Option<&str>, memory: Option<&str>) -> CommonContainerSpec {
        CommonContainerSpec {
            image: "alpine:latest".into(),
            registry_connection: None,
            connections: None,
            command: Some(vec!["true".into()]),
            args: None,
            env: None,
            cpu: cpu.map(str::to_string),
            memory: memory.map(str::to_string),
            privileged: None,
            storage_mounts: None,
        }
    }

    #[test]
    fn a_step_that_asks_for_nothing_is_still_bounded() {
        let machine = get_dummy_machine();
        let config = machine
            .build_container_config(&container_spec(None, None), vec![], None, &[], "c", None)
            .unwrap();
        let host = config.host_config.unwrap();
        // One CPU, 2 GiB with no swap on top, and a ceiling on processes
        assert_eq!(host.cpu_quota, Some(100_000));
        assert_eq!(host.memory, Some(2 * 1024 * 1024 * 1024));
        assert_eq!(host.memory_swap, host.memory);
        assert_eq!(host.pids_limit, Some(4096));
    }

    #[test]
    fn a_steps_own_request_is_kept_within_the_limits_and_refused_beyond_them() {
        let machine = get_dummy_machine();
        let config = machine
            .build_container_config(
                &container_spec(Some("500m"), Some("512Mi")),
                vec![],
                None,
                &[],
                "c",
                None,
            )
            .unwrap();
        let host = config.host_config.unwrap();
        assert_eq!(host.cpu_quota, Some(50_000));
        assert_eq!(host.memory, Some(512 * 1024 * 1024));

        for (cpu, memory) in [
            (Some("64"), None),
            (None, Some("64Gi")),
            (Some("lots"), None),
        ] {
            assert!(machine
                .build_container_config(&container_spec(cpu, memory), vec![], None, &[], "c", None)
                .is_err());
        }

        let mut slow = get_dummy_machine();
        slow.metadata.step_dsl.timeout = Some("30d".into());
        let err = slow
            .build_container_config(&container_spec(None, None), vec![], None, &[], "c", None)
            .unwrap_err();
        assert!(err.to_string().contains("timeout"), "{err}");
    }

    #[test]
    fn test_build_container_config_rejects_privileged_by_default() {
        let machine = get_dummy_machine();
        let spec = CommonContainerSpec {
            image: "alpine:latest".into(),
            registry_connection: None,
            connections: None,
            command: None,
            args: None,
            env: None,
            cpu: None,
            memory: None,
            privileged: Some(true),
            storage_mounts: None,
        };

        let err = machine
            .build_container_config(&spec, vec![], None, &[], "test-container", None)
            .unwrap_err();
        assert!(err.to_string().contains(dsl::ALLOW_PRIVILEGED_ENV));
    }

    #[tokio::test]
    async fn test_step_network_override() {
        let machine = get_dummy_machine();
        std::env::set_var(STEP_NETWORK_ENV, "stormchaser_steps");
        assert_eq!(
            machine.get_network_mode().await,
            Some("stormchaser_steps".to_string())
        );
        std::env::remove_var(STEP_NETWORK_ENV);
    }
}
