//! External RDP clients. No UI, terminal renderer, profile persistence or secret store.
mod client;
mod credentials;
mod file;

pub use client::{
    ClientProcessState, LaunchedClient, PasswordDelivery, RdpClientInfo, detect_clients, launch,
    resolve_client,
};
pub use file::{RdpEndpoint, cleanup_stale_files, render_connection_file};

/// Remove only expired credentials carrying this application's ownership marker.
pub fn cleanup_stale_credentials() -> anyhow::Result<()> {
    credentials::cleanup_stale()
}
