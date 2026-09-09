use anyhow::{Context, Result, bail};
use reqwest::{Client, Url};
use std::time::Duration;

use super::providers::PushCondition;
use crate::capability::{
    CapabilityError, CapabilityReason, CapabilityReport, EtagKind, classify_etag,
};

pub(crate) const WEBDAV_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const WEBDAV_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) enum WebDavPushOutcome {
    Pushed { etag: Option<String> },
    Conflict,
    ChangedAfterPush,
}

#[derive(Debug)]
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
        Self::new_with_timeouts(
            url,
            username,
            password,
            WEBDAV_CONNECT_TIMEOUT,
            WEBDAV_REQUEST_TIMEOUT,
        )
    }

    fn new_with_timeouts(
        url: String,
        username: String,
        password: String,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self> {
        validate_webdav_url(&url)?;
        Ok(Self {
            client: Client::builder()
                .connect_timeout(connect_timeout)
                .timeout(request_timeout)
                .build()
                .context("failed to build WebDAV HTTP client")?,
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
        if let PushCondition::IfMatch(tag) = condition {
            let kind = classify_etag(Some(tag));
            if kind != EtagKind::Strong {
                return Err(CapabilityError(CapabilityReport::issue(
                    CapabilityReason::VersionUnavailable,
                    "upload-precondition",
                    None,
                    kind,
                ))
                .into());
            }
        }
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
        let mut etag = response_etag(&response);
        if !matches!(condition, PushCondition::Unconditional) {
            if classify_etag(etag.as_deref()) != EtagKind::Strong {
                // A successful PUT may omit ETag. Bind the fetched version only
                // to the snapshot we uploaded, never to a concurrent writer.
                drop(response);
                match self.pull_checked(None, true).await? {
                    WebDavPullOutcome::Payload {
                        content,
                        etag: fetched,
                    } if content == payload_json => etag = fetched,
                    _ => return Ok(WebDavPushOutcome::ChangedAfterPush),
                }
            }
            if let PushCondition::IfMatch(previous) = condition
                && etag.as_ref() == Some(previous)
            {
                return Err(CapabilityError(CapabilityReport::issue(
                    CapabilityReason::ConditionalWrite,
                    "upload-version-unchanged",
                    None,
                    EtagKind::Strong,
                ))
                .into());
            }
        }
        Ok(WebDavPushOutcome::Pushed { etag })
    }

    /// Download the payload JSON with HTTP GET.
    /// Returns `None` when the resource does not exist yet (HTTP 404) and
    /// `NotModified` when `etag` matches the remote representation (HTTP 304).
    pub async fn pull(&self, etag: Option<&str>) -> Result<WebDavPullOutcome> {
        self.pull_checked(etag, false).await
    }

    pub async fn pull_checked(
        &self,
        etag: Option<&str>,
        automatic: bool,
    ) -> Result<WebDavPullOutcome> {
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
            if automatic {
                let returned = response_etag(&response);
                let kind = classify_etag(returned.as_deref().or(etag));
                if etag.is_none()
                    || kind != EtagKind::Strong
                    || returned.as_deref().is_some_and(|tag| Some(tag) != etag)
                {
                    return Err(CapabilityError(CapabilityReport::issue(
                        CapabilityReason::VersionUnavailable,
                        "poll-304",
                        Some(304),
                        kind,
                    ))
                    .into());
                }
            }
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
        if automatic {
            let kind = classify_etag(remote_etag.as_deref());
            if kind != EtagKind::Strong {
                return Err(CapabilityError(CapabilityReport::issue(
                    CapabilityReason::VersionUnavailable,
                    "poll",
                    Some(200),
                    kind,
                ))
                .into());
            }
            if etag.is_some() && etag == remote_etag.as_deref() {
                return Err(CapabilityError(CapabilityReport::issue(
                    CapabilityReason::ConditionalRead,
                    "poll",
                    Some(200),
                    kind,
                ))
                .into());
            }
        }
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
        .map(|value| value.to_str().unwrap_or("invalid").to_owned())
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

    fn stalled_server(delay: Duration) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let address = listener.local_addr().expect("test address should resolve");
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should connect");
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);
            std::thread::sleep(delay);
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
        let (url, request) = test_server("201 Created\r\nETag: \"created\"");
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

    #[tokio::test]
    async fn request_timeout_releases_a_stalled_webdav_operation() {
        let (url, server) = stalled_server(Duration::from_millis(250));
        let backend = WebDavBackend::new_with_timeouts(
            url,
            "user".into(),
            "password".into(),
            Duration::from_millis(50),
            Duration::from_millis(50),
        )
        .expect("test backend should build");
        let started = std::time::Instant::now();

        let error = backend
            .pull(None)
            .await
            .expect_err("a stalled response must time out");

        assert!(started.elapsed() < Duration::from_millis(200));
        assert!(error.to_string().contains("failed to GET"));
        server.join().expect("stalled server should finish");
    }

    #[tokio::test]
    async fn conditional_upload_rejects_weak_or_invalid_tags_before_sending() {
        let backend = WebDavBackend::new(
            "http://127.0.0.1:1/sync.json".into(),
            "user".into(),
            "password".into(),
        )
        .unwrap();
        for tag in ["W/\"weak\"", "", "unquoted", "*"] {
            let error = backend
                .push("{}", &PushCondition::IfMatch(tag.into()))
                .await
                .err()
                .unwrap();
            assert_eq!(
                error.downcast_ref::<CapabilityError>().unwrap().0.step,
                "upload-precondition"
            );
        }
    }

    #[tokio::test]
    async fn conditional_poll_rejects_ignored_conditions_and_invalid_versions() {
        for (response, sent, reason) in [
            (
                "200 OK\r\nETag: \"same\"",
                Some("\"same\""),
                CapabilityReason::ConditionalRead,
            ),
            (
                "200 OK\r\nETag: W/\"weak\"",
                None,
                CapabilityReason::VersionUnavailable,
            ),
            ("200 OK", None, CapabilityReason::VersionUnavailable),
            (
                "200 OK\r\nETag: invalid",
                None,
                CapabilityReason::VersionUnavailable,
            ),
            (
                "304 Not Modified",
                None,
                CapabilityReason::VersionUnavailable,
            ),
            (
                "304 Not Modified\r\nETag: \"other\"",
                Some("\"same\""),
                CapabilityReason::VersionUnavailable,
            ),
        ] {
            let (url, server) = test_server(response);
            let backend = WebDavBackend::new(url, "user".into(), "password".into()).unwrap();
            let error = backend.pull_checked(sent, true).await.unwrap_err();
            assert_eq!(
                error.downcast_ref::<CapabilityError>().unwrap().0.reason,
                Some(reason)
            );
            server.join().unwrap();
        }
    }

    #[tokio::test]
    async fn upload_without_etag_reads_back_only_the_uploaded_snapshot() {
        for condition in [
            PushCondition::MustNotExist,
            PushCondition::IfMatch("\"old\"".into()),
        ] {
            for body in ["{}", "concurrent-edit"] {
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                let url = format!("http://{}/sync.json", listener.local_addr().unwrap());
                let server = std::thread::spawn(move || {
                    for method in ["PUT", "GET"] {
                        let (mut stream, _) = listener.accept().unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        let mut bytes = [0; 4096];
                        let count = stream.read(&mut bytes).unwrap();
                        assert!(String::from_utf8_lossy(&bytes[..count]).starts_with(method));
                        let response = if method == "PUT" {
                            "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
                        } else {
                            format!(
                                "HTTP/1.1 200 OK\r\nETag: \"new\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            )
                        };
                        stream.write_all(response.as_bytes()).unwrap();
                    }
                });
                let backend = WebDavBackend::new(url, "user".into(), "password".into()).unwrap();
                let outcome = backend.push("{}", &condition).await.unwrap();
                if body == "{}" {
                    assert!(
                        matches!(outcome, WebDavPushOutcome::Pushed { etag: Some(tag) } if tag == "\"new\"")
                    );
                } else {
                    assert!(matches!(outcome, WebDavPushOutcome::ChangedAfterPush));
                }
                server.join().unwrap();
            }
        }
    }
}
