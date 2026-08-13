use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::providers::PushCondition;

const GIST_FILENAME: &str = "miaominal_sync.json";

#[derive(Debug, Serialize)]
struct GistFile {
    content: String,
}

#[derive(Debug, Serialize)]
struct CreateGistRequest {
    description: String,
    public: bool,
    files: HashMap<String, GistFile>,
}

#[derive(Debug, Deserialize)]
struct CreateGistResponse {
    id: String,
}

pub(super) enum GithubGistPullOutcome {
    BindingRequired,
    Missing {
        etag: Option<String>,
    },
    NotModified,
    Payload {
        content: String,
        etag: Option<String>,
    },
}

pub(super) enum GithubGistPushOutcome {
    Pushed {
        gist_id: String,
        etag: Option<String>,
    },
    Conflict,
}

pub struct GithubGistBackend {
    client: Client,
    token: String,
    pub gist_id: Option<String>,
}

impl GithubGistBackend {
    pub fn new(token: String, gist_id: Option<String>) -> Self {
        Self {
            client: Client::new(),
            token,
            gist_id,
        }
    }

    /// Push `payload_json` to the Gist. Creates the Gist if no `gist_id` is set.
    /// Returns the Gist ID (new or existing) and the response ETag when
    /// GitHub provides one.
    pub async fn push(
        &mut self,
        payload_json: &str,
        condition: &PushCondition,
    ) -> Result<GithubGistPushOutcome> {
        let mut files = HashMap::new();
        files.insert(
            GIST_FILENAME.to_string(),
            GistFile {
                content: payload_json.to_string(),
            },
        );

        if let Some(ref id) = self.gist_id {
            // GitHub's Gist PATCH endpoint cannot atomically create a missing
            // file. A plain PATCH here could overwrite a file another device
            // created after our GET, so reject the weak precondition before
            // issuing any request.
            if matches!(condition, PushCondition::MustNotExist) {
                return Ok(GithubGistPushOutcome::Conflict);
            }

            let url = format!("https://api.github.com/gists/{id}");
            let body = serde_json::json!({ "files": files });
            let mut request = self
                .client
                .patch(&url)
                .header("Authorization", format!("Bearer {}", self.token))
                .header("User-Agent", "miaominal")
                .json(&body);
            request = match condition {
                PushCondition::IfMatch(etag) => request.header(reqwest::header::IF_MATCH, etag),
                PushCondition::Unconditional => request,
                PushCondition::MustNotExist => unreachable!("handled before building PATCH"),
            };
            let response = request
                .send()
                .await
                .context("failed to update GitHub Gist")?;

            if response.status() == reqwest::StatusCode::PRECONDITION_FAILED {
                return Ok(GithubGistPushOutcome::Conflict);
            }
            if !response.status().is_success() {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                bail!("GitHub Gist update failed: {status} - {text}");
            }
            let etag = response_etag(&response);
            Ok(GithubGistPushOutcome::Pushed {
                gist_id: id.clone(),
                etag,
            })
        } else {
            let request = CreateGistRequest {
                description: "Miaominal configuration sync".to_string(),
                public: false,
                files,
            };
            let response = self
                .client
                .post("https://api.github.com/gists")
                .header("Authorization", format!("Bearer {}", self.token))
                .header("User-Agent", "miaominal")
                .json(&request)
                .send()
                .await
                .context("failed to create GitHub Gist")?;

            if !response.status().is_success() {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                bail!("GitHub Gist create failed: {status} - {text}");
            }
            let etag = response_etag(&response);
            let gist: CreateGistResponse = response
                .json()
                .await
                .context("failed to parse Gist response")?;
            self.gist_id = Some(gist.id.clone());
            Ok(GithubGistPushOutcome::Pushed {
                gist_id: gist.id,
                etag,
            })
        }
    }

    /// Pull the current payload JSON from the configured Gist.
    /// Returns `BindingRequired` when no Gist ID has been configured yet and
    /// `NotModified` when `etag` matches the remote representation (HTTP 304).
    pub async fn pull(&self, etag: Option<&str>) -> Result<GithubGistPullOutcome> {
        let id = match &self.gist_id {
            Some(id) => id,
            None => return Ok(GithubGistPullOutcome::BindingRequired),
        };

        #[derive(Deserialize)]
        struct GistFileContent {
            content: String,
        }

        #[derive(Deserialize)]
        struct GistGetResponse {
            files: HashMap<String, GistFileContent>,
        }

        let url = format!("https://api.github.com/gists/{id}");
        let mut request = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "miaominal");
        if let Some(etag) = etag {
            request = request.header("If-None-Match", etag);
        }
        let response = request
            .send()
            .await
            .context("failed to fetch GitHub Gist")?;

        if response.status().as_u16() == 304 {
            return Ok(GithubGistPullOutcome::NotModified);
        }
        if response.status().as_u16() == 404 {
            bail!(
                "GitHub Gist fetch failed: configured Gist {id} was not found or is not accessible with the current token"
            );
        }
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            bail!("GitHub Gist fetch failed: {status} - {text}");
        }

        let remote_etag = response_etag(&response);
        let gist: GistGetResponse = response
            .json()
            .await
            .context("failed to parse Gist response")?;
        match gist.files.get(GIST_FILENAME) {
            Some(file) => Ok(GithubGistPullOutcome::Payload {
                content: file.content.clone(),
                etag: remote_etag,
            }),
            None => Ok(GithubGistPullOutcome::Missing { etag: remote_etag }),
        }
    }
}

fn response_etag(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bound_gist_refuses_non_atomic_must_not_exist_patch() {
        let mut backend = GithubGistBackend::new("unused".into(), Some("bound-gist".into()));

        let outcome = backend
            .push("{}", &PushCondition::MustNotExist)
            .await
            .expect("MustNotExist should be rejected before a network request");

        assert!(matches!(outcome, GithubGistPushOutcome::Conflict));
    }
}
