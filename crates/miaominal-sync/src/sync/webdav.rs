use anyhow::{Context, Result, bail};
use reqwest::{Client, Url};

use super::providers::PushCondition;

pub(super) enum WebDavPushOutcome {
    Pushed { etag: Option<String> },
    Conflict,
}

pub(super) enum WebDavPullOutcome {
    Missing,
    NotModified,
    Payload {
        content: String,
        etag: Option<String>,
    },
}

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

    /// Upload `payload_json` with HTTP PUT. Returns the response ETag when the
    /// server provides one so the engine can later use conditional requests.
    pub async fn push(
        &self,
        payload_json: &str,
        condition: &PushCondition,
    ) -> Result<WebDavPushOutcome> {
        let mut request = self
            .client
            .put(&self.url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Content-Type", "application/json")
            .body(payload_json.to_string());
        request = match condition {
            PushCondition::IfMatch(etag) => request.header(reqwest::header::IF_MATCH, etag),
            PushCondition::MustNotExist => request.header(reqwest::header::IF_NONE_MATCH, "*"),
            PushCondition::Unconditional => request,
        };
        let response = request
            .send()
            .await
            .context("failed to PUT to WebDAV server")?;

        if response.status() == reqwest::StatusCode::PRECONDITION_FAILED {
            return Ok(WebDavPushOutcome::Conflict);
        }
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            bail!("WebDAV PUT failed: {status} - {text}");
        }
        Ok(WebDavPushOutcome::Pushed {
            etag: response_etag(&response),
        })
    }

    /// Download the payload JSON with HTTP GET.
    /// Returns `None` when the resource does not exist yet (HTTP 404) and
    /// `NotModified` when `etag` matches the remote representation (HTTP 304).
    pub async fn pull(&self, etag: Option<&str>) -> Result<WebDavPullOutcome> {
        let mut request = self
            .client
            .get(&self.url)
            .basic_auth(&self.username, Some(&self.password));
        if let Some(etag) = etag {
            request = request.header("If-None-Match", etag);
        }
        let response = request
            .send()
            .await
            .context("failed to GET from WebDAV server")?;

        if response.status().as_u16() == 304 {
            return Ok(WebDavPullOutcome::NotModified);
        }
        if response.status().as_u16() == 404 {
            return Ok(WebDavPullOutcome::Missing);
        }
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            bail!("WebDAV GET failed: {status} - {text}");
        }

        let remote_etag = response_etag(&response);
        let content = response
            .text()
            .await
            .context("failed to read WebDAV response body")?;
        Ok(WebDavPullOutcome::Payload {
            content,
            etag: remote_etag,
        })
    }
}

fn response_etag(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
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
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn test_server(status: &str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let address = listener.local_addr().expect("test address should resolve");
        let status = status.to_string();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should connect");
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let read = stream.read(&mut buffer).expect("request should read");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .expect("response should write");
            String::from_utf8(request).expect("request should be UTF-8")
        });
        (format!("http://{address}/sync.json"), handle)
    }

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

    #[tokio::test]
    async fn push_sends_if_match_and_maps_precondition_failure_to_conflict() {
        let (url, request) = test_server("412 Precondition Failed");
        let backend = WebDavBackend::new(url, "user".into(), "password".into()).unwrap();

        let outcome = backend
            .push("{}", &PushCondition::IfMatch("\"etag-v1\"".into()))
            .await
            .expect("412 should be a sync conflict");

        assert!(matches!(outcome, WebDavPushOutcome::Conflict));
        assert!(
            request
                .join()
                .expect("test server should finish")
                .to_ascii_lowercase()
                .contains("if-match: \"etag-v1\"")
        );
    }

    #[tokio::test]
    async fn first_webdav_push_requires_the_resource_to_be_absent() {
        let (url, request) = test_server("201 Created");
        let backend = WebDavBackend::new(url, "user".into(), "password".into()).unwrap();

        let outcome = backend
            .push("{}", &PushCondition::MustNotExist)
            .await
            .expect("create should succeed");

        assert!(matches!(outcome, WebDavPushOutcome::Pushed { .. }));
        assert!(
            request
                .join()
                .expect("test server should finish")
                .to_ascii_lowercase()
                .contains("if-none-match: *")
        );
    }
}
