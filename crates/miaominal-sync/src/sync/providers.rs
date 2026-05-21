use anyhow::{Result, anyhow};

use super::github_gist::{GithubGistBackend, GithubGistPullOutcome};
use super::store::SyncConfigStore;
use super::webdav::WebDavBackend;
use crate::SyncProvider;

/// Outcome returned after a successful push. `provider_resource_id` lets
/// providers (currently GitHub Gist) report a resource id that the engine must
/// persist so subsequent pushes target the same remote object.
pub(super) struct PushOutcome {
    pub provider_resource_id: Option<String>,
}

pub(super) enum PullOutcome {
    BindingRequired { provider: SyncProvider },
    Missing,
    Payload(String),
}

/// Concrete backend handle constructed from `SyncConfigStore`. Encapsulates the
/// per-provider transport so the engine can share a single push/pull surface
/// for the current provider and avoid duplicated match arms.
pub(super) enum RemoteBackend {
    Gist(GithubGistBackend),
    WebDav(WebDavBackend),
}

impl RemoteBackend {
    /// Build the backend matching the active provider. Returns `Ok(None)` when
    /// sync is disabled (`SyncProvider::None`), and an error when credentials
    /// or settings required by the provider are missing.
    pub(super) fn build(config_store: &SyncConfigStore) -> Result<Option<Self>> {
        match config_store.config.provider {
            SyncProvider::None => Ok(None),
            SyncProvider::GithubGist => {
                let token = config_store
                    .get_github_token()?
                    .ok_or_else(|| anyhow!("GitHub token not configured"))?;
                Ok(Some(Self::Gist(GithubGistBackend::new(
                    token,
                    config_store.config.gist_id.clone(),
                ))))
            }
            SyncProvider::WebDav => {
                let password = config_store
                    .get_webdav_password()?
                    .ok_or_else(|| anyhow!("WebDAV password not configured"))?;
                Ok(Some(Self::WebDav(WebDavBackend::new(
                    config_store.config.webdav_url.clone(),
                    config_store.config.webdav_username.clone(),
                    password,
                )?)))
            }
        }
    }

    pub(super) async fn push(&mut self, payload_json: &str) -> Result<PushOutcome> {
        match self {
            Self::Gist(backend) => {
                let gist_id = backend.push(payload_json).await?;
                Ok(PushOutcome {
                    provider_resource_id: Some(gist_id),
                })
            }
            Self::WebDav(backend) => {
                backend.push(payload_json).await?;
                Ok(PushOutcome {
                    provider_resource_id: None,
                })
            }
        }
    }

    pub(super) async fn pull(&self) -> Result<PullOutcome> {
        match self {
            Self::Gist(backend) => match backend.pull().await? {
                GithubGistPullOutcome::BindingRequired => Ok(PullOutcome::BindingRequired {
                    provider: SyncProvider::GithubGist,
                }),
                GithubGistPullOutcome::Missing => Ok(PullOutcome::Missing),
                GithubGistPullOutcome::Payload(payload) => Ok(PullOutcome::Payload(payload)),
            },
            Self::WebDav(backend) => match backend.pull().await? {
                Some(payload) => Ok(PullOutcome::Payload(payload)),
                None => Ok(PullOutcome::Missing),
            },
        }
    }
}
