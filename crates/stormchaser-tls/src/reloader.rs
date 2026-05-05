use anyhow::{Context, Result};
use arc_swap::ArcSwap;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use rustls_pemfile::{certs, private_key};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{error, info, warn};

/// Configuration for TLS certificate paths.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TlsConfig {
    /// Path to the Certificate Authority (CA) certificate.
    pub ca_cert_path: Option<PathBuf>,
    /// Path to the client or server certificate.
    pub cert_path: PathBuf,
    /// Path to the client or server private key.
    pub key_path: PathBuf,
    /// Optional expected server name for SNI/validation.
    pub server_name: Option<String>,
}

/// Handles hot-reloading of TLS configurations.
pub struct TlsReloader {
    client_config: Arc<ArcSwap<ClientConfig>>,
    server_config: Arc<ArcSwap<ServerConfig>>,
    _watcher: RecommendedWatcher,
}

impl TlsReloader {
    /// Creates a new `TlsReloader` watching the paths specified in `config`.
    pub async fn new(config: TlsConfig) -> Result<Self> {
        // Ensure a default crypto provider is installed (ignoring errors if already installed)
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();

        let client_config = Arc::new(ArcSwap::from_pointee(Self::load_client_config(&config)?));
        let server_config = Arc::new(ArcSwap::from_pointee(Self::load_server_config(&config)?));

        let client_config_clone = client_config.clone();
        let server_config_clone = server_config.clone();
        let config_clone = config.clone();

        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<Event>| match res {
                Ok(event) => {
                    if event.kind.is_modify() || event.kind.is_create() {
                        info!("TLS certificate files changed, reloading...");
                        match Self::load_client_config(&config_clone) {
                            Ok(new_config) => client_config_clone.store(Arc::new(new_config)),
                            Err(e) => error!("Failed to reload TLS client config: {:?}", e),
                        }
                        match Self::load_server_config(&config_clone) {
                            Ok(new_config) => server_config_clone.store(Arc::new(new_config)),
                            Err(e) => error!("Failed to reload TLS server config: {:?}", e),
                        }
                    }
                }
                Err(e) => error!("Watch error: {:?}", e),
            })?;

        // Watch the directory containing the certificates
        if let Some(parent) = config.cert_path.parent() {
            watcher.watch(parent, RecursiveMode::NonRecursive)?;
        }
        if let Some(ca_parent) = config.ca_cert_path.as_ref().and_then(|p| p.parent()) {
            if ca_parent != config.cert_path.parent().unwrap_or(Path::new("")) {
                watcher.watch(ca_parent, RecursiveMode::NonRecursive)?;
            }
        }

        Ok(Self {
            client_config,
            server_config,
            _watcher: watcher,
        })
    }

    /// Returns an atomically updated `Arc<ClientConfig>`.
    pub fn client_config(&self) -> Arc<ClientConfig> {
        self.client_config.load_full()
    }

    /// Returns an atomically updated `Arc<ServerConfig>`.
    pub fn server_config(&self) -> Arc<ServerConfig> {
        self.server_config.load_full()
    }

    fn load_client_config(config: &TlsConfig) -> Result<ClientConfig> {
        let mut root_store = RootCertStore::empty();

        // Add native roots
        let native_certs = rustls_native_certs::load_native_certs();
        for cert in native_certs.certs {
            root_store.add(cert)?;
        }
        if !native_certs.errors.is_empty() {
            warn!(
                "Errors loading native certificates: {:?}",
                native_certs.errors
            );
        }
        if let Some(ca_path) = &config.ca_cert_path {
            let ca_certs = load_certs(ca_path)?;
            for cert in ca_certs {
                root_store.add(cert)?;
            }
        }

        let certs = load_certs(&config.cert_path)?;
        let key = load_key(&config.key_path)?;

        let client_config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_client_auth_cert(certs, key)
            .context("failed to create client config")?;

        Ok(client_config)
    }

    fn load_server_config(config: &TlsConfig) -> Result<ServerConfig> {
        let certs = load_certs(&config.cert_path)?;
        let key = load_key(&config.key_path)?;

        let mut root_store = RootCertStore::empty();
        if let Some(ca_path) = &config.ca_cert_path {
            let ca_certs = load_certs(ca_path)?;
            for cert in ca_certs {
                root_store.add(cert)?;
            }
        }

        // For mTLS, we need a client cert verifier
        let client_verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(root_store))
            .build()
            .context("failed to build client verifier")?;

        let server_config = ServerConfig::builder()
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(certs, key)
            .context("failed to create server config")?;

        Ok(server_config)
    }
}

/// Loads one or more certificates from a PEM-encoded string.
pub fn load_certs_from_memory(data: &str) -> Result<Vec<CertificateDer<'static>>> {
    let mut reader = BufReader::new(data.as_bytes());
    let certs = certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .context("failed to load certs from memory")?;
    Ok(certs)
}

/// Loads a private key from a PEM-encoded string.
pub fn load_key_from_memory(data: &str) -> Result<PrivateKeyDer<'static>> {
    let mut reader = BufReader::new(data.as_bytes());
    private_key(&mut reader)
        .context("failed to load key from memory")?
        .context("no key found in memory")
}

/// Builds a static `ClientConfig` using in-memory PEM strings.
pub fn build_client_config(
    ca_cert: Option<&str>,
    client_cert: Option<&str>,
    client_key: Option<&str>,
) -> Result<ClientConfig> {
    let mut root_store = RootCertStore::empty();

    // Add native roots
    let native_certs = rustls_native_certs::load_native_certs();
    for cert in native_certs.certs {
        root_store.add(cert)?;
    }
    if !native_certs.errors.is_empty() {
        warn!(
            "Errors loading native certificates: {:?}",
            native_certs.errors
        );
    }

    if let Some(ca) = ca_cert {
        let ca_certs = load_certs_from_memory(ca)?;
        for cert in ca_certs {
            root_store.add(cert)?;
        }
    }

    let builder = ClientConfig::builder().with_root_certificates(root_store);

    if let (Some(cert), Some(key)) = (client_cert, client_key) {
        let certs = load_certs_from_memory(cert)?;
        let key = load_key_from_memory(key)?;
        Ok(builder.with_client_auth_cert(certs, key)?)
    } else {
        Ok(builder.with_no_client_auth())
    }
}

fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>> {
    let file = File::open(path).with_context(|| format!("failed to open cert file {:?}", path))?;
    let mut reader = BufReader::new(file);
    let certs = certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("failed to load certs from {:?}", path))?;
    Ok(certs)
}

fn load_key(path: &Path) -> Result<PrivateKeyDer<'static>> {
    let file = File::open(path).with_context(|| format!("failed to open key file {:?}", path))?;
    let mut reader = BufReader::new(file);
    private_key(&mut reader)
        .with_context(|| format!("failed to load key from {:?}", path))?
        .context("no key found in file")
}

impl Default for TlsConfig {
    fn default() -> Self {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../");
        Self {
            ca_cert_path: Some(base.join("tests/certs/ca.crt")),
            cert_path: base.join("tests/certs/tls.crt"),
            key_path: base.join("tests/certs/tls.key"),
            server_name: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Once;
    use tempfile::tempdir;

    static INIT: Once = Once::new();

    fn init_test() {
        INIT.call_once(|| {
            rustls::crypto::ring::default_provider()
                .install_default()
                .expect("Failed to install default crypto provider");
        });
    }

    const TEST_CERT: &str = include_str!("../../../tests/certs/tls.crt");
    const TEST_KEY: &str = include_str!("../../../tests/certs/tls.key");
    const TEST_CA: &str = include_str!("../../../tests/certs/ca.crt");

    #[test]
    fn test_load_certs_from_memory() {
        init_test();
        let certs = load_certs_from_memory(TEST_CERT).unwrap();
        assert!(!certs.is_empty());
    }

    #[test]
    fn test_load_key_from_memory() {
        init_test();
        let _key = load_key_from_memory(TEST_KEY).unwrap();
    }

    #[test]
    fn test_build_client_config() {
        init_test();
        let config = build_client_config(Some(TEST_CA), Some(TEST_CERT), Some(TEST_KEY)).unwrap();
        // Just verify we can build it without error
        drop(config);
    }

    #[test]
    fn test_build_client_config_no_auth() {
        init_test();
        let config = build_client_config(Some(TEST_CA), None, None).unwrap();
        drop(config);
    }

    #[tokio::test]
    async fn test_tls_reloader_initial_load() {
        init_test();
        let dir = tempdir().unwrap();
        let ca_path = dir.path().join("ca.crt");
        let cert_path = dir.path().join("tls.crt");
        let key_path = dir.path().join("tls.key");

        File::create(&ca_path)
            .unwrap()
            .write_all(TEST_CA.as_bytes())
            .unwrap();
        File::create(&cert_path)
            .unwrap()
            .write_all(TEST_CERT.as_bytes())
            .unwrap();
        File::create(&key_path)
            .unwrap()
            .write_all(TEST_KEY.as_bytes())
            .unwrap();

        let config = TlsConfig {
            ca_cert_path: Some(ca_path),
            cert_path,
            key_path,
            server_name: None,
        };

        let reloader = TlsReloader::new(config).await.unwrap();
        let _client_cfg = reloader.client_config();
        let _server_cfg = reloader.server_config();
    }
}
