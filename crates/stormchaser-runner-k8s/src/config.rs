#[derive(Debug, Clone)]
/// Config.
pub struct Config {
    /// The nats url.
    pub nats_url: String,
    /// The runner id.
    pub runner_id: String,
    /// The encryption key.
    pub encryption_key: Option<String>,
    /// The rust log.
    pub rust_log: String,
    /// Namespace the runner itself is deployed in, when known.
    pub own_namespace: Option<String>,
    /// Namespace workflow step Jobs are created in.
    pub step_namespace: Option<String>,
}

/// Environment variable naming the namespace workflow step Jobs run in.
pub const STEP_NAMESPACE_ENV: &str = "STORMCHASER_STEP_NAMESPACE";

/// Where Kubernetes tells a pod which namespace it runs in.
const IN_CLUSTER_NAMESPACE_FILE: &str = "/var/run/secrets/kubernetes.io/serviceaccount/namespace";

/// Cluster-owned namespaces that must never receive workflow steps.
const SYSTEM_NAMESPACES: [&str; 3] = ["kube-system", "kube-public", "kube-node-lease"];

impl Config {
    /// From env.
    pub fn from_env<I, K, V>(env: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut nats_url = "nats://localhost:4222".to_string();
        let mut explicit_runner_id = None;
        let mut hostname = None;
        let mut encryption_key = None;
        let mut rust_log = "stormchaser_runner_k8s=info".to_string();
        let mut own_namespace = None;
        let mut step_namespace = None;

        for (k, v) in env {
            match k.as_ref() {
                "NATS_URL" => nats_url = v.as_ref().to_string(),
                "RUNNER_ID" => explicit_runner_id = Some(v.as_ref().to_string()),
                "HOSTNAME" => hostname = Some(v.as_ref().to_string()),
                "STORMCHASER_STATE_ENCRYPTION_KEY" => encryption_key = Some(v.as_ref().to_string()),
                "RUST_LOG" => rust_log = v.as_ref().to_string(),
                "KUBERNETES_NAMESPACE" => own_namespace = Some(v.as_ref().to_string()),
                "STORMCHASER_STEP_NAMESPACE" => step_namespace = Some(v.as_ref().to_string()),
                _ => {}
            }
        }

        // Stable across restarts of this runner, so it comes back as itself
        let runner_id = stormchaser_model::runner::runner_id(
            "k8s",
            explicit_runner_id.as_deref(),
            hostname
                .or_else(stormchaser_model::runner::kernel_hostname)
                .as_deref(),
        );

        Self {
            nats_url,
            runner_id,
            encryption_key,
            rust_log,
            own_namespace,
            step_namespace,
        }
    }

    /// Returns the namespace step Jobs run in, refusing any configuration that would put
    /// them alongside Stormchaser itself.
    ///
    /// Steps run workflow authors' images. Keeping them out of the namespace that holds
    /// the runner, the control plane and their secrets is what stops a step from
    /// mounting or reading those, so the runner will not start without it.
    pub fn resolve_step_namespace(&self) -> anyhow::Result<String> {
        let step = self
            .step_namespace
            .as_deref()
            .map(str::trim)
            .filter(|ns| !ns.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{} must name the namespace workflow steps run in",
                    STEP_NAMESPACE_ENV
                )
            })?;

        if SYSTEM_NAMESPACES.contains(&step) {
            anyhow::bail!(
                "{} must not be the system namespace '{}'",
                STEP_NAMESPACE_ENV,
                step
            );
        }

        let own = self.own_namespace.clone().or_else(|| {
            std::fs::read_to_string(IN_CLUSTER_NAMESPACE_FILE)
                .ok()
                .map(|ns| ns.trim().to_string())
        });
        if own.as_deref() == Some(step) {
            anyhow::bail!(
                "{} is '{}', the namespace the runner itself runs in; workflow steps must run in a separate namespace",
                STEP_NAMESPACE_ENV,
                step
            );
        }

        Ok(step.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_from_env_defaults() {
        let env: Vec<(String, String)> = vec![];
        let config = Config::from_env(env);
        assert_eq!(config.nats_url, "nats://localhost:4222");
        assert!(config.encryption_key.is_none());
        assert_eq!(config.rust_log, "stormchaser_runner_k8s=info");
        // runner_id is random, just assert it's not empty
        assert!(!config.runner_id.is_empty());
    }

    #[test]
    fn test_config_from_env_custom() {
        let env = vec![
            ("NATS_URL".to_string(), "nats://remote:4222".to_string()),
            ("RUNNER_ID".to_string(), "my-runner".to_string()),
            (
                "STORMCHASER_STATE_ENCRYPTION_KEY".to_string(),
                "my-key".to_string(),
            ),
            ("RUST_LOG".to_string(), "debug".to_string()),
        ];
        let config = Config::from_env(env);
        assert_eq!(config.nats_url, "nats://remote:4222");
        assert_eq!(config.runner_id, "my-runner");
        assert_eq!(config.encryption_key.as_deref(), Some("my-key"));
        assert_eq!(config.rust_log, "debug");
    }

    fn config_with_namespaces(own: Option<&str>, step: Option<&str>) -> Config {
        let mut env = Vec::new();
        if let Some(own) = own {
            env.push(("KUBERNETES_NAMESPACE".to_string(), own.to_string()));
        }
        if let Some(step) = step {
            env.push((STEP_NAMESPACE_ENV.to_string(), step.to_string()));
        }
        Config::from_env(env)
    }

    #[test]
    fn test_resolve_step_namespace_accepts_a_separate_namespace() {
        let config = config_with_namespaces(Some("stormchaser"), Some("stormchaser-steps"));
        assert_eq!(
            config.resolve_step_namespace().unwrap(),
            "stormchaser-steps"
        );
    }

    #[test]
    fn test_resolve_step_namespace_rejects_shared_missing_and_system_namespaces() {
        let shared = config_with_namespaces(Some("stormchaser"), Some("stormchaser"));
        assert!(shared
            .resolve_step_namespace()
            .unwrap_err()
            .to_string()
            .contains("separate namespace"));

        for step in [None, Some(""), Some("  ")] {
            let missing = config_with_namespaces(Some("stormchaser"), step);
            assert!(missing.resolve_step_namespace().is_err());
        }

        let system = config_with_namespaces(Some("stormchaser"), Some("kube-system"));
        assert!(system.resolve_step_namespace().is_err());
    }
}
