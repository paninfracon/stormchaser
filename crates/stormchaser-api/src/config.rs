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
    /// The nats url.
    pub nats_url: String,
    /// The opa url.
    pub opa_url: Option<String>,
    /// The opa wasm path.
    pub opa_wasm_path: Option<String>,
    /// The opa entrypoint.
    pub opa_entrypoint: Option<String>,
    /// The loki url.
    pub loki_url: Option<String>,
    /// The elasticsearch url.
    pub elasticsearch_url: Option<String>,
    /// The elasticsearch index.
    pub elasticsearch_index: Option<String>,
    /// The oidc issuer.
    pub oidc_issuer: Option<String>,
    /// The oidc external issuer.
    pub oidc_external_issuer: Option<String>,
    /// The oidc client id.
    pub oidc_client_id: Option<String>,
    /// The oidc client secret.
    pub oidc_client_secret: Option<String>,
    /// The API base URL for MCP callback.
    pub api_base_url: String,
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
        let mut nats_url = "nats://localhost:4222".to_string();
        let mut opa_url = None;
        let mut opa_wasm_path = None;
        let mut opa_entrypoint = None;
        let mut loki_url = None;
        let mut elasticsearch_url = None;
        let mut elasticsearch_index = None;
        let mut oidc_issuer = None;
        let mut oidc_external_issuer = None;
        let mut oidc_client_id = None;
        let mut oidc_client_secret = None;
        let mut api_base_url = "http://localhost:3000".to_string();

        for (k, v) in env {
            match k.as_ref() {
                "DATABASE_URL" => database_url = Some(v.as_ref().to_string()),
                "TLS_CA_CERT_PATH" => tls_ca_cert_path = Some(PathBuf::from(v.as_ref())),
                "TLS_CERT_PATH" => tls_cert_path = PathBuf::from(v.as_ref()),
                "TLS_KEY_PATH" => tls_key_path = PathBuf::from(v.as_ref()),
                "TLS_SERVER_NAME" => tls_server_name = Some(v.as_ref().to_string()),
                "STORMCHASER_DB_SSL" => db_ssl = v.as_ref() == "true",
                "NATS_URL" => nats_url = v.as_ref().to_string(),
                "OPA_URL" => opa_url = Some(v.as_ref().to_string()),
                "OPA_WASM_PATH" => opa_wasm_path = Some(v.as_ref().to_string()),
                "OPA_ENTRYPOINT" => opa_entrypoint = Some(v.as_ref().to_string()),
                "LOKI_URL" => loki_url = Some(v.as_ref().to_string()),
                "ELASTICSEARCH_URL" => elasticsearch_url = Some(v.as_ref().to_string()),
                "ELASTICSEARCH_INDEX" => elasticsearch_index = Some(v.as_ref().to_string()),
                "OIDC_ISSUER" => oidc_issuer = Some(v.as_ref().to_string()),
                "OIDC_EXTERNAL_ISSUER" => oidc_external_issuer = Some(v.as_ref().to_string()),
                "OIDC_CLIENT_ID" => oidc_client_id = Some(v.as_ref().to_string()),
                "OIDC_CLIENT_SECRET" => oidc_client_secret = Some(v.as_ref().to_string()),
                "API_BASE_URL" => api_base_url = v.as_ref().to_string(),
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
            nats_url,
            opa_url,
            opa_wasm_path,
            opa_entrypoint,
            loki_url,
            elasticsearch_url,
            elasticsearch_index,
            oidc_issuer,
            oidc_external_issuer,
            oidc_client_id,
            oidc_client_secret,
            api_base_url,
        })
    }
}
