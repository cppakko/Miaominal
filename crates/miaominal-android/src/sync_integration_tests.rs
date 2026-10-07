//! P6 integration tests for sync and configuration import.
//!
//! Every test runs against a local in-process HTTP server that speaks the
//! subset of WebDAV or the GitHub Gist API the engine uses. Nothing here
//! touches the public network, so the suite is deterministic and repeatable.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::backend::{Backend, BackendConfig};
use crate::dto::CredentialAction;
use crate::error::BackendError;
use crate::platform::PlatformHost;
use crate::sync_dto::{
    ImportSourceDto, SyncConfigUpdate, SyncCredentialKind, SyncPhaseDto, SyncProviderDto,
    SyncResolution, SyncStatusDto,
};
use crate::test_support::{
    business_root, cache_root, clear_test_root, initialize_test_runtime, shared_test_lock, test_host,
};

const SYNC_PASSPHRASE: &str = "p6-sync-passphrase";
const WEBDAV_USER: &str = "sync-user";
const WEBDAV_PASSWORD: &str = "sync-password";

// ---------------------------------------------------------------------------
// Fake WebDAV server
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebDavMode {
    /// Honours ETags and conditional requests (a capable server).
    Conforming,
    /// Ignores conditional headers: accepts every PUT and never sends 412.
    IgnoreConditions,
    /// Rejects every request with 401.
    Unauthorized,
}

#[derive(Default)]
struct WebDavResources {
    files: Mutex<HashMap<String, (String, u64)>>,
    generation: AtomicU64,
}

struct FakeWebDav {
    address: SocketAddr,
    resources: Arc<WebDavResources>,
    /// Stall GETs/PUTs on the sync resource until released (cancel tests).
    stall_sync_get: Arc<AtomicBool>,
    stall_sync_put: Arc<AtomicBool>,
    stall_release: Arc<tokio::sync::Notify>,
    observed_get: Arc<AtomicBool>,
    observed_put: Arc<AtomicBool>,
    /// Reject every request with 401 (simulates an offline/failed push).
    force_unauthorized: Arc<AtomicBool>,
}

impl FakeWebDav {
    async fn start(mode: WebDavMode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("fake WebDAV listener should bind");
        let address = listener.local_addr().expect("fake WebDAV address");
        let resources = Arc::new(WebDavResources::default());
        let mode = Arc::new(Mutex::new(mode));
        let stall_sync_get = Arc::new(AtomicBool::new(false));
        let stall_sync_put = Arc::new(AtomicBool::new(false));
        let stall_release = Arc::new(tokio::sync::Notify::new());
        let observed_get = Arc::new(AtomicBool::new(false));
        let observed_put = Arc::new(AtomicBool::new(false));
        let force_unauthorized = Arc::new(AtomicBool::new(false));
        let server = Self {
            address,
            resources: Arc::clone(&resources),
            stall_sync_get: Arc::clone(&stall_sync_get),
            stall_sync_put: Arc::clone(&stall_sync_put),
            stall_release: Arc::clone(&stall_release),
            observed_get: Arc::clone(&observed_get),
            observed_put: Arc::clone(&observed_put),
            force_unauthorized: Arc::clone(&force_unauthorized),
        };
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let resources = Arc::clone(&resources);
                let mode = Arc::clone(&mode);
                let stall = Arc::clone(&stall_sync_get);
                let stall_put = Arc::clone(&stall_sync_put);
                let release = Arc::clone(&stall_release);
                let observed = Arc::clone(&observed_get);
                let observed_put = Arc::clone(&observed_put);
                let unauthorized = Arc::clone(&force_unauthorized);
                tokio::spawn(async move {
                    let _ = serve_webdav(
                        stream,
                        resources,
                        mode,
                        stall,
                        stall_put,
                        release,
                        observed,
                        observed_put,
                        unauthorized,
                    )
                    .await;
                });
            }
        });
        server
    }

    fn url(&self) -> String {
        format!("http://{}/sync.json", self.address)
    }

    fn body(&self) -> Option<String> {
        self.resources
            .files
            .lock()
            .expect("webdav resources")
            .get("/sync.json")
            .map(|(body, _)| body.clone())
    }

    fn set_body(&self, body: &str) {
        let generation = self.resources.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.resources
            .files
            .lock()
            .expect("webdav resources")
            .insert("/sync.json".to_string(), (body.to_string(), generation));
    }

    fn set_stall_sync_get(&self, stalled: bool) {
        self.stall_sync_get.store(stalled, Ordering::SeqCst);
    }

    fn set_stall_sync_put(&self, stalled: bool) {
        self.stall_sync_put.store(stalled, Ordering::SeqCst);
    }

    /// Release any stalled request so it can answer normally.
    fn release_stall(&self) {
        self.stall_release.notify_waiters();
    }

    /// Make every subsequent request fail with 401 until disabled.
    fn set_force_unauthorized(&self, unauthorized: bool) {
        self.force_unauthorized.store(unauthorized, Ordering::SeqCst);
    }

    /// Forget earlier observations so a test can wait for its own request.
    fn clear_observed_get(&self) {
        self.observed_get.store(false, Ordering::SeqCst);
    }

    fn clear_observed_put(&self) {
        self.observed_put.store(false, Ordering::SeqCst);
    }
}

#[allow(clippy::too_many_arguments)]
async fn serve_webdav(
    mut stream: TcpStream,
    resources: Arc<WebDavResources>,
    mode: Arc<Mutex<WebDavMode>>,
    stall_sync_get: Arc<AtomicBool>,
    stall_sync_put: Arc<AtomicBool>,
    stall_release: Arc<tokio::sync::Notify>,
    observed_get: Arc<AtomicBool>,
    observed_put: Arc<AtomicBool>,
    force_unauthorized: Arc<AtomicBool>,
) -> std::io::Result<()> {
    let request = read_http_request(&mut stream).await?;
    let current_mode = *mode.lock().expect("webdav mode");
    if current_mode == WebDavMode::Unauthorized || force_unauthorized.load(Ordering::SeqCst) {
        return write_response(&mut stream, 401, &[], "", false).await;
    }
    let method = request.method.as_str();
    let path = request.path.clone();
    if method == "GET" && path == "/sync.json" {
        observed_get.store(true, Ordering::SeqCst);
        if stall_sync_get.load(Ordering::SeqCst) {
            tokio::select! {
                () = stall_release.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(30)) => {}
            }
        }
    }
    if method == "PUT" && path == "/sync.json" {
        observed_put.store(true, Ordering::SeqCst);
        if stall_sync_put.load(Ordering::SeqCst) {
            tokio::select! {
                () = stall_release.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(30)) => {}
            }
        }
    }
    let current = resources
        .files
        .lock()
        .expect("webdav resources")
        .get(&path)
        .cloned();
    let ignore_conditions = current_mode == WebDavMode::IgnoreConditions;

    match method {
        "GET" => match current {
            None => write_response(&mut stream, 404, &[], "", false).await,
            Some((body, generation)) => {
                let tag = format!("\"gen-{generation}\"");
                let not_modified = !ignore_conditions
                    && request
                        .header("if-none-match")
                        .is_some_and(|value| value == "*" || value == tag);
                if not_modified {
                    write_response(&mut stream, 304, &[("ETag", tag.as_str())], "", true).await
                } else {
                    write_response(
                        &mut stream,
                        200,
                        &[("ETag", tag.as_str())],
                        &body,
                        true,
                    )
                    .await
                }
            }
        },
        "PUT" => {
            if !ignore_conditions {
                if let Some(expected) = request.header("if-match") {
                    let matches = current
                        .as_ref()
                        .is_some_and(|(_, generation)| expected == format!("\"gen-{generation}\""));
                    if !matches {
                        return write_response(&mut stream, 412, &[], "", true).await;
                    }
                }
                if request.header("if-none-match").is_some_and(|value| value == "*")
                    && current.is_some()
                {
                    return write_response(&mut stream, 412, &[], "", true).await;
                }
            }
            let generation = resources.generation.fetch_add(1, Ordering::SeqCst) + 1;
            let status = if current.is_some() { 204 } else { 201 };
            resources
                .files
                .lock()
                .expect("webdav resources")
                .insert(path, (request.body.clone(), generation));
            let tag = format!("\"gen-{generation}\"");
            write_response(&mut stream, status, &[("ETag", tag.as_str())], "", false).await
        }
        "DELETE" => {
            if let Some(expected) = request.header("if-match")
                && !ignore_conditions
            {
                let matches = current
                    .as_ref()
                    .is_some_and(|(_, generation)| expected == format!("\"gen-{generation}\""));
                if !matches {
                    return write_response(&mut stream, 412, &[], "", true).await;
                }
            }
            if current.is_none() {
                return write_response(&mut stream, 404, &[], "", true).await;
            }
            resources
                .files
                .lock()
                .expect("webdav resources")
                .remove(&path);
            write_response(&mut stream, 204, &[], "", true).await
        }
        _ => write_response(&mut stream, 405, &[], "", true).await,
    }
}

// ---------------------------------------------------------------------------
// Fake Gist API server
// ---------------------------------------------------------------------------

#[derive(Default)]
struct GistState {
    inner: Mutex<Option<GistDocument>>,
    next_id: AtomicU64,
}

struct GistDocument {
    id: String,
    content: String,
    generation: u64,
}

struct FakeGist {
    address: SocketAddr,
    state: Arc<GistState>,
}

impl FakeGist {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("fake gist listener should bind");
        let address = listener.local_addr().expect("fake gist address");
        let state = Arc::new(GistState::default());
        let server_state = Arc::clone(&state);
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let state = Arc::clone(&server_state);
                tokio::spawn(async move {
                    let _ = serve_gist(stream, state).await;
                });
            }
        });
        Self { address, state }
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }

    fn content(&self) -> Option<String> {
        self.state
            .inner
            .lock()
            .expect("gist state")
            .as_ref()
            .map(|document| document.content.clone())
    }

    fn set_content(&self, content: &str) {
        let mut inner = self.state.inner.lock().expect("gist state");
        let document = inner.get_or_insert_with(|| GistDocument {
            id: format!(
                "gist-{}",
                self.state.next_id.fetch_add(1, Ordering::SeqCst) + 1
            ),
            content: content.to_string(),
            generation: 0,
        });
        document.content = content.to_string();
        document.generation += 1;
    }
}

async fn serve_gist(mut stream: TcpStream, state: Arc<GistState>) -> std::io::Result<()> {
    let request = read_http_request(&mut stream).await?;
    if request.header("authorization").as_deref() != Some("Bearer test-token") {
        return write_response(&mut stream, 401, &[], "", true).await;
    }
    let method = request.method.as_str();
    if method == "POST" && request.path == "/gists" {
        let body: serde_json::Value =
            serde_json::from_str(&request.body).unwrap_or(serde_json::Value::Null);
        let content = body["files"]["miaominal_sync.json"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let id = format!(
            "gist-{}",
            state.next_id.fetch_add(1, Ordering::SeqCst) + 1
        );
        *state.inner.lock().expect("gist state") = Some(GistDocument {
            id: id.clone(),
            content,
            generation: 1,
        });
        let body = format!("{{\"id\":\"{id}\"}}");
        return write_response(&mut stream, 201, &[("ETag", "\"gen-1\"")], &body, true).await;
    }

    // Build the response while holding the state lock, then release it before
    // any await (the guard is not `Send`).
    let response: Option<(u16, Vec<(String, String)>, String)> = {
        let mut inner = state.inner.lock().expect("gist state");
        match inner.as_mut() {
            None => None,
            Some(document) if !request.path.ends_with(&document.id) => None,
            Some(document) => {
                let tag = format!("\"gen-{}\"", document.generation);
                match method {
                    "GET" => {
                        if request.header("if-none-match").as_deref() == Some(tag.as_str()) {
                            Some((304, vec![("ETag".into(), tag)], String::new()))
                        } else {
                            let body = serde_json::json!({
                                "files": { "miaominal_sync.json": { "content": document.content } }
                            })
                            .to_string();
                            Some((200, vec![("ETag".into(), tag)], body))
                        }
                    }
                    "PATCH" => {
                        if let Some(expected) = request.header("if-match")
                            && expected != tag
                        {
                            Some((412, Vec::new(), String::new()))
                        } else {
                            let body: serde_json::Value =
                                serde_json::from_str(&request.body).unwrap_or(serde_json::Value::Null);
                            if let Some(content) =
                                body["files"]["miaominal_sync.json"]["content"].as_str()
                            {
                                document.content = content.to_string();
                            }
                            document.generation += 1;
                            let next_tag = format!("\"gen-{}\"", document.generation);
                            let response = format!("{{\"id\":\"{}\"}}", document.id);
                            Some((200, vec![("ETag".into(), next_tag)], response))
                        }
                    }
                    _ => Some((405, Vec::new(), String::new())),
                }
            }
        }
    };

    match response {
        Some((status, headers, body)) => {
            let header_refs = headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str()))
                .collect::<Vec<_>>();
            write_response(&mut stream, status, &header_refs, &body, true).await
        }
        None => write_response(&mut stream, 404, &[], "", true).await,
    }
}

// ---------------------------------------------------------------------------
// Minimal HTTP/1.1 plumbing
// ---------------------------------------------------------------------------

struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

impl HttpRequest {
    fn header(&self, name: &str) -> Option<String> {
        self.headers.get(name).map(|value| value.to_string())
    }
}

async fn read_http_request(stream: &mut TcpStream) -> std::io::Result<HttpRequest> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "request ended before headers",
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    let path = target.split('?').next().unwrap_or_default().to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);
    Ok(HttpRequest {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).to_string(),
    })
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    headers: &[(&str, &str)],
    body: &str,
    with_length: bool,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        304 => "Not Modified",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        412 => "Precondition Failed",
        _ => "Error",
    };
    let mut response = format!("HTTP/1.1 {status} {reason}\r\nConnection: close\r\n");
    for (name, value) in headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    if with_length {
        response.push_str(&format!("Content-Length: {}\r\n", body.len()));
    } else {
        response.push_str("Content-Length: 0\r\n");
    }
    response.push_str("\r\n");
    response.push_str(body);
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn test_backend() -> Arc<Backend> {
    initialize_test_runtime();
    Backend::new(
        test_host(),
        BackendConfig {
            data_dir: business_root().to_string_lossy().into_owned(),
            cache_dir: cache_root().to_string_lossy().into_owned(),
        },
    )
}

async fn configure_webdav(backend: &Backend, url: &str) {
    backend
        .update_sync_config(SyncConfigUpdate {
            provider: Some(SyncProviderDto::WebDav),
            webdav_url: Some(url.to_string()),
            webdav_username: Some(WEBDAV_USER.to_string()),
            ..SyncConfigUpdate::default()
        })
        .await
        .expect("webdav config should save");
    backend
        .set_sync_credential(
            SyncCredentialKind::WebdavPassword,
            CredentialAction::Set {
                value: WEBDAV_PASSWORD.to_string(),
            },
        )
        .await
        .expect("webdav password should save");
    backend
        .set_sync_credential(
            SyncCredentialKind::Passphrase,
            CredentialAction::Set {
                value: SYNC_PASSPHRASE.to_string(),
            },
        )
        .await
        .expect("sync passphrase should save");
}

async fn configure_gist(backend: &Backend, api_base: &str) {
    backend
        .update_sync_config(SyncConfigUpdate {
            provider: Some(SyncProviderDto::GithubGist),
            gist_id: Some(String::new()),
            ..SyncConfigUpdate::default()
        })
        .await
        .expect("gist config should save");
    backend
        .set_sync_credential(
            SyncCredentialKind::GithubToken,
            CredentialAction::Set {
                value: "test-token".to_string(),
            },
        )
        .await
        .expect("gist token should save");
    backend
        .set_sync_credential(
            SyncCredentialKind::Passphrase,
            CredentialAction::Set {
                value: SYNC_PASSPHRASE.to_string(),
            },
        )
        .await
        .expect("sync passphrase should save");
    append_gist_api_base(api_base);
}

/// Test-only: the production UI never writes this hidden field.
fn append_gist_api_base(base: &str) {
    let path = miaominal_paths::config_file("sync_config.toml")
        .expect("sync config path should resolve");
    let mut content = std::fs::read_to_string(&path).unwrap_or_default();
    if !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&format!("gist_api_base = \"{base}\"\n"));
    std::fs::write(&path, content).expect("sync config should be writable");
}

async fn save_demo_host(backend: &Backend, name: &str, host: &str) {
    backend
        .save_host(crate::dto::HostUpdate {
            id: "host-1".to_string(),
            name: Some(name.to_string()),
            group: Some("Group".to_string()),
            tags: Some(vec!["tag-a".to_string()]),
            kind: None,
            host: Some(host.to_string()),
            port: Some(2222),
            username: Some("akko".to_string()),
            auth_method: None,
            private_key_path: None,
            managed_key_id: None,
            certificate_path: None,
            startup_command: Some("echo ready".to_string()),
            environment_variables: None,
            shell_type: None,
            is_favorite: None,
            last_connected_at_ms: None,
        })
        .await
        .expect("host should save");
}

fn expect_sync_failure(error: BackendError, needle: &str) {
    let text = format!("{error:?}");
    assert!(text.contains(needle), "expected {needle}, got {text}");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn capability_check_accepts_a_conforming_webdav_server() {
    let _guard = shared_test_lock();
    clear_test_root();
    let server = FakeWebDav::start(WebDavMode::Conforming).await;
    let backend = test_backend();
    backend.initialize().await.expect("backend should initialize");
    configure_webdav(&backend, &server.url()).await;

    let result = backend
        .sync_check_capability()
        .await
        .expect("a conforming server should pass the probe");

    assert!(matches!(
        result.status,
        SyncStatusDto::Idle | SyncStatusDto::Failed { .. }
    ));
    let snapshot = backend.sync_snapshot().await.expect("snapshot");
    assert_eq!(
        snapshot.capability.state,
        crate::sync_dto::SyncCapabilityStateDto::Supported
    );
    assert!(snapshot.capability.cleanup_pending == false);
    backend.shutdown();
}

#[tokio::test]
async fn capability_check_reports_a_server_that_ignores_conditions() {
    let _guard = shared_test_lock();
    clear_test_root();
    let server = FakeWebDav::start(WebDavMode::IgnoreConditions).await;
    let backend = test_backend();
    backend.initialize().await.expect("backend should initialize");
    configure_webdav(&backend, &server.url()).await;

    let result = backend
        .sync_check_capability()
        .await
        .expect("the probe returns a report instead of failing");

    assert!(matches!(result.status, SyncStatusDto::Failed { .. }));
    let snapshot = backend.sync_snapshot().await.expect("snapshot");
    assert_ne!(
        snapshot.capability.state,
        crate::sync_dto::SyncCapabilityStateDto::Supported
    );
    backend.shutdown();
}

#[tokio::test]
async fn wrong_webdav_password_is_reported_as_authentication() {
    let _guard = shared_test_lock();
    clear_test_root();
    let server = FakeWebDav::start(WebDavMode::Unauthorized).await;
    let backend = test_backend();
    backend.initialize().await.expect("backend should initialize");
    configure_webdav(&backend, &server.url()).await;

    let error = backend
        .sync_pull(true)
        .await
        .expect_err("401 must not be treated as an empty remote");
    expect_sync_failure(error, "Authentication");
    backend.shutdown();
}

#[tokio::test]
async fn manual_push_binds_the_remote_and_pull_roundtrips_locally() {
    let _guard = shared_test_lock();
    clear_test_root();
    let server = FakeWebDav::start(WebDavMode::Conforming).await;
    let backend = test_backend();
    backend.initialize().await.expect("backend should initialize");
    configure_webdav(&backend, &server.url()).await;
    save_demo_host(&backend, "Demo", "demo.example.com").await;

    // First push creates the remote resource (If-None-Match: *).
    backend
        .sync_push(true)
        .await
        .expect("manual push should bind the remote");
    let body = server.body().expect("the remote must hold the payload");
    assert!(body.contains("encrypted_payload"));

    // Local edits after the push are preserved by a fresh pull (up to date).
    let result = backend
        .sync_pull(true)
        .await
        .expect("pull should succeed");
    assert!(matches!(
        result.status,
        SyncStatusDto::UpToDate { .. } | SyncStatusDto::Pulled { .. }
    ));
    let hosts = backend.hosts_snapshot().await.expect("hosts snapshot");
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].name, "Demo");
    backend.shutdown();
}

#[tokio::test]
async fn a_remote_change_after_the_push_is_pulled_on_the_next_pass() {
    let _guard = shared_test_lock();
    clear_test_root();
    let server = FakeWebDav::start(WebDavMode::Conforming).await;
    let backend = test_backend();
    backend.initialize().await.expect("backend should initialize");
    configure_webdav(&backend, &server.url()).await;
    save_demo_host(&backend, "Demo", "demo.example.com").await;
    backend.sync_push(true).await.expect("push should bind");

    // Another device replaces the remote payload with content encrypted under
    // a different passphrase: the pull must report an authentication failure
    // instead of overwriting anything.
    let wrong_passphrase_payload =
        build_remote_payload_with_passphrase("Other Device", "other.example.com", "other-passphrase");
    server.set_body(&wrong_passphrase_payload);

    let error = backend
        .sync_pull(false)
        .await
        .expect_err("a payload encrypted with another passphrase must be reported");
    expect_sync_failure(error, "Authentication");
    backend.shutdown();
}

#[tokio::test]
async fn diverged_content_requires_explicit_resolution() {
    let _guard = shared_test_lock();
    clear_test_root();
    let server = FakeWebDav::start(WebDavMode::Conforming).await;
    let backend = test_backend();
    backend.initialize().await.expect("backend should initialize");
    configure_webdav(&backend, &server.url()).await;
    save_demo_host(&backend, "Demo", "demo.example.com").await;
    backend.sync_push(true).await.expect("first push should bind");
    backend
        .sync_set_auto_sync(true)
        .await
        .expect("automatic sync should enable against a capable server");

    // This device edits locally...
    save_demo_host(&backend, "LocalEdit", "demo.example.com").await;
    // ...while another device replaces the remote content.
    let remote = build_remote_payload_with_host("RemoteEdit", "remote.example.com");
    server.set_body(&remote);

    let result = backend
        .sync_auto_check(false)
        .await
        .expect("the auto pass should report a conflict");
    assert!(
        matches!(result.status, SyncStatusDto::PullRequired { .. }),
        "expected a conflict, got {:?}",
        result.status
    );
    let snapshot = backend.sync_snapshot().await.expect("snapshot");
    assert!(snapshot.pending_conflict.is_some());
    assert_eq!(snapshot.phase, SyncPhaseDto::Conflict);

    // Keep remote: the local edit is replaced by the remote content.
    backend
        .sync_resolve_conflict(SyncResolution::KeepRemote)
        .await
        .expect("keep-remote should apply the remote payload");
    let hosts = backend.hosts_snapshot().await.expect("hosts snapshot");
    assert!(
        hosts.iter().any(|host| host.name == "RemoteEdit"),
        "remote content should win: {hosts:?}"
    );
    backend.shutdown();
}

#[tokio::test]
async fn repeated_auto_checks_inside_the_cooldown_do_not_deadlock() {
    let _guard = shared_test_lock();
    clear_test_root();
    let backend = test_backend();
    backend.initialize().await.expect("backend should initialize");
    configure_gist(&backend, "http://127.0.0.1:1").await;
    backend
        .update_sync_config(SyncConfigUpdate {
            auto_sync_enabled: Some(true),
            ..SyncConfigUpdate::default()
        })
        .await
        .expect("automatic sync should be recorded");

    // First pass records the check timestamp (the remote is not bound yet, so
    // it stops at `RemoteBindingRequired` without any network access).
    let first = tokio::time::timeout(Duration::from_secs(10), backend.sync_auto_check(false))
        .await
        .expect("the first check should not hang");
    assert!(first.is_ok(), "the first check should complete");

    // A second check inside the cooldown must return promptly. This used to
    // deadlock: the cooldown branch held the state lock and then re-locked it
    // while building the result, permanently blocking the operation gate.
    let started = std::time::Instant::now();
    let second = tokio::time::timeout(Duration::from_secs(5), backend.sync_auto_check(false))
        .await
        .expect("a cooldown check must not deadlock");
    assert!(second.is_ok(), "the cooldown branch returns the last state");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the cooldown path must be immediate, took {:?}",
        started.elapsed()
    );
    backend.shutdown();
}

#[tokio::test]
async fn conflict_resolution_uses_the_current_vault_for_push_and_pull() {
    let _guard = shared_test_lock();
    clear_test_root();
    let server = FakeWebDav::start(WebDavMode::Conforming).await;
    let host = test_host();
    let backend = Backend::new(
        host.clone(),
        BackendConfig {
            data_dir: business_root().to_string_lossy().into_owned(),
            cache_dir: cache_root().to_string_lossy().into_owned(),
        },
    );
    backend.initialize().await.unwrap();
    configure_webdav(&backend, &server.url()).await;
    save_demo_host(&backend, "Demo", "demo.example.com").await;
    backend
        .set_host_credential(
            "host-1".to_string(),
            crate::dto::CredentialKindDto::Password,
            CredentialAction::Set {
                value: "must-survive".to_string(),
            },
        )
        .await
        .unwrap();
    backend.sync_push(true).await.unwrap();
    miaominal_secrets::set_vault_test_parameters();
    backend
        .enable_vault("vault-passphrase".to_string())
        .await
        .unwrap();

    // The shared service still held the platform backend before this call.
    let pushed = backend
        .sync_resolve_conflict(SyncResolution::KeepLocal)
        .await
        .unwrap();
    assert!(matches!(pushed.status, SyncStatusDto::Pushed { .. }));
    let payload = miaominal_sync::payload::parse_remote_payload(&server.body().unwrap()).unwrap();
    let plaintext =
        miaominal_sync::payload::decrypt_remote_payload(&payload, SYNC_PASSPHRASE).unwrap();
    assert!(plaintext.secrets.profile_secrets.iter().any(|secret| {
        secret.id == "host-1" && secret.password.as_deref() == Some("must-survive")
    }));

    // Unlock creates a fresh vault backend and revokes the previous one. A
    // force pull must write to that new backend, not the service's old clone.
    backend
        .set_host_credential(
            "host-1".to_string(),
            crate::dto::CredentialKindDto::Password,
            CredentialAction::Clear,
        )
        .await
        .unwrap();
    backend.lock_vault().await.unwrap();
    backend
        .unlock_vault("vault-passphrase".to_string())
        .await
        .unwrap();
    let pulled = backend
        .sync_resolve_conflict(SyncResolution::KeepRemote)
        .await
        .unwrap();
    assert!(matches!(pulled.status, SyncStatusDto::Pulled { .. }));
    backend.disable_vault().await.unwrap();
    assert_eq!(host.stored("host-1:password").as_deref(), Some("must-survive"));
    backend.shutdown();
}

#[tokio::test]
async fn a_local_change_that_a_failed_push_missed_is_sent_on_the_next_check() {
    let _guard = shared_test_lock();
    clear_test_root();
    let server = FakeWebDav::start(WebDavMode::Conforming).await;
    let backend = test_backend();
    backend.initialize().await.expect("backend should initialize");
    configure_webdav(&backend, &server.url()).await;
    save_demo_host(&backend, "Before Offline", "demo.example.com").await;
    backend.sync_push(true).await.expect("first push should bind");
    let bound_body = server.body().expect("remote body");

    // The user edits while offline and the debounced push fails.
    save_demo_host(&backend, "Offline Edit", "demo.example.com").await;
    server.set_force_unauthorized(true);
    assert!(
        backend.sync_push(false).await.is_err(),
        "the push must fail while the remote rejects requests"
    );
    server.set_force_unauthorized(false);

    // Enable automatic sync and run a check: the remote did not change since
    // the last successful push, so the check must notice the local edit.
    backend
        .update_sync_config(SyncConfigUpdate {
            auto_sync_enabled: Some(true),
            ..SyncConfigUpdate::default()
        })
        .await
        .expect("automatic sync should be recorded");
    let result = tokio::time::timeout(Duration::from_secs(20), backend.sync_auto_check(false))
        .await
        .expect("the auto check should finish")
        .expect("the auto check should recover the missed upload");
    assert!(
        matches!(result.status, SyncStatusDto::Pushed { .. }),
        "expected the missed local change to be pushed, got {:?}",
        result.status
    );

    let body = server.body().expect("remote body after recovery");
    assert_ne!(body, bound_body, "the remote must have been updated");
    let payload = miaominal_sync::payload::parse_remote_payload(&body).expect("payload should parse");
    let plaintext =
        miaominal_sync::payload::decrypt_remote_payload(&payload, SYNC_PASSPHRASE)
            .expect("payload should decrypt");
    assert!(
        plaintext
            .sessions
            .iter()
            .any(|session| session.name == "Offline Edit"),
        "the offline edit must reach the remote"
    );
    backend.shutdown();
}

#[tokio::test]
async fn failed_and_cancelled_checks_leave_no_running_phase() {
    let _guard = shared_test_lock();
    clear_test_root();

    // A failing automatic check must return to Idle and record the failure.
    let server = FakeWebDav::start(WebDavMode::Conforming).await;
    let backend = test_backend();
    backend.initialize().await.expect("backend should initialize");
    configure_webdav(&backend, &server.url()).await;
    backend
        .sync_set_auto_sync(true)
        .await
        .expect("automatic sync should enable against a capable server");
    // The remote now rejects every request: the check must fail cleanly.
    server.set_force_unauthorized(true);
    let _ = tokio::time::timeout(Duration::from_secs(10), backend.sync_auto_check(false)).await;
    let snapshot = backend.sync_snapshot().await.expect("snapshot");
    assert_eq!(
        snapshot.phase,
        SyncPhaseDto::Idle,
        "a failed check must not stay in a running phase"
    );
    assert_eq!(
        snapshot.last_failure.as_ref().map(|failure| failure.kind),
        Some(crate::sync_dto::SyncFailureKindDto::Authentication),
        "the 401 must be recorded as a typed failure"
    );
    server.set_force_unauthorized(false);
    backend.shutdown();

    // Cancelling an in-flight capability probe must also return to Idle.
    let server = FakeWebDav::start(WebDavMode::Conforming).await;
    let backend = test_backend();
    backend.initialize().await.expect("backend should initialize");
    configure_webdav(&backend, &server.url()).await;
    server.set_stall_sync_get(true);
    server.clear_observed_get();
    let worker = Arc::clone(&backend);
    let task = crate::runtime::runtime().spawn(async move { worker.sync_check_capability().await });
    for _ in 0..250 {
        if server.observed_get.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        server.observed_get.load(Ordering::SeqCst),
        "the probe should reach the server"
    );
    backend.sync_cancel();
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("cancel must release the probe")
        .expect("task should not panic");
    assert!(matches!(result, Err(BackendError::Cancelled)));
    let snapshot = backend.sync_snapshot().await.expect("snapshot");
    assert_eq!(
        snapshot.phase,
        SyncPhaseDto::Idle,
        "a cancelled probe must not stay in a running phase"
    );
    assert_eq!(snapshot.status, SyncStatusDto::Cancelled);
    server.set_stall_sync_get(false);
    backend.shutdown();
}

/// Builds a payload as another device would, through the shared payload
/// builder, so the engine can decrypt and classify it by revision.
fn build_remote_payload_with_host(name: &str, host: &str) -> String {
    build_remote_payload_with_passphrase(name, host, SYNC_PASSPHRASE)
}

fn build_remote_payload_with_passphrase(name: &str, host: &str, passphrase: &str) -> String {
    let mut profile = miaominal_core::profile::SessionProfile::blank("session-remote", 1);
    profile.name = name.to_string();
    profile.host = host.to_string();
    profile.username = "remote".to_string();
    let plaintext = miaominal_sync::SyncPlaintextPayload {
        sessions: vec![profile],
        proxies: Vec::new(),
        snippets: Vec::new(),
        managed_keys: Vec::new(),
        settings: miaominal_settings::SyncedSettings::from(
            &miaominal_settings::AppSettings::default_for_system(),
        ),
        secrets: miaominal_sync::PlaintextSecrets::default(),
    };
    let payload =
        miaominal_sync::payload::build_payload("remote-device", None, &plaintext, passphrase)
            .expect("remote payload should build");
    serde_json::to_string(&payload).expect("remote payload should serialize")
}

#[tokio::test]
async fn cancel_stops_an_in_flight_pull() {
    assert_in_flight_pull_stops(false).await;
}

#[tokio::test]
async fn shutdown_stops_an_in_flight_pull() {
    assert_in_flight_pull_stops(true).await;
}

async fn assert_in_flight_pull_stops(shutdown: bool) {
    let _guard = shared_test_lock();
    clear_test_root();
    let server = FakeWebDav::start(WebDavMode::Conforming).await;
    let backend = test_backend();
    backend.initialize().await.expect("backend should initialize");
    configure_webdav(&backend, &server.url()).await;
    save_demo_host(&backend, "Demo", "demo.example.com").await;
    backend.sync_push(true).await.expect("push should bind");
    server.set_stall_sync_get(true);
    server.clear_observed_get();

    let worker = Arc::clone(&backend);
    let task = crate::runtime::runtime().spawn(async move { worker.sync_pull(true).await });
    // Wait until the stalled pull reached the server: cancelling before the
    // operation starts would legitimately be a no-op.
    for _ in 0..250 {
        if server.observed_get.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        server.observed_get.load(Ordering::SeqCst),
        "the pull should reach the server before cancel"
    );
    if shutdown {
        backend.shutdown();
    } else {
        backend.sync_cancel();
    }

    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("cancel must release the in-flight pull")
        .expect("task should not panic");
    assert!(matches!(result, Err(BackendError::Cancelled)));
    server.set_stall_sync_get(false);
    backend.shutdown();
}

#[tokio::test]
async fn cancel_stops_an_in_flight_push() {
    let _guard = shared_test_lock();
    clear_test_root();
    let server = FakeWebDav::start(WebDavMode::Conforming).await;
    let backend = test_backend();
    backend.initialize().await.expect("backend should initialize");
    configure_webdav(&backend, &server.url()).await;
    save_demo_host(&backend, "Demo", "demo.example.com").await;
    backend.sync_push(true).await.expect("push should bind");
    // A local change forces the next push to upload over the network.
    save_demo_host(&backend, "Push Cancel", "demo.example.com").await;
    server.set_stall_sync_put(true);
    server.clear_observed_put();

    let worker = Arc::clone(&backend);
    let task = crate::runtime::runtime().spawn(async move { worker.sync_push(true).await });
    for _ in 0..250 {
        if server.observed_put.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        server.observed_put.load(Ordering::SeqCst),
        "the push should reach the server before cancel"
    );
    backend.sync_cancel();

    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("cancel must release the in-flight push")
        .expect("task should not panic");
    assert!(matches!(result, Err(BackendError::Cancelled)));
    server.set_stall_sync_put(false);
    backend.shutdown();
}

#[tokio::test]
async fn a_local_edit_during_a_pull_is_not_overwritten() {
    let _guard = shared_test_lock();
    clear_test_root();
    let server = FakeWebDav::start(WebDavMode::Conforming).await;
    let backend = test_backend();
    backend.initialize().await.expect("backend should initialize");
    configure_webdav(&backend, &server.url()).await;
    save_demo_host(&backend, "Before", "demo.example.com").await;
    backend.sync_push(true).await.expect("push should bind");

    server.set_stall_sync_get(true);
    server.clear_observed_get();
    let worker = Arc::clone(&backend);
    let task = crate::runtime::runtime().spawn(async move { worker.sync_pull(false).await });
    for _ in 0..250 {
        if server.observed_get.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // The user edits while the remote fetch is still in flight.
    save_demo_host(&backend, "Edited During Pull", "demo.example.com").await;
    server.release_stall();

    let result = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("the pull should finish once the server answers")
        .expect("task should not panic")
        .expect("the pull reports the intervention instead of failing");
    assert!(
        matches!(
            result.status,
            SyncStatusDto::PullRequired {
                reason: crate::sync_dto::SyncInterventionReasonDto::LocalChangedDuringPull,
                ..
            }
        ),
        "expected LocalChangedDuringPull, got {:?}",
        result.status
    );
    let hosts = backend.hosts_snapshot().await.expect("hosts snapshot");
    assert_eq!(
        hosts[0].name, "Edited During Pull",
        "the local edit must survive the pull"
    );
    server.set_stall_sync_get(false);
    backend.shutdown();
}

#[tokio::test]
async fn a_locked_vault_rejects_manual_sync() {
    let _guard = shared_test_lock();
    clear_test_root();
    let server = FakeWebDav::start(WebDavMode::Conforming).await;
    let backend = test_backend();
    backend.initialize().await.expect("backend should initialize");
    configure_webdav(&backend, &server.url()).await;
    miaominal_secrets::set_vault_test_parameters();
    backend
        .enable_vault("vault-passphrase".to_string())
        .await
        .expect("vault should enable");
    backend.lock_vault().await.expect("vault should lock");

    let error = backend
        .sync_push(true)
        .await
        .expect_err("a locked vault must refuse sync");
    assert!(matches!(error, BackendError::VaultLocked));
    let snapshot = backend.sync_snapshot().await.expect("snapshot");
    assert!(snapshot.vault_locked);
    backend.shutdown();
}

#[tokio::test]
async fn gist_binding_creates_a_gist_and_roundtrips() {
    let _guard = shared_test_lock();
    clear_test_root();
    let server = FakeGist::start().await;
    let backend = test_backend();
    backend.initialize().await.expect("backend should initialize");
    configure_gist(&backend, &server.base_url()).await;
    save_demo_host(&backend, "Demo", "demo.example.com").await;

    let push = backend
        .sync_push(true)
        .await
        .expect("first Gist push should create the gist");
    assert!(matches!(push.status, SyncStatusDto::Pushed { .. }));
    assert!(server.content().is_some(), "the gist must hold the payload");
    let config = backend.sync_snapshot().await.expect("snapshot").config;
    assert!(config.gist_id.is_some(), "the gist id must be persisted");

    let remote = backend
        .sync_remote_state()
        .await
        .expect("remote state should answer");
    assert!(matches!(
        remote,
        crate::sync_dto::SyncRemoteStateDto::NotModified
            | crate::sync_dto::SyncRemoteStateDto::UpToDate
    ));

    // Another device replaces the gist content; the poll must notice it.
    server.set_content(&build_remote_payload_with_host("Gist Remote", "gist.example.com"));
    let remote = backend
        .sync_remote_state()
        .await
        .expect("remote state should answer after the change");
    assert!(
        matches!(
            remote,
            crate::sync_dto::SyncRemoteStateDto::Updated { content_revision: Some(_), .. }
        ),
        "a changed gist must be reported as updated: {remote:?}"
    );
    backend.shutdown();
}

#[tokio::test]
async fn legacy_device_credentials_migrate_into_the_backend_once() {
    let _guard = shared_test_lock();
    clear_test_root();
    let host = test_host();
    host.credential_set("sync:gist-token".into(), "legacy-token".into())
        .expect("legacy test credential");
    host.credential_set("sync:webdav-password".into(), "legacy-password".into())
        .expect("legacy test credential");
    host.credential_set("sync:sync-passphrase".into(), "legacy-passphrase".into())
        .expect("legacy test credential");
    let backend = Backend::new(
        Arc::clone(&host) as Arc<dyn crate::platform::PlatformHost>,
        BackendConfig {
            data_dir: business_root().to_string_lossy().into_owned(),
            cache_dir: cache_root().to_string_lossy().into_owned(),
        },
    );
    backend.initialize().await.expect("backend should initialize");

    let result = backend
        .sync_migrate_legacy_credentials()
        .await
        .expect("migration should succeed");
    assert!(result.migrated_github_token);
    assert!(result.migrated_webdav_password);
    assert!(result.migrated_passphrase);
    assert!(host.stored("sync:gist-token").is_none());
    assert!(host.stored("sync:sync-passphrase").is_none());
    // The WebDAV password shares the Rust account name, so it was already in
    // the shared store and stays there.
    assert_eq!(
        host.stored("sync:webdav-password").as_deref(),
        Some("legacy-password")
    );
    for kind in [
        SyncCredentialKind::GithubToken,
        SyncCredentialKind::WebdavPassword,
        SyncCredentialKind::Passphrase,
    ] {
        assert_eq!(
            backend.sync_credential_health(kind).await.expect("health"),
            crate::dto::CredentialHealth::Present,
            "migrated credential {kind:?} should be present"
        );
    }
    // Idempotent: a second run finds nothing to migrate.
    let second = backend
        .sync_migrate_legacy_credentials()
        .await
        .expect("second migration should succeed");
    assert!(!second.migrated_github_token);
    backend.shutdown();
}

#[tokio::test]
async fn a_differing_legacy_credential_is_reported_and_kept() {
    let _guard = shared_test_lock();
    clear_test_root();
    let host = test_host();
    host.credential_set("sync:gist-token".into(), "legacy-token".into())
        .expect("legacy test credential");
    let backend = Backend::new(
        Arc::clone(&host) as Arc<dyn crate::platform::PlatformHost>,
        BackendConfig {
            data_dir: business_root().to_string_lossy().into_owned(),
            cache_dir: cache_root().to_string_lossy().into_owned(),
        },
    );
    backend.initialize().await.expect("backend should initialize");
    backend
        .set_sync_credential(
            SyncCredentialKind::GithubToken,
            CredentialAction::Set {
                value: "configured-token".to_string(),
            },
        )
        .await
        .expect("configured token should save");

    let error = backend
        .sync_migrate_legacy_credentials()
        .await
        .expect_err("a differing legacy credential must not overwrite the configured one");
    assert!(matches!(error, BackendError::SyncConflict { .. }));
    assert_eq!(
        host.stored("sync:gist-token").as_deref(),
        Some("legacy-token"),
        "the only remaining copy must be kept"
    );
    backend.shutdown();
}

#[tokio::test]
async fn a_failed_migration_verification_keeps_the_legacy_entry() {
    let _guard = shared_test_lock();
    clear_test_root();
    let host = test_host();
    host.credential_set("sync:gist-token".into(), "legacy-token".into())
        .expect("legacy test credential");
    // The configured check (read 1) and the legacy read (read 2) succeed; the
    // verification read then fails, forcing the rollback path.
    host.invalidate_after(2);
    let backend = Backend::new(
        Arc::clone(&host) as Arc<dyn crate::platform::PlatformHost>,
        BackendConfig {
            data_dir: business_root().to_string_lossy().into_owned(),
            cache_dir: cache_root().to_string_lossy().into_owned(),
        },
    );
    backend.initialize().await.expect("backend should initialize");

    let error = backend
        .sync_migrate_legacy_credentials()
        .await
        .expect_err("a failed verification must fail the migration");
    assert!(
        matches!(error, BackendError::Storage { .. } | BackendError::Platform { .. }),
        "unexpected error: {error:?}"
    );
    assert_eq!(
        host.stored("sync:gist-token").as_deref(),
        Some("legacy-token"),
        "the legacy entry must survive a failed migration"
    );
    assert!(
        host.stored("sync:github-token").is_none(),
        "the half-written target must be rolled back"
    );
    backend.shutdown();
}

#[tokio::test]
async fn cross_end_roundtrip_preserves_proxies_keys_ai_and_unshown_fields() {
    let _guard = shared_test_lock();
    clear_test_root();
    let server = FakeWebDav::start(WebDavMode::Conforming).await;
    let mut desktop = DesktopFixture::create(&server.url());
    desktop.push().await;

    // Android pulls the desktop payload (first pull is explicitly confirmed).
    let backend = test_backend();
    backend.initialize().await.expect("backend should initialize");
    configure_webdav(&backend, &server.url()).await;
    backend
        .sync_pull(true)
        .await
        .expect("first pull should apply the desktop payload");
    let hosts = backend.hosts_snapshot().await.expect("hosts snapshot");
    let desktop_host = hosts
        .iter()
        .find(|host| host.id == "session-1")
        .expect("the desktop host should arrive");
    assert!(desktop_host.tags.contains(&"desk-tag".to_string()));
    // Unshown fields are not part of the DTO; assert them on the stored data.
    let android_content = miaominal_storage::config_store::store::SessionStore::with_path(
        miaominal_paths::config_file("sessions.toml").expect("sessions path"),
    )
    .read_sessions_content()
    .expect("stored sessions should read")
    .unwrap_or_default();
    assert!(android_content.contains("GBK"), "charset lost: {android_content}");
    assert!(android_content.contains("P6_ENV"), "environment lost");
    assert!(android_content.contains("/var/log"), "favorites lost");

    // Android edits one displayed field and pushes back.
    backend
        .save_host(crate::dto::HostUpdate {
            id: "session-1".to_string(),
            name: Some("Android Rename".to_string()),
            group: None,
            tags: None,
            kind: None,
            host: None,
            port: None,
            username: None,
            auth_method: None,
            private_key_path: None,
            managed_key_id: None,
            certificate_path: None,
            startup_command: None,
            environment_variables: None,
            shell_type: None,
            is_favorite: None,
            last_connected_at_ms: None,
        })
        .await
        .expect("local edit should save");
    backend
        .sync_push(true)
        .await
        .expect("Android push should succeed");

    // The desktop applies the round-tripped payload.
    desktop.pull().await;
    let session = desktop
        .session("session-1")
        .expect("the host should round trip");
    assert_eq!(session.name, "Android Rename");
    assert_eq!(session.charset, "GBK", "an unshown field was lost");
    assert_eq!(session.environment_variables.len(), 1);
    assert_eq!(session.environment_variables[0].name, "P6_ENV");
    assert_eq!(session.environment_variables[0].value, "kept");
    assert_eq!(session.remote_path_favorites, vec!["/var/log".to_string()]);
    assert!(session.has_stored_password);
    assert_eq!(
        desktop
            .secrets
            .get_profile_secrets("session-1")
            .expect("profile secrets")
            .password
            .as_deref(),
        Some("host-password"),
        "the profile password must survive the round trip"
    );
    assert_eq!(
        desktop
            .secrets
            .get("proxy-1", miaominal_secrets::SecretKind::ProxyPassword)
            .expect("proxy secret"),
        Some("proxy-pass".to_string())
    );
    assert_eq!(
        desktop
            .secrets
            .get("key-1", miaominal_secrets::SecretKind::ManagedPrivateKey)
            .expect("key secret"),
        Some("-----BEGIN PRIVATE KEY-----\nP6\n-----END PRIVATE KEY-----".to_string())
    );
    let settings = desktop.reload_settings();
    assert_eq!(settings.terminal_font_size, 18.5);
    assert_eq!(settings.ai_providers.len(), 1);
    assert_eq!(settings.ai_providers[0].name, "P6 AI Provider");
    assert!(settings.web_search.enabled);
    backend.shutdown();
}

/// Desktop-side fixture: real stores, a real sync engine, and a temporary
/// credential backend. It plays the role of the desktop application.
struct DesktopFixture {
    _temp: tempfile::TempDir,
    session_store: miaominal_storage::config_store::store::SessionStore,
    settings_store: miaominal_storage::SettingsStore,
    secrets: miaominal_secrets::SecretStore,
    engine: miaominal_sync::SyncEngine,
    service: miaominal_services::SyncService,
}

impl DesktopFixture {
    fn create(webdav_url: &str) -> Self {
        use miaominal_secrets::{APP_CREDENTIAL_SERVICE, CredentialStore, ProtectedPassphrase,
            SecretKind, VaultCredentialBackend};
        miaominal_secrets::set_vault_test_parameters();
        let temp = tempfile::tempdir().expect("desktop temp dir");
        let credentials = CredentialStore::with_backend(
            APP_CREDENTIAL_SERVICE,
            VaultCredentialBackend::new_with_path(
                temp.path().join("vault.json"),
                ProtectedPassphrase::try_from_string("desktop-vault".to_string())
                    .expect("desktop vault passphrase"),
            ),
        );
        credentials.initialize().expect("desktop vault should initialize");
        let secrets = miaominal_secrets::SecretStore::with_credentials(credentials.clone());
        let session_store = miaominal_storage::config_store::store::SessionStore::with_path(
            temp.path().join("sessions.toml"),
        );
        let proxy_store =
            miaominal_storage::ProxyStore::with_path(temp.path().join("proxies.toml"));
        let key_store = miaominal_storage::keychain_store::ManagedKeyStore::with_path(
            temp.path().join("managed_keys.toml"),
        );
        let mut settings_store =
            miaominal_storage::SettingsStore::load_with_path(temp.path().join("settings.toml"))
                .expect("desktop settings should load");
        settings_store.update(|settings| {
            settings.terminal_font_size = 18.5;
            settings.ai_providers = vec![miaominal_settings::AiProviderConfig {
                id: "ai-1".to_string(),
                name: "P6 AI Provider".to_string(),
                kind: miaominal_settings::AiProviderKind::OpenAi,
                model: "gpt-test".to_string(),
                base_url: String::new(),
                api_key_env: String::new(),
                has_api_key: false,
                enabled: true,
                context_window: None,
                temperature: None,
                max_tokens: None,
                reasoning_effort: miaominal_settings::AiReasoningEffort::Default,
            }];
            settings.web_search.enabled = true;
        });

        let mut profile = miaominal_core::profile::SessionProfile::blank("session-1", 1);
        profile.name = "Desktop Host".to_string();
        profile.group = "Desktop".to_string();
        profile.tags = vec!["desk-tag".to_string()];
        profile.host = "desktop.example.com".to_string();
        profile.port = 2200;
        profile.username = "root".to_string();
        profile.charset = "GBK".to_string();
        profile.environment_variables = vec![miaominal_core::profile::SessionEnvironmentVariable {
            name: "P6_ENV".to_string(),
            value: "kept".to_string(),
        }];
        profile.remote_path_favorites = vec!["/var/log".to_string()];
        profile.shell_type = miaominal_core::profile::ShellType::Fish;
        profile.has_stored_password = true;
        session_store.save(&[profile]).expect("desktop sessions");
        secrets
            .set("session-1", SecretKind::Password, "host-password")
            .expect("host password secret");
        let proxy = miaominal_core::proxy::ProxyProfile {
            id: "proxy-1".to_string(),
            name: "Desktop Proxy".to_string(),
            protocol: miaominal_core::proxy::ProxyProtocol::Socks5,
            host: "proxy.example.com".to_string(),
            port: 1080,
            auth_mode: miaominal_core::proxy::ProxyAuthMode::UsernamePassword,
            username: "proxy-user".to_string(),
            resolve_dns_through_proxy: true,
            has_stored_password: true,
        };
        proxy_store.save(&[proxy]).expect("desktop proxies");
        secrets
            .set("proxy-1", SecretKind::ProxyPassword, "proxy-pass")
            .expect("proxy password secret");
        key_store
            .save(&[miaominal_core::keychain::ManagedKeyRecord {
                id: "key-1".to_string(),
                name: "Desktop Key".to_string(),
                algorithm: "ssh-ed25519".to_string(),
                public_key: "ssh-ed25519 AAAA test".to_string(),
                source: miaominal_core::keychain::ManagedKeySource::Generated,
            }])
            .expect("desktop keys");
        secrets
            .set(
                "key-1",
                SecretKind::ManagedPrivateKey,
                "-----BEGIN PRIVATE KEY-----\nP6\n-----END PRIVATE KEY-----",
            )
            .expect("key secret");

        let config = miaominal_sync::SyncConfig {
            provider: miaominal_sync::SyncProvider::WebDav,
            webdav_url: webdav_url.to_string(),
            webdav_username: WEBDAV_USER.to_string(),
            has_webdav_password: true,
            has_passphrase: true,
            device_id: "desktop-device".to_string(),
            ..miaominal_sync::SyncConfig::default()
        };
        let engine = miaominal_sync::SyncEngine {
            config_store: miaominal_sync::SyncConfigStore::with_credentials(
                temp.path().join("sync_config.toml"),
                config,
                credentials,
            ),
            last_capability_report: None,
        };
        engine
            .config_store
            .set_webdav_password(WEBDAV_PASSWORD)
            .expect("desktop webdav password");
        engine
            .config_store
            .set_passphrase(SYNC_PASSPHRASE)
            .expect("desktop sync passphrase");
        let service = miaominal_services::SyncService::new(
            tokio::runtime::Handle::current(),
            Some(session_store.clone()),
            Some(proxy_store.clone()),
            Some(
                miaominal_storage::config_store::store::SnippetStore::with_path(
                    temp.path().join("snippets.toml"),
                ),
            ),
            Some(key_store.clone()),
            secrets.clone(),
        )
        .expect("desktop sync service");
        Self {
            _temp: temp,
            session_store,
            settings_store,
            secrets,
            engine,
            service,
        }
    }

    async fn push(&mut self) {
        self.service
            .replace_secrets(self.secrets.clone());
        self.service
            .push_manual(self.engine.clone(), self.settings_store.clone())
            .await
            .expect("desktop push should succeed");
    }

    async fn pull(&self) {
        self.service
            .replace_secrets(self.secrets.clone());
        self.service
            .pull_force(self.engine.clone(), self.settings_store.clone())
            .await
            .expect("desktop pull should succeed");
    }

    fn session(&self, id: &str) -> Option<miaominal_core::profile::SessionProfile> {
        self.session_store
            .load(&self.secrets)
            .expect("desktop sessions should load")
            .into_iter()
            .find(|profile| profile.id == id)
    }

    fn reload_settings(&self) -> miaominal_settings::AppSettings {
        miaominal_storage::SettingsStore::load_with_path(self.settings_path())
            .expect("desktop settings should reload")
            .settings()
            .clone()
    }

    fn settings_path(&self) -> std::path::PathBuf {
        self._temp.path().join("settings.toml")
    }
}

#[tokio::test]
async fn import_preview_reports_warnings_and_commits_hosts() {
    let _guard = shared_test_lock();
    clear_test_root();
    let backend = test_backend();
    backend.initialize().await.expect("backend should initialize");

    let content = b"Host imported-demo\n  HostName demo.example.com\n  User akko\n  ProxyJump bastion\n";
    let preview = backend
        .preview_import(
            ImportSourceDto::OpenSsh,
            "config".to_string(),
            content.to_vec(),
        )
        .await
        .expect("OpenSSH content should parse");
    assert_eq!(preview.entries.len(), 1);
    assert_eq!(preview.entries[0].host, "demo.example.com");
    assert!(
        preview
            .warnings
            .iter()
            .any(|warning| warning.reason.contains("proxy_jump")),
        "ProxyJump must be reported: {:?}",
        preview.warnings
    );

    let commit = backend
        .commit_import(preview.preview_id)
        .await
        .expect("commit should succeed");
    assert_eq!(commit.imported, 1);
    let hosts = backend.hosts_snapshot().await.expect("hosts snapshot");
    assert!(hosts.iter().any(|host| host.name == "imported-demo"));

    // A consumed preview cannot be committed twice.
    assert!(matches!(
        backend.commit_import(preview.preview_id).await,
        Err(BackendError::InvalidArgument { .. })
    ));
    backend.shutdown();
}
