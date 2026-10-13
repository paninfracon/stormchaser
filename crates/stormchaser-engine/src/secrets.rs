use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

/// Comma-separated secret paths that workflows may read with `secret()`. A trailing `*`
/// matches a prefix. Empty by default, which refuses every lookup.
///
/// The engine's Vault token usually reaches more than any one workflow author should:
/// without a limit, whoever can write a workflow can read everything the token can.
pub const VAULT_ALLOWED_PATHS_ENV: &str = "STORMCHASER_VAULT_ALLOWED_PATHS";

fn path_is_allowed(allow_list: &str, path: &str) -> bool {
    // A prefix entry must not be escaped with `..`, and Vault ignores empty segments
    if path
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return false;
    }
    allow_list
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .any(|entry| match entry.strip_suffix('*') {
            Some(prefix) => path.starts_with(prefix),
            None => entry == path,
        })
}

/// Refuses a secret path the operator has not allow-listed.
pub fn ensure_secret_path_allowed(path: &str) -> Result<()> {
    let allow_list = std::env::var(VAULT_ALLOWED_PATHS_ENV).unwrap_or_default();
    if path_is_allowed(&allow_list, path) {
        Ok(())
    } else {
        anyhow::bail!(
            "Secret path {} is not in {}; the engine will not read it",
            path,
            VAULT_ALLOWED_PATHS_ENV
        )
    }
}

#[async_trait]
/// Secretbackend.
pub trait SecretBackend: Send + Sync {
    /// Retrieves a secret value from the specified path and key.
    async fn get_secret(&self, path: &str, key: &str) -> Result<String>;
}

/// A secret backend that interacts with HashiCorp Vault.
pub struct VaultBackend {
    #[cfg(feature = "vault")]
    client: vaultrs::client::VaultClient,
    #[cfg(not(feature = "vault"))]
    _address: String,
}

impl VaultBackend {
    /// Creates a new `VaultBackend` with the specified Vault address and token.
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

/// Mockbackend.
pub struct MockBackend {
    /// The secrets.
    pub secrets: HashMap<String, String>,
}

impl MockBackend {
    /// New.
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

/// A thread-safe, shareable reference to a `SecretBackend` implementation.
pub type SharedSecretBackend = Arc<dyn SecretBackend>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_list_allows_no_path() {
        assert!(!path_is_allowed("", "apps/db"));
        assert!(!path_is_allowed(" , ", "apps/db"));
    }

    #[test]
    fn paths_match_exactly_or_by_prefix() {
        let list = "shared/smtp, apps/*";
        assert!(path_is_allowed(list, "shared/smtp"));
        assert!(path_is_allowed(list, "apps/db"));
        assert!(!path_is_allowed(list, "shared/smtp-admin"));
        assert!(!path_is_allowed(list, "infra/root"));
    }

    #[test]
    fn a_prefix_cannot_be_escaped() {
        let list = "apps/*";
        assert!(!path_is_allowed(list, "apps/../infra/root"));
        assert!(!path_is_allowed(list, "apps//infra"));
        assert!(!path_is_allowed(list, "apps/./db"));
    }
}
