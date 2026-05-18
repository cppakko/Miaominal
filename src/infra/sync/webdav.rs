use anyhow::{Context, Result, bail};
use reqwest::{Client, Url};

pub struct WebDavBackend {
    client: Client,
    url: String,
    username: String,
    password: String,
}

impl WebDavBackend {
    pub fn new(url: String, username: String, password: String) -> Result<Self> {
        validate_webdav_url(&url)?;
        Ok(Self {
            client: Client::new(),
            url,
            username,
            password,
        })
    }

    /// Upload `payload_json` with HTTP PUT.
    pub async fn push(&self, payload_json: &str) -> Result<()> {
        let response = self
            .client
            .put(&self.url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Content-Type", "application/json")
            .body(payload_json.to_string())
            .send()
            .await
            .context("failed to PUT to WebDAV server")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            bail!("WebDAV PUT failed: {status} — {text}");
        }
        Ok(())
    }

    /// Download the payload JSON with HTTP GET.
    /// Returns `None` when the resource does not exist yet (HTTP 404).
    pub async fn pull(&self) -> Result<Option<String>> {
        let response = self
            .client
            .get(&self.url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .context("failed to GET from WebDAV server")?;

        if response.status().as_u16() == 404 {
            return Ok(None);
        }

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            bail!("WebDAV GET failed: {status} — {text}");
        }

        let content = response
            .text()
            .await
            .context("failed to read WebDAV response body")?;
        Ok(Some(content))
    }
}

fn validate_webdav_url(url: &str) -> Result<()> {
    let parsed = Url::parse(url).context("failed to parse WebDAV URL")?;
    match parsed.scheme() {
        "https" => Ok(()),
        "http" if is_localhost_url(&parsed) => Ok(()),
        "http" => bail!("WebDAV sync requires HTTPS unless the host is localhost"),
        scheme => bail!("unsupported WebDAV URL scheme: {scheme}"),
    }
}

fn is_localhost_url(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some("localhost") | Some("127.0.0.1") | Some("[::1]") | Some("::1")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_local_http_urls() {
        assert!(
            WebDavBackend::new(
                "http://example.com/sync.json".into(),
                "user".into(),
                "password".into(),
            )
            .is_err()
        );
    }

    #[test]
    fn allows_localhost_http_urls() {
        assert!(
            WebDavBackend::new(
                "http://localhost:8080/sync.json".into(),
                "user".into(),
                "password".into(),
            )
            .is_ok()
        );
    }
}
