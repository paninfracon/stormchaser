//! The clients the TUI talks to the API, the query service and the identity provider with.

use anyhow::Context;
use std::path::Path;
use std::sync::OnceLock;
use stormchaser_client::Client;

/// The PEM bundle of CA certificates trusted in addition to the system's, from `--ca-cert`.
static EXTRA_ROOTS: OnceLock<Vec<u8>> = OnceLock::new();

/// Trusts the CA certificates in the PEM file at `path` as well as the system's, for
/// a server whose certificate a private CA signed. Call before building any client.
pub fn trust_ca(path: &Path) -> anyhow::Result<()> {
    let pem = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let certs = reqwest::Certificate::from_pem_bundle(&pem)
        .with_context(|| format!("{} is not a PEM bundle", path.display()))?;
    anyhow::ensure!(!certs.is_empty(), "{} holds no certificate", path.display());
    EXTRA_ROOTS
        .set(pem)
        .map_err(|_| anyhow::anyhow!("the CA certificate is already set"))
}

/// A client builder that trusts the system's CAs and any given with `--ca-cert`, for
/// requests that are not API calls (the identity provider's login pages).
#[expect(
    clippy::disallowed_methods,
    reason = "a client on the user's machine, calling servers its user names"
)]
pub fn builder() -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder();
    match EXTRA_ROOTS.get() {
        Some(pem) => builder.tls_certs_merge(
            reqwest::Certificate::from_pem_bundle(pem).expect("checked by trust_ca"),
        ),
        None => builder,
    }
}

/// A client of the API at `url` and the query service at `query_url`, without a token,
/// that trusts the system's CAs and any given with `--ca-cert`.
pub fn api_client(url: &str, query_url: &str) -> Client {
    let mut builder = Client::builder(url).query_url(query_url);
    if let Some(pem) = EXTRA_ROOTS.get() {
        builder = builder.ca_pem(pem.clone());
    }
    builder.build().expect("failed to build the HTTP client")
}
