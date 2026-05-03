use anyhow::{Context, Result};
use dashmap::DashMap;
use http::Request;
use kube::Client;
use serde_json::Value;
use tracing::{error, info};

/// Pool of Kubernetes cluster connections
pub struct ClusterPool {
    clients: DashMap<String, (Client, String)>, // (Client, Version)
}

impl Default for ClusterPool {
    fn default() -> Self {
        Self::new()
    }
}

impl ClusterPool {
    /// New.
    pub fn new() -> Self {
        Self {
            clients: DashMap::new(),
        }
    }

    /// Get a client and version for a named cluster, or the default if not found
    pub async fn get_client(&self, cluster_name: &str) -> Result<(Client, String)> {
        if let Some(entry) = self.clients.get(cluster_name) {
            return Ok(entry.clone());
        }

        let client = Client::try_default().await.context(format!(
            "Failed to create default K8s client for cluster {}",
            cluster_name
        ))?;

        // Query cluster version
        let version_resp = client
            .request_text(Request::builder().uri("/version").body(vec![])?)
            .await?;

        let version_data: Value = serde_json::from_str(&version_resp)?;
        let major = version_data["major"].as_str().unwrap_or("0");
        let minor = version_data["minor"].as_str().unwrap_or("0");
        let version = format!("{}.{}", major, minor.replace('+', ""));

        info!(
            "Connected to Kubernetes cluster {}: v{}",
            cluster_name, version
        );

        let major_int: i32 = major.parse().unwrap_or(0);
        let minor_int: i32 = minor.replace('+', "").parse().unwrap_or(0);

        if major_int < 1 || (major_int == 1 && minor_int < 21) {
            let err_msg = format!("Kubernetes cluster {} version v{}.{} is not supported. Minimum required is v1.21.0", cluster_name, major, minor);
            error!("{}", err_msg);
            return Err(anyhow::anyhow!(err_msg));
        }

        self.clients
            .insert(cluster_name.to_string(), (client.clone(), version.clone()));
        Ok((client, version))
    }

    /// Add a pre-configured client to the pool
    pub fn add_client(&self, cluster_name: &str, client: Client, version: String) {
        self.clients
            .insert(cluster_name.to_string(), (client, version));
    }

    /// Get list of all cluster names in the pool
    pub fn cluster_names(&self) -> Vec<String> {
        self.clients.iter().map(|r| r.key().clone()).collect()
    }
}
