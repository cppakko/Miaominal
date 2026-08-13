use anyhow::{Result, anyhow};

use super::github_gist::{GithubGistBackend, GithubGistPullOutcome, GithubGistPushOutcome};
use super::store::SyncConfigStore;
use super::webdav::{WebDavBackend, WebDavPullOutcome, WebDavPushOutcome};
use crate::SyncProvider;

/// Outcome returned after a successful push. `provider_resource_id` lets
/// providers (currently GitHub Gist) report a resource id that the engine must
/// persist so subsequent pushes target the same remote object. `etag` lets the
/// engine persist the remote representation for conditional pulls.
pub(super) enum PushCondition {
    IfMatch(String),
    MustNotExist,
    Unconditional,
}

pub(super) enum PushOutcome {
    Pushed {
        provider_resource_id: Option<String>,
        etag: Option<String>,
    },
    Conflict,
}

pub(super) struct PullPayload {
    pub json: String,
    pub etag: Option<String>,
}

pub(super) enum PullOutcome {
    BindingRequired { provider: SyncProvider },
    Missing { etag: Option<String> },
    NotModified,
    Payload(PullPayload),
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

    pub(super) async fn push(
        &mut self,
        payload_json: &str,
        condition: &PushCondition,
    ) -> Result<PushOutcome> {
        match self {
            Self::Gist(backend) => match backend.push(payload_json, condition).await? {
                GithubGistPushOutcome::Pushed { gist_id, etag } => Ok(PushOutcome::Pushed {
                    provider_resource_id: Some(gist_id),
                    etag,
                }),
                GithubGistPushOutcome::Conflict => Ok(PushOutcome::Conflict),
            },
            Self::WebDav(backend) => match backend.push(payload_json, condition).await? {
                WebDavPushOutcome::Pushed { etag } => Ok(PushOutcome::Pushed {
                    provider_resource_id: None,
                    etag,
                }),
                WebDavPushOutcome::Conflict => Ok(PushOutcome::Conflict),
            },
        }
    }

    pub(super) async fn pull(&self, etag: Option<&str>) -> Result<PullOutcome> {
        match self {
            Self::Gist(backend) => match backend.pull(etag).await? {
                GithubGistPullOutcome::BindingRequired => Ok(PullOutcome::BindingRequired {
                    provider: SyncProvider::GithubGist,
                }),
                GithubGistPullOutcome::Missing { etag } => Ok(PullOutcome::Missing { etag }),
                GithubGistPullOutcome::NotModified => Ok(PullOutcome::NotModified),
                GithubGistPullOutcome::Payload { content, etag } => {
                    Ok(PullOutcome::Payload(PullPayload {
                        json: content,
                        etag,
                    }))
                }
            },
            Self::WebDav(backend) => match backend.pull(etag).await? {
                WebDavPullOutcome::Missing => Ok(PullOutcome::Missing { etag: None }),
                WebDavPullOutcome::NotModified => Ok(PullOutcome::NotModified),
                WebDavPullOutcome::Payload { content, etag } => {
                    Ok(PullOutcome::Payload(PullPayload {
                        json: content,
                        etag,
                    }))
                }
            },
        }
    }
}
