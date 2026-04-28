use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

#[async_trait]
pub trait SecretBackend: Send + Sync {
    async fn get_secret(&self, path: &str, key: &str) -> Result<String>;
}

pub struct VaultBackend {
    #[cfg(feature = "vault")]
    client: vaultrs::client::VaultClient,
    #[cfg(not(feature = "vault"))]
    _address: String,
}

impl VaultBackend {
    pub fn new(address: String, _token: String) -> Result<Self> {
        #[cfg(feature = "vault")]
        {
            use vaultrs::client::VaultClientSettingsBuilder;
            let settings = VaultClientSettingsBuilder::default()
                .address(address)
                .token(_token)
                .build()?;
            let client = vaultrs::client::VaultClient::new(settings)?;
            Ok(Self { client })
        }
        #[cfg(not(feature = "vault"))]
        {
            Ok(Self { _address: address })
        }
    }
}

#[async_trait]
impl SecretBackend for VaultBackend {
    async fn get_secret(&self, _path: &str, _key: &str) -> Result<String> {
        #[cfg(feature = "vault")]
        {
            use vaultrs::kv2;
            let secret: HashMap<String, String> = kv2::read(&self.client, "secret", _path).await?;
            secret
                .get(_key)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Key {} not found in path {}", _key, _path))
        }
        #[cfg(not(feature = "vault"))]
        {
            anyhow::bail!("Vault backend is not enabled in this build. Enable 'vault' feature.")
        }
    }
}

pub struct MockBackend {
    pub secrets: HashMap<String, String>,
}

impl MockBackend {
    pub fn new(secrets: HashMap<String, String>) -> Self {
        Self { secrets }
    }
}

#[async_trait]
impl SecretBackend for MockBackend {
    async fn get_secret(&self, path: &str, key: &str) -> Result<String> {
        let full_key = format!("{}:{}", path, key);
        self.secrets
            .get(&full_key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Secret not found: {}", full_key))
    }
}

pub type SharedSecretBackend = Arc<dyn SecretBackend>;
