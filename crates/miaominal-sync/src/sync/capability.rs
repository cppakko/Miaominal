//! WebDAV capability checks use disposable data, never writes to the sync file.
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::{Client, Method, Response, Url};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EtagKind {
    #[default]
    Missing,
    Weak,
    Strong,
    Invalid,
}

pub fn classify_etag(value: Option<&str>) -> EtagKind {
    let Some(value) = value else {
        return EtagKind::Missing;
    };
    let (weak, tag) = value
        .strip_prefix("W/")
        .map_or((false, value), |tag| (true, tag));
    let bytes = tag.as_bytes();
    if bytes.len() < 2
        || bytes[0] != b'"'
        || bytes[bytes.len() - 1] != b'"'
        || !bytes[1..bytes.len() - 1]
            .iter()
            .all(|b| *b == 0x21 || (0x23..=0x7e).contains(b) || *b >= 0x80)
    {
        return EtagKind::Invalid;
    }
    if weak {
        EtagKind::Weak
    } else {
        EtagKind::Strong
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CapabilityState {
    #[default]
    Unchecked,
    Checking,
    Supported,
    Unsupported,
    Incomplete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityReason {
    VersionUnavailable,
    ConditionalRead,
    ConditionalWrite,
    Network,
    Authentication,
    Permission,
    Redirect,
    Http,
    Cancelled,
    Cleanup,
    Configuration,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CapabilityReport {
    pub state: CapabilityState,
    pub reason: Option<CapabilityReason>,
    pub step: String,
    pub http_status: Option<u16>,
    pub etag_kind: EtagKind,
    pub checked_at: u64,
    pub cleanup_file: Option<String>,
}

impl CapabilityReport {
    pub fn issue(
        reason: CapabilityReason,
        step: &str,
        status: Option<u16>,
        etag_kind: EtagKind,
    ) -> Self {
        Self {
            state: match reason {
                CapabilityReason::VersionUnavailable
                | CapabilityReason::ConditionalRead
                | CapabilityReason::ConditionalWrite => CapabilityState::Unsupported,
                _ => CapabilityState::Incomplete,
            },
            reason: Some(reason),
            step: step.into(),
            http_status: status,
            etag_kind,
            checked_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            cleanup_file: None,
        }
    }

    pub fn supported() -> Self {
        Self {
            state: CapabilityState::Supported,
            reason: None,
            step: "complete".into(),
            http_status: None,
            etag_kind: EtagKind::Strong,
            checked_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            cleanup_file: None,
        }
    }

    /// No URL, raw ETag, credentials, remote body or local configuration.
    pub fn diagnostics(&self) -> String {
        format!(
            "WebDAV capability: {:?}\nReason: {:?}\nStep: {}\nHTTP: {:?}\nETag: {:?}\nChecked at (Unix): {}\nTemporary file: {}",
            self.state,
            self.reason,
            self.step,
            self.http_status,
            self.etag_kind,
            self.checked_at,
            self.cleanup_file.as_deref().unwrap_or("none")
        )
    }
}

#[derive(Debug)]
pub struct CapabilityError(pub CapabilityReport);
impl std::fmt::Display for CapabilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "WebDAV capability check: {:?} ({})",
            self.0.reason, self.0.step
        )
    }
}
impl std::error::Error for CapabilityError {}

/// Cancellation generations let the UI invalidate an in-flight check without
/// aborting its cleanup future. A new operation captures the latest generation.
#[derive(Clone, Debug, Default)]
pub struct ProbeCancellation {
    generation: Arc<AtomicU64>,
    started: u64,
}
impl ProbeCancellation {
    pub fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
    pub fn fresh(&self) -> Self {
        Self {
            generation: self.generation.clone(),
            started: self.generation.load(Ordering::SeqCst),
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.started != self.generation.load(Ordering::SeqCst)
    }
}

/// Kept inside the service, never in UI snapshots or diagnostics. Retains the
/// original endpoint and credentials so changing settings cannot redirect cleanup.
pub struct PendingProbe {
    client: Client,
    url: Url,
    username: String,
    password: String,
    bodies: [String; 3],
    filename: String,
}
impl std::fmt::Debug for PendingProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingProbe")
            .field("filename", &self.filename)
            .finish_non_exhaustive()
    }
}

fn response_tag(response: &Response) -> Option<String> {
    response
        .headers()
        .get(reqwest::header::ETAG)
        .map(|v| v.to_str().unwrap_or("invalid").to_owned())
}

fn failure(step: &str, status: u16) -> CapabilityReport {
    CapabilityReport::issue(
        match status {
            401 => CapabilityReason::Authentication,
            403 => CapabilityReason::Permission,
            304 => CapabilityReason::ConditionalRead,
            300..=399 => CapabilityReason::Redirect,
            _ => CapabilityReason::Http,
        },
        step,
        Some(status),
        EtagKind::Missing,
    )
}

async fn small_body(mut response: Response, step: &str) -> Result<String, CapabilityReport> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        CapabilityReport::issue(CapabilityReason::Network, step, None, EtagKind::Missing)
    })? {
        if bytes.len() + chunk.len() > 8192 {
            return Err(failure(step, response.status().as_u16()));
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| failure(step, 200))
}

impl PendingProbe {
    async fn request(
        &self,
        method: Method,
        condition: Option<(&str, &str)>,
        body: Option<&str>,
        step: &str,
    ) -> Result<Response, CapabilityReport> {
        let mut request = self
            .client
            .request(method, self.url.clone())
            .basic_auth(&self.username, Some(&self.password));
        if let Some((name, value)) = condition {
            request = request.header(name, value);
        }
        if let Some(body) = body {
            request = request
                .header("Content-Type", "application/json")
                .body(body.to_owned());
        }
        request.send().await.map_err(|_| {
            CapabilityReport::issue(CapabilityReason::Network, step, None, EtagKind::Missing)
        })
    }

    async fn read(&self, step: &str) -> Result<(String, String), CapabilityReport> {
        let response = self.request(Method::GET, None, None, step).await?;
        if response.status().as_u16() != 200 {
            return Err(failure(step, response.status().as_u16()));
        }
        let tag = response_tag(&response);
        let kind = classify_etag(tag.as_deref());
        let body = small_body(response, step).await?;
        if kind != EtagKind::Strong {
            return Err(CapabilityReport::issue(
                CapabilityReason::VersionUnavailable,
                step,
                Some(200),
                kind,
            ));
        }
        Ok((body, tag.unwrap()))
    }

    async fn verify(&self, cancel: &ProbeCancellation) -> Result<(), CapabilityReport> {
        let checkpoint = || {
            if cancel.is_cancelled() {
                Err(CapabilityReport::issue(
                    CapabilityReason::Cancelled,
                    "cancel",
                    None,
                    EtagKind::Missing,
                ))
            } else {
                Ok(())
            }
        };
        checkpoint()?;
        let response = self
            .request(
                Method::PUT,
                Some(("If-None-Match", "*")),
                Some(&self.bodies[0]),
                "create",
            )
            .await?;
        if !response.status().is_success() {
            return Err(condition_failure(
                "create",
                response.status().as_u16(),
                false,
            ));
        }
        checkpoint()?;
        let (body, tag) = self.read("read-created").await?;
        if body != self.bodies[0] {
            return Err(CapabilityReport::issue(
                CapabilityReason::ConditionalWrite,
                "read-created",
                Some(200),
                EtagKind::Strong,
            ));
        }
        let response = self
            .request(
                Method::PUT,
                Some(("If-None-Match", "*")),
                Some(&self.bodies[2]),
                "create-existing",
            )
            .await?;
        if response.status().as_u16() != 412 {
            return Err(condition_failure(
                "create-existing",
                response.status().as_u16(),
                false,
            ));
        }
        let (body, _) = self.read("verify-create-rejected").await?;
        if body != self.bodies[0] {
            return Err(condition_failure("verify-create-rejected", 200, false));
        }
        checkpoint()?;
        self.conditional_read(&tag, 304, None, "read-current")
            .await?;
        checkpoint()?;
        let response = self
            .request(
                Method::PUT,
                Some(("If-Match", &tag)),
                Some(&self.bodies[1]),
                "update-current",
            )
            .await?;
        if !response.status().is_success() {
            return Err(condition_failure(
                "update-current",
                response.status().as_u16(),
                false,
            ));
        }
        let (body, next_tag) = self.read("read-updated").await?;
        if body != self.bodies[1] || tag == next_tag {
            return Err(condition_failure("read-updated", 200, false));
        }
        checkpoint()?;
        let response = self
            .request(
                Method::PUT,
                Some(("If-Match", &tag)),
                Some(&self.bodies[2]),
                "update-stale",
            )
            .await?;
        if response.status().as_u16() != 412 {
            return Err(condition_failure(
                "update-stale",
                response.status().as_u16(),
                false,
            ));
        }
        let (body, _) = self.read("verify-stale-rejected").await?;
        if body != self.bodies[1] {
            return Err(condition_failure("verify-stale-rejected", 200, false));
        }
        checkpoint()?;
        self.conditional_read(&tag, 200, Some(&self.bodies[1]), "read-stale")
            .await?;
        self.conditional_read(&next_tag, 304, None, "read-latest")
            .await?;
        checkpoint()
    }

    async fn conditional_read(
        &self,
        tag: &str,
        expected: u16,
        body: Option<&str>,
        step: &str,
    ) -> Result<(), CapabilityReport> {
        let response = self
            .request(Method::GET, Some(("If-None-Match", tag)), None, step)
            .await?;
        let status = response.status().as_u16();
        if status != expected {
            return Err(condition_failure(step, status, true));
        }
        let returned_tag = response_tag(&response);
        if expected == 200 && classify_etag(returned_tag.as_deref()) != EtagKind::Strong {
            return Err(CapabilityReport::issue(
                CapabilityReason::VersionUnavailable,
                step,
                Some(status),
                classify_etag(returned_tag.as_deref()),
            ));
        }
        if expected == 200 && returned_tag.as_deref() == Some(tag)
            || expected == 304
                && returned_tag
                    .as_deref()
                    .is_some_and(|returned| returned != tag)
        {
            return Err(condition_failure(step, status, true));
        }
        if let Some(body) = body
            && small_body(response, step).await? != body
        {
            return Err(condition_failure(step, status, true));
        }
        Ok(())
    }

    pub async fn cleanup(&self) -> Result<(), CapabilityReport> {
        let result = tokio::time::timeout(Duration::from_secs(35), async {
            let response = self
                .request(Method::GET, None, None, "cleanup-read")
                .await?;
            if response.status().as_u16() == 404 {
                return Ok(());
            }
            if response.status().as_u16() != 200 {
                return Err(failure("cleanup-read", response.status().as_u16()));
            }
            let tag = response_tag(&response);
            let body = small_body(response, "cleanup-read").await?;
            if !self.bodies.contains(&body) {
                return Err(failure("cleanup-ownership", 200));
            }
            let condition = tag
                .as_deref()
                .filter(|tag| classify_etag(Some(tag)) == EtagKind::Strong)
                .map(|tag| ("If-Match", tag));
            let response = self
                .request(Method::DELETE, condition, None, "cleanup-delete")
                .await?;
            if !response.status().is_success() && response.status().as_u16() != 404 {
                return Err(failure("cleanup-delete", response.status().as_u16()));
            }
            let response = self
                .request(Method::GET, None, None, "cleanup-verify")
                .await?;
            if response.status().as_u16() != 404 {
                return Err(failure("cleanup-verify", response.status().as_u16()));
            }
            Ok(())
        })
        .await;
        match result {
            Ok(Ok(())) => Ok(()),
            other => {
                let mut report = match other {
                    Ok(Err(report)) => report,
                    _ => failure("cleanup-timeout", 0),
                };
                report.state = CapabilityState::Incomplete;
                report.reason = Some(CapabilityReason::Cleanup);
                report.cleanup_file = Some(self.filename.clone());
                Err(report)
            }
        }
    }
}

fn condition_failure(step: &str, status: u16, read: bool) -> CapabilityReport {
    if status == 401
        || status == 403
        || (300..400).contains(&status) && status != 304
        || status >= 500
    {
        return failure(step, status);
    }
    CapabilityReport::issue(
        if read {
            CapabilityReason::ConditionalRead
        } else {
            CapabilityReason::ConditionalWrite
        },
        step,
        Some(status),
        EtagKind::Strong,
    )
}

pub async fn check_webdav(
    url: &str,
    username: String,
    password: String,
    cancel: ProbeCancellation,
) -> (CapabilityReport, Option<PendingProbe>) {
    check_webdav_with_deadline(url, username, password, cancel, Duration::from_secs(90)).await
}

async fn check_webdav_with_deadline(
    url: &str,
    username: String,
    password: String,
    cancel: ProbeCancellation,
    deadline: Duration,
) -> (CapabilityReport, Option<PendingProbe>) {
    let mut pending = None;
    let result = tokio::time::timeout(deadline, async {
        let url = Url::parse(url).map_err(|_| {
            CapabilityReport::issue(
                CapabilityReason::Configuration,
                "configuration",
                None,
                EtagKind::Missing,
            )
        })?;
        if !(url.scheme() == "https"
            || url.scheme() == "http"
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
            || url.path().ends_with('/')
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(CapabilityReport::issue(
                CapabilityReason::Configuration,
                "configuration",
                None,
                EtagKind::Missing,
            ));
        }
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(crate::webdav::WEBDAV_CONNECT_TIMEOUT)
            .timeout(crate::webdav::WEBDAV_REQUEST_TIMEOUT)
            .build()
            .map_err(|_| failure("client", 0))?;
        if cancel.is_cancelled() {
            return Err(CapabilityReport::issue(
                CapabilityReason::Cancelled,
                "cancel",
                None,
                EtagKind::Missing,
            ));
        }
        let response = client
            .get(url.clone())
            .basic_auth(&username, Some(&password))
            .send()
            .await
            .map_err(|_| {
                CapabilityReport::issue(
                    CapabilityReason::Network,
                    "read-resource",
                    None,
                    EtagKind::Missing,
                )
            })?;
        let status = response.status().as_u16();
        if status == 200 {
            let kind = classify_etag(response_tag(&response).as_deref());
            if kind != EtagKind::Strong {
                return Err(CapabilityReport::issue(
                    CapabilityReason::VersionUnavailable,
                    "read-resource",
                    Some(status),
                    kind,
                ));
            }
        } else if status != 404 {
            return Err(failure("read-resource", status));
        }
        drop(response);
        let filename = format!("miaominal-sync-probe-{}.json", uuid::Uuid::new_v4());
        let probe_url = url
            .join(&filename)
            .map_err(|_| failure("configuration", 0))?;
        let probe = PendingProbe {
            client,
            url: probe_url,
            username,
            password,
            bodies: std::array::from_fn(|_| format!("{{\"probe\":\"{}\"}}", uuid::Uuid::new_v4())),
            filename,
        };
        let response = probe.request(Method::GET, None, None, "check-path").await?;
        if response.status().as_u16() != 404 {
            return Err(failure("check-path", response.status().as_u16()));
        }
        // Record ownership candidates before PUT: its response may be lost.
        pending = Some(probe);
        pending.as_ref().unwrap().verify(&cancel).await
    })
    .await;
    let mut report = match result {
        Ok(Ok(())) => CapabilityReport::supported(),
        Ok(Err(report)) => report,
        Err(_) => CapabilityReport::issue(
            CapabilityReason::Network,
            "check-timeout",
            None,
            EtagKind::Missing,
        ),
    };
    if let Some(probe) = &pending {
        match probe.cleanup().await {
            Ok(()) => pending = None,
            Err(cleanup) => report = cleanup,
        }
    }
    if cancel.is_cancelled() && pending.is_none() {
        report = CapabilityReport::issue(
            CapabilityReason::Cancelled,
            "cancel",
            None,
            EtagKind::Missing,
        );
    }
    (report, pending)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Mutex, atomic::AtomicBool};

    #[derive(Clone, Copy, Debug, Default)]
    enum Behavior {
        #[default]
        Compatible,
        Weak,
        Missing,
        Invalid,
        WeakResource,
        MissingResourceTag,
        InvalidResourceTag,
        IgnoreMatch,
        RejectMatch,
        IgnoreNoneMatch,
        UnchangedTag,
        Always304,
        DenyCreate,
        Unauthorized,
        Redirect,
        MissingResource,
        LostCreateResponse,
        ForeignBody,
    }

    struct TestServer {
        url: String,
        requests: Arc<Mutex<Vec<(String, String, String)>>>,
        content: Arc<Mutex<Option<String>>>,
        deny_delete: Arc<AtomicBool>,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl TestServer {
        fn new(behavior: Behavior, cancel_on_create: Option<ProbeCancellation>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/sync.json", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let content = Arc::new(Mutex::new(None::<String>));
            let deny_delete = Arc::new(AtomicBool::new(false));
            let stop = Arc::new(AtomicBool::new(false));
            let (events, stored, denied, stopped) = (
                requests.clone(),
                content.clone(),
                deny_delete.clone(),
                stop.clone(),
            );
            let handle = std::thread::spawn(move || {
                let mut version = 1;
                while !stopped.load(Ordering::SeqCst) {
                    let (mut stream, _) = match listener.accept() {
                        Ok(stream) => stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        Err(e) => panic!("{e}"),
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut bytes = Vec::new();
                    let mut buffer = [0; 2048];
                    let end = loop {
                        let count = stream.read(&mut buffer).unwrap();
                        assert_ne!(count, 0);
                        bytes.extend_from_slice(&buffer[..count]);
                        if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                            let headers =
                                String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                            let length = headers
                                .lines()
                                .find_map(|line| line.strip_prefix("content-length:"))
                                .map_or(0, |v| v.trim().parse::<usize>().unwrap());
                            if bytes.len() >= end + 4 + length {
                                break end;
                            }
                        }
                    };
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let mut line = headers.lines().next().unwrap().split_whitespace();
                    let method = line.next().unwrap();
                    let path = line.next().unwrap();
                    let body = String::from_utf8(bytes[end + 4..].to_vec()).unwrap();
                    events
                        .lock()
                        .unwrap()
                        .push((method.into(), path.into(), body.clone()));
                    let status;
                    let mut response_body = String::new();
                    let tag = format!("\"v{version}\"");
                    let mut etag = Some(tag.clone());
                    if path == "/sync.json" {
                        assert_eq!(method, "get", "the actual sync resource must be read-only");
                        response_body = "private-real-config".into();
                        etag = match behavior {
                            Behavior::WeakResource => Some("W/\"weak\"".into()),
                            Behavior::MissingResourceTag => None,
                            Behavior::InvalidResourceTag => Some("invalid".into()),
                            _ => etag,
                        };
                        status = match behavior {
                            Behavior::Unauthorized => 401,
                            Behavior::Redirect => 302,
                            Behavior::MissingResource => 404,
                            _ => 200,
                        };
                    } else {
                        assert!(
                            path.starts_with("/miaominal-sync-probe-") && path.ends_with(".json")
                        );
                        let mut content = stored.lock().unwrap();
                        match method {
                            "get" => {
                                if let Some(value) = content.as_ref() {
                                    let conditional = headers
                                        .lines()
                                        .find_map(|l| l.strip_prefix("if-none-match: "));
                                    status = if matches!(behavior, Behavior::Always304)
                                        && conditional.is_some()
                                        || conditional == Some(tag.as_str())
                                            && !matches!(behavior, Behavior::IgnoreNoneMatch)
                                    {
                                        304
                                    } else {
                                        200
                                    };
                                    if status == 200 {
                                        response_body = value.clone();
                                    }
                                    etag = match behavior {
                                        Behavior::Weak => Some("W/\"weak\"".into()),
                                        Behavior::Missing => None,
                                        Behavior::Invalid => Some("unquoted".into()),
                                        _ => Some(tag),
                                    };
                                } else {
                                    status = 404;
                                }
                            }
                            "put" => {
                                let create = headers.contains("if-none-match: *");
                                let matched =
                                    headers.lines().find_map(|l| l.strip_prefix("if-match: "));
                                status = if create && matches!(behavior, Behavior::DenyCreate) {
                                    403
                                } else if create
                                    && content.is_some()
                                    && !matches!(behavior, Behavior::IgnoreNoneMatch)
                                    || matched.is_some()
                                        && matches!(behavior, Behavior::RejectMatch)
                                    || matched.is_some_and(|v| v != tag)
                                        && !matches!(behavior, Behavior::IgnoreMatch)
                                {
                                    412
                                } else {
                                    204
                                };
                                if status == 204 {
                                    *content = Some(if matches!(behavior, Behavior::ForeignBody) {
                                        "foreign-data".into()
                                    } else {
                                        body
                                    });
                                    if !matches!(behavior, Behavior::UnchangedTag) {
                                        version += 1;
                                    }
                                    if let Some(cancel) = &cancel_on_create {
                                        cancel.cancel();
                                    }
                                    if create && matches!(behavior, Behavior::LostCreateResponse) {
                                        continue;
                                    }
                                }
                                // PUT ETags are optional: the probe must read them with GET.
                                etag = None;
                            }
                            "delete" => {
                                if denied.load(Ordering::SeqCst) {
                                    status = 403;
                                } else {
                                    *content = None;
                                    status = 204;
                                }
                            }
                            other => panic!("unexpected method {other}"),
                        }
                    }
                    let tag_header = etag.map_or(String::new(), |v| format!("ETag: {v}\r\n"));
                    let redirect = if status == 302 {
                        "Location: http://127.0.0.1:1/private\r\n"
                    } else {
                        ""
                    };
                    write!(stream, "HTTP/1.1 {status} Test\r\n{tag_header}{redirect}Content-Length: {}\r\nConnection: close\r\n\r\n{response_body}", response_body.len()).ok();
                }
            });
            Self {
                url,
                requests,
                content,
                deny_delete,
                stop,
                handle: Some(handle),
            }
        }

        async fn check(
            &self,
            cancel: ProbeCancellation,
        ) -> (CapabilityReport, Option<PendingProbe>) {
            check_webdav(
                &self.url,
                "private-user".into(),
                "private-password".into(),
                cancel,
            )
            .await
        }
    }
    impl Drop for TestServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            self.handle.take().unwrap().join().unwrap();
        }
    }

    #[test]
    fn etags_are_classified_without_promoting_weak_tags() {
        for (tag, expected) in [
            (None, EtagKind::Missing),
            (Some("\"abc\""), EtagKind::Strong),
            (Some("\"\""), EtagKind::Strong),
            (Some("W/\"abc\""), EtagKind::Weak),
            (Some("abc"), EtagKind::Invalid),
            (Some("w/\"abc\""), EtagKind::Invalid),
            (Some("\"a b\""), EtagKind::Invalid),
            (Some("\"a\"b\""), EtagKind::Invalid),
        ] {
            assert_eq!(classify_etag(tag), expected, "{tag:?}");
        }
    }

    #[tokio::test]
    async fn compatible_service_checks_conditions_and_cleans_only_its_file() {
        for behavior in [Behavior::Compatible, Behavior::MissingResource] {
            let server = TestServer::new(behavior, None);
            let (report, pending) = server.check(ProbeCancellation::default()).await;
            assert_eq!(report.state, CapabilityState::Supported, "{report:?}");
            assert!(pending.is_none());
            assert!(server.content.lock().unwrap().is_none());
            assert_eq!(
                server
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(m, _, _)| m == "delete")
                    .count(),
                1
            );
            let diagnostic = report.diagnostics();
            for secret in [
                "private-user",
                "private-password",
                "private-real-config",
                &server.url,
            ] {
                assert!(!diagnostic.contains(secret));
            }
        }
    }

    #[tokio::test]
    async fn incompatible_services_are_rejected_and_temporary_data_is_cleaned() {
        for behavior in [
            Behavior::Weak,
            Behavior::Missing,
            Behavior::Invalid,
            Behavior::IgnoreMatch,
            Behavior::RejectMatch,
            Behavior::IgnoreNoneMatch,
            Behavior::UnchangedTag,
            Behavior::Always304,
        ] {
            let server = TestServer::new(behavior, None);
            let (report, pending) = server.check(ProbeCancellation::default()).await;
            assert_eq!(
                report.state,
                CapabilityState::Unsupported,
                "{behavior:?}: {report:?}"
            );
            assert!(pending.is_none(), "{behavior:?}");
            assert!(server.content.lock().unwrap().is_none());
        }
    }

    #[tokio::test]
    async fn permissions_authentication_and_redirects_are_incomplete_not_unsupported() {
        for (behavior, reason) in [
            (Behavior::DenyCreate, CapabilityReason::Permission),
            (Behavior::Unauthorized, CapabilityReason::Authentication),
            (Behavior::Redirect, CapabilityReason::Redirect),
        ] {
            let server = TestServer::new(behavior, None);
            let (report, pending) = server.check(ProbeCancellation::default()).await;
            assert_eq!(report.state, CapabilityState::Incomplete, "{report:?}");
            assert_eq!(report.reason, Some(reason));
            assert!(pending.is_none());
        }
    }

    #[tokio::test]
    async fn actual_resource_without_strong_version_never_creates_a_probe() {
        for behavior in [
            Behavior::WeakResource,
            Behavior::MissingResourceTag,
            Behavior::InvalidResourceTag,
        ] {
            let server = TestServer::new(behavior, None);
            let (report, pending) = server.check(ProbeCancellation::default()).await;
            assert_eq!(report.state, CapabilityState::Unsupported);
            assert_eq!(report.step, "read-resource");
            assert!(pending.is_none());
            let requests = server.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].0, "get");
            assert_eq!(requests[0].1, "/sync.json");
        }
    }

    #[tokio::test]
    async fn total_deadline_stops_a_stalled_check_without_creating_a_resource() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/sync.json", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(200));
        });
        let (report, pending) = check_webdav_with_deadline(
            &url,
            "user".into(),
            "password".into(),
            ProbeCancellation::default(),
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(report.state, CapabilityState::Incomplete);
        assert_eq!(report.reason, Some(CapabilityReason::Network));
        assert_eq!(report.step, "check-timeout");
        assert!(pending.is_none());
        server.join().unwrap();
    }

    #[tokio::test]
    async fn cancellation_after_create_still_cleans_the_file() {
        let cancel = ProbeCancellation::default();
        let server = TestServer::new(Behavior::Compatible, Some(cancel.clone()));
        let (report, pending) = server.check(cancel).await;
        assert_eq!(report.reason, Some(CapabilityReason::Cancelled));
        assert!(pending.is_none());
        assert!(server.content.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn lost_create_response_checks_ownership_before_cleanup() {
        let server = TestServer::new(Behavior::LostCreateResponse, None);
        let (report, pending) = server.check(ProbeCancellation::default()).await;
        assert_eq!(report.reason, Some(CapabilityReason::Network));
        assert!(pending.is_none());
        assert!(server.content.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn failed_cleanup_can_retry_the_original_resource() {
        let server = TestServer::new(Behavior::Compatible, None);
        server.deny_delete.store(true, Ordering::SeqCst);
        let (report, pending) = server.check(ProbeCancellation::default()).await;
        assert_eq!(report.reason, Some(CapabilityReason::Cleanup));
        assert!(
            report
                .cleanup_file
                .as_ref()
                .unwrap()
                .starts_with("miaominal-sync-probe-")
        );
        server.deny_delete.store(false, Ordering::SeqCst);
        pending.unwrap().cleanup().await.unwrap();
        assert!(server.content.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn cleanup_never_deletes_unrecognized_content() {
        let server = TestServer::new(Behavior::ForeignBody, None);
        let (report, pending) = server.check(ProbeCancellation::default()).await;
        assert_eq!(report.reason, Some(CapabilityReason::Cleanup));
        assert!(pending.is_some());
        assert!(
            !server
                .requests
                .lock()
                .unwrap()
                .iter()
                .any(|(method, _, _)| method == "delete")
        );
    }
}
