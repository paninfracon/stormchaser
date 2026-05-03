use uuid::Uuid;

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
}

impl Config {
    /// From env.
    pub fn from_env<I, K, V>(env: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut nats_url = "nats://localhost:4222".to_string();
        let mut runner_id = Uuid::new_v4().to_string();
        let mut encryption_key = None;
        let mut rust_log = "stormchaser_runner_k8s=info".to_string();

        for (k, v) in env {
            match k.as_ref() {
                "NATS_URL" => nats_url = v.as_ref().to_string(),
                "RUNNER_ID" => runner_id = v.as_ref().to_string(),
                "STORMCHASER_STATE_ENCRYPTION_KEY" => encryption_key = Some(v.as_ref().to_string()),
                "RUST_LOG" => rust_log = v.as_ref().to_string(),
                _ => {}
            }
        }

        Self {
            nats_url,
            runner_id,
            encryption_key,
            rust_log,
        }
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
}
