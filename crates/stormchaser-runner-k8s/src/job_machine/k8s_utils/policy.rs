use anyhow::{bail, Result};

use super::StepSpec;

/// Comma-separated ServiceAccount names a step may request.
pub const ALLOWED_SERVICE_ACCOUNTS_ENV: &str = "STORMCHASER_ALLOWED_SERVICE_ACCOUNTS";
/// Comma-separated Secret names a step may mount.
pub const ALLOWED_SECRET_MOUNTS_ENV: &str = "STORMCHASER_ALLOWED_SECRET_MOUNTS";
/// Comma-separated ConfigMap names a step may mount.
pub const ALLOWED_CONFIG_MAP_MOUNTS_ENV: &str = "STORMCHASER_ALLOWED_CONFIG_MAP_MOUNTS";

/// What the operator lets a workflow step borrow from the cluster.
///
/// Step specs are written by workflow authors, so the namespace objects they can attach
/// to a pod are limited to names the operator has listed. Every list is empty by default.
/// An entry ending in `*` matches any name with that prefix.
#[derive(Debug, Default, Clone)]
pub(crate) struct StepPolicy {
    pub(crate) allowed_service_accounts: Vec<String>,
    pub(crate) allowed_secret_mounts: Vec<String>,
    pub(crate) allowed_config_map_mounts: Vec<String>,
}

fn parse_list(value: Option<String>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn is_listed(allowed: &[String], name: &str) -> bool {
    allowed.iter().any(|entry| match entry.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => entry == name,
    })
}

impl StepPolicy {
    pub(crate) fn from_env() -> Self {
        Self {
            allowed_service_accounts: parse_list(std::env::var(ALLOWED_SERVICE_ACCOUNTS_ENV).ok()),
            allowed_secret_mounts: parse_list(std::env::var(ALLOWED_SECRET_MOUNTS_ENV).ok()),
            allowed_config_map_mounts: parse_list(
                std::env::var(ALLOWED_CONFIG_MAP_MOUNTS_ENV).ok(),
            ),
        }
    }

    /// Rejects a step that asks for a ServiceAccount, Secret or ConfigMap the operator
    /// has not listed.
    pub(crate) fn check(&self, step_spec: &StepSpec) -> Result<()> {
        if let Some(sa) = &step_spec.service_account_name {
            if !is_listed(&self.allowed_service_accounts, sa) {
                bail!(
                    "Step requests service account '{}', which this runner does not permit (list it in {})",
                    sa,
                    ALLOWED_SERVICE_ACCOUNTS_ENV
                );
            }
        }
        for sm in &step_spec.secret_mounts {
            if !is_listed(&self.allowed_secret_mounts, &sm.name) {
                bail!(
                    "Step mounts secret '{}', which this runner does not permit (list it in {})",
                    sm.name,
                    ALLOWED_SECRET_MOUNTS_ENV
                );
            }
        }
        for cm in &step_spec.config_map_mounts {
            if !is_listed(&self.allowed_config_map_mounts, &cm.name) {
                bail!(
                    "Step mounts config map '{}', which this runner does not permit (list it in {})",
                    cm.name,
                    ALLOWED_CONFIG_MAP_MOUNTS_ENV
                );
            }
        }
        Ok(())
    }
}
