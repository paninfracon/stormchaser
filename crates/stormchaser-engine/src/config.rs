use anyhow::Context;
use std::path::PathBuf;

#[derive(Debug, Clone)]
/// Config.
pub struct Config {
    /// The database url.
    pub database_url: String,
    /// The tls ca cert path.
    pub tls_ca_cert_path: Option<PathBuf>,
    /// The tls cert path.
    pub tls_cert_path: PathBuf,
    /// The tls key path.
    pub tls_key_path: PathBuf,
    /// The tls server name.
    pub tls_server_name: Option<String>,
    /// The db ssl.
    pub db_ssl: bool,
    /// The git cache dir.
    pub git_cache_dir: PathBuf,
    /// The opa url.
    pub opa_url: Option<String>,
    /// The opa wasm path.
    pub opa_wasm_path: Option<PathBuf>,
    /// The opa entrypoint.
    pub opa_entrypoint: Option<String>,
    /// The loki url.
    pub loki_url: Option<String>,
    /// The elasticsearch url.
    pub elasticsearch_url: Option<String>,
    /// The elasticsearch index.
    pub elasticsearch_index: Option<String>,
    /// The vault addr.
    pub vault_addr: String,
    /// The vault token.
    pub vault_token: String,
    /// The nats url.
    pub nats_url: String,
    /// The rust log.
    pub rust_log: String,
}

impl Config {
    /// From env.
    pub fn from_env<I, K, V>(env: I) -> anyhow::Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut database_url = None;
        let mut tls_ca_cert_path = None;
        let mut tls_cert_path = PathBuf::from("/etc/engine/certs/tls.crt");
        let mut tls_key_path = PathBuf::from("/etc/engine/certs/tls.key");
        let mut tls_server_name = None;
        let mut db_ssl = false;
        let mut git_cache_dir = PathBuf::from("/tmp/stormchaser/git-cache");
        let mut opa_url = None;
        let mut opa_wasm_path = None;
        let mut opa_entrypoint = None;
        let mut loki_url = None;
        let mut elasticsearch_url = None;
        let mut elasticsearch_index = None;
        let mut vault_addr = "http://localhost:8200".to_string();
        let mut vault_token = "root".to_string();
        let mut nats_url = "nats://localhost:4222".to_string();
        let mut rust_log = "stormchaser_engine=debug".to_string();

        for (k, v) in env {
            match k.as_ref() {
                "DATABASE_URL" => database_url = Some(v.as_ref().to_string()),
                "TLS_CA_CERT_PATH" => tls_ca_cert_path = Some(PathBuf::from(v.as_ref())),
                "TLS_CERT_PATH" => tls_cert_path = PathBuf::from(v.as_ref()),
                "TLS_KEY_PATH" => tls_key_path = PathBuf::from(v.as_ref()),
                "TLS_SERVER_NAME" => tls_server_name = Some(v.as_ref().to_string()),
                "STORMCHASER_DB_SSL" => db_ssl = v.as_ref() == "true",
                "GIT_CACHE_DIR" => git_cache_dir = PathBuf::from(v.as_ref()),
                "OPA_URL" => opa_url = Some(v.as_ref().to_string()),
                "OPA_WASM_PATH" => opa_wasm_path = Some(PathBuf::from(v.as_ref())),
                "OPA_ENTRYPOINT" => opa_entrypoint = Some(v.as_ref().to_string()),
                "LOKI_URL" => loki_url = Some(v.as_ref().to_string()),
                "ELASTICSEARCH_URL" => elasticsearch_url = Some(v.as_ref().to_string()),
                "ELASTICSEARCH_INDEX" => elasticsearch_index = Some(v.as_ref().to_string()),
                "VAULT_ADDR" => vault_addr = v.as_ref().to_string(),
                "VAULT_TOKEN" => vault_token = v.as_ref().to_string(),
                "NATS_URL" => nats_url = v.as_ref().to_string(),
                "RUST_LOG" => rust_log = v.as_ref().to_string(),
                _ => {}
            }
        }

        Ok(Self {
            database_url: database_url.context("DATABASE_URL must be set")?,
            tls_ca_cert_path,
            tls_cert_path,
            tls_key_path,
            tls_server_name,
            db_ssl,
            git_cache_dir,
            opa_url,
            opa_wasm_path,
            opa_entrypoint,
            loki_url,
            elasticsearch_url,
            elasticsearch_index,
            vault_addr,
            vault_token,
            nats_url,
            rust_log,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_from_env_missing_database_url() {
        let env: Vec<(&str, &str)> = vec![];
        let config = Config::from_env(env);
        assert!(config.is_err());
        assert_eq!(config.unwrap_err().to_string(), "DATABASE_URL must be set");
    }

    #[test]
    fn test_config_from_env_valid() {
        let env = vec![
            ("DATABASE_URL", "postgres://user:pass@localhost/db"),
            ("TLS_SERVER_NAME", "engine.example.com"),
            ("STORMCHASER_DB_SSL", "true"),
            ("LOKI_URL", "http://loki:3100"),
            ("VAULT_ADDR", "http://vault:8200"),
        ];
        let config = Config::from_env(env).unwrap();
        assert_eq!(config.database_url, "postgres://user:pass@localhost/db");
        assert_eq!(
            config.tls_server_name.as_deref(),
            Some("engine.example.com")
        );
        assert!(config.db_ssl);
        assert_eq!(config.nats_url, "nats://localhost:4222");
        assert_eq!(config.loki_url.as_deref(), Some("http://loki:3100"));
        assert_eq!(config.vault_addr, "http://vault:8200");
        assert_eq!(config.vault_token, "root");
        assert_eq!(
            config.tls_cert_path,
            PathBuf::from("/etc/engine/certs/tls.crt")
        );
    }
}
