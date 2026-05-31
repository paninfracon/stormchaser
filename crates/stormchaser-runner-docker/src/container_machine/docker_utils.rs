use super::{crypto, DockerContainerMachine};
use anyhow::Result;
use bollard::container::Config;
use bollard::models::HostConfigLogConfig;
use bollard::service::{HostConfig, Mount};
use std::collections::HashMap;
use stormchaser_model::dsl::CommonContainerSpec;
use tracing::info;

impl<S> DockerContainerMachine<S> {
    /// Attempts to detect the Docker network mode the runner is operating in.
    pub async fn get_network_mode(&self) -> Option<String> {
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
    ) -> Result<Config<String>> {
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

        let step_dsl_json = serde_json::to_string(&self.metadata.step_dsl).unwrap_or_default();
        let step_dsl_val = if let Some(key) = &self.metadata.encryption_key {
            crypto::encrypt_state(&step_dsl_json, key)?
        } else {
            step_dsl_json
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
        Ok(Config {
            image: Some(final_image),
            cmd: final_args,
            entrypoint: final_command,
            env: Some(env),
            labels: Some(labels),
            host_config: Some(HostConfig {
                cpu_quota: spec.cpu.as_ref().and_then(|c| self.parse_cpu_to_quota(c)),
                memory: spec
                    .memory
                    .as_ref()
                    .and_then(|m| self.parse_memory_to_bytes(m)),
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
}
