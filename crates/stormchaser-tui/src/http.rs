//! The HTTP clients the TUI talks to the API and the query service with.

use anyhow::Context;
use std::path::Path;
use std::sync::OnceLock;

/// CA certificates trusted in addition to the system's, from `--ca-cert`.
static EXTRA_ROOTS: OnceLock<Vec<reqwest::Certificate>> = OnceLock::new();

/// Trusts the CA certificates in the PEM file at `path` as well as the system's, for
/// a server whose certificate a private CA signed. Call before building any client.
pub fn trust_ca(path: &Path) -> anyhow::Result<()> {
    let pem = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let certs = reqwest::Certificate::from_pem_bundle(&pem)
        .with_context(|| format!("{} is not a PEM bundle", path.display()))?;
    anyhow::ensure!(!certs.is_empty(), "{} holds no certificate", path.display());
    EXTRA_ROOTS
        .set(certs)
        .map_err(|_| anyhow::anyhow!("the CA certificate is already set"))
}

/// A client builder that trusts the system's CAs and any given with `--ca-cert`.
pub fn builder() -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder();
    match EXTRA_ROOTS.get() {
        Some(certs) => builder.tls_certs_merge(certs.clone()),
        None => builder,
    }
}

/// A client that trusts the system's CAs and any given with `--ca-cert`.
pub fn client() -> reqwest::Client {
    builder().build().expect("failed to build the HTTP client")
}
