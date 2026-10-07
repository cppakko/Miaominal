//! Shared HTTP client construction for the sync providers.
//!
//! Android cannot use reqwest's default rustls platform verifier without
//! additional JVM integration: certificate verification goes through
//! `rustls-platform-verifier`, which panics until the hosting JVM and the
//! application context are registered. That panic happens inside the spawned
//! request task, so HTTPS requests would die silently instead of reporting an
//! error. Trusting the bundled Mozilla root store keeps certificate
//! verification self-contained on Android. Desktop keeps the default platform
//! verifier so certificates trusted by the operating system remain usable.

#[cfg(target_os = "android")]
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};

pub(crate) fn client(
    connect_timeout: Duration,
    request_timeout: Duration,
) -> Result<reqwest::Client> {
    client_builder(connect_timeout, request_timeout)?
        .build()
        .context("failed to build the sync HTTP client")
}

/// Apply the shared TLS policy before callers customize request behavior.
pub(crate) fn client_builder(
    connect_timeout: Duration,
    request_timeout: Duration,
) -> Result<reqwest::ClientBuilder> {
    let builder = reqwest::Client::builder()
        .connect_timeout(connect_timeout)
        .timeout(request_timeout);
    #[cfg(target_os = "android")]
    let builder = builder.use_preconfigured_tls(bundled_root_tls_config()?);
    Ok(builder)
}

#[cfg(target_os = "android")]
fn bundled_root_tls_config() -> Result<rustls::ClientConfig> {
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .context("failed to configure the sync TLS protocol versions")?
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(tls)
}
