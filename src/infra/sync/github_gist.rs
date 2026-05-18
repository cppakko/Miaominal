use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    Missing,
    Payload(String),
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
    /// Returns the Gist ID (new or existing).
    pub async fn push(&mut self, payload_json: &str) -> Result<String> {
        let mut files = HashMap::new();
        files.insert(
            GIST_FILENAME.to_string(),
            GistFile {
                content: payload_json.to_string(),
            },
        );

        if let Some(ref id) = self.gist_id {
            let url = format!("https://api.github.com/gists/{id}");
            let body = serde_json::json!({ "files": files });
            let response = self
                .client
                .patch(&url)
                .header("Authorization", format!("Bearer {}", self.token))
                .header("User-Agent", "miaominal")
                .json(&body)
                .send()
                .await
                .context("failed to update GitHub Gist")?;

            if !response.status().is_success() {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                bail!("GitHub Gist update failed: {status} — {text}");
            }
            Ok(id.clone())
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
                bail!("GitHub Gist create failed: {status} — {text}");
            }
            let gist: CreateGistResponse = response
                .json()
                .await
                .context("failed to parse Gist response")?;
            self.gist_id = Some(gist.id.clone());
            Ok(gist.id)
        }
    }

    /// Pull the current payload JSON from the configured Gist.
    /// Returns `BindingRequired` when no Gist ID has been configured yet.
    pub async fn pull(&self) -> Result<GithubGistPullOutcome> {
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
        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "miaominal")
            .send()
            .await
            .context("failed to fetch GitHub Gist")?;

        if response.status().as_u16() == 404 {
            bail!(
                "GitHub Gist fetch failed: configured Gist {id} was not found or is not accessible with the current token"
            );
        }

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            bail!("GitHub Gist fetch failed: {status} — {text}");
        }

        let gist: GistGetResponse = response
            .json()
            .await
            .context("failed to parse Gist response")?;
        match gist.files.get(GIST_FILENAME) {
            Some(file) => Ok(GithubGistPullOutcome::Payload(file.content.clone())),
            None => Ok(GithubGistPullOutcome::Missing),
        }
    }
}
