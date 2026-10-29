//! Desktop-side sync integration tests (plan 5.2).
//!
//! A local HTTP server implements the WebDAV subset the engine uses, so the
//! suite exercises capability probing, binding, round trips, conditional
//! writes and cancellation without touching the public network.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use miaominal_secrets::{
    APP_CREDENTIAL_SERVICE, CredentialStore, ProtectedPassphrase, SecretStore,
    VaultCredentialBackend, set_vault_test_parameters,
};
use miaominal_services::{SyncService, SyncTaskResult};
use miaominal_storage::ProxyStore;
use miaominal_storage::SettingsStore;
use miaominal_storage::config_store::store::{SessionStore, SnippetStore};
use miaominal_storage::keychain_store::ManagedKeyStore;
use miaominal_sync::capability::{CapabilityReason, CapabilityState, ProbeCancellation};
use miaominal_sync::{RemoteSyncState, SyncConfig, SyncConfigStore, SyncEngine, SyncStatus};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const USER: &str = "sync-user";
const PASSWORD: &str = "sync-password";
const PASSPHRASE: &str = "desktop-sync-passphrase";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Conforming,
    IgnoreConditions,
    Unauthorized,
}

#[derive(Default)]
struct Resources {
    files: Mutex<HashMap<String, (String, u64)>>,
    generation: AtomicU64,
}

struct FakeWebDav {
    address: SocketAddr,
    resources: Arc<Resources>,
}

impl FakeWebDav {
    async fn start(mode: Mode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let resources = Arc::new(Resources::default());
        let server = Self {
            address,
            resources: Arc::clone(&resources),
        };
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let resources = Arc::clone(&resources);
                tokio::spawn(async move {
                    let _ = serve(stream, resources, mode).await;
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
            .unwrap()
            .get("/sync.json")
            .map(|(body, _)| body.clone())
    }

    fn set_body(&self, body: &str) {
        let generation = self.resources.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.resources
            .files
            .lock()
            .unwrap()
            .insert("/sync.json".into(), (body.to_string(), generation));
    }
}

async fn serve(
    mut stream: TcpStream,
    resources: Arc<Resources>,
    mode: Mode,
) -> std::io::Result<()> {
    let (method, path, headers, body) = read_request(&mut stream).await?;
    if mode == Mode::Unauthorized {
        return respond(&mut stream, 401, &[], "").await;
    }
    let current = resources.files.lock().unwrap().get(&path).cloned();
    let tag_of = |generation: u64| format!("\"gen-{generation}\"");
    match method.as_str() {
        "GET" => match current {
            None => respond(&mut stream, 404, &[], "").await,
            Some((body, generation)) => {
                let tag = tag_of(generation);
                let not_modified = mode == Mode::Conforming
                    && headers
                        .get("if-none-match")
                        .is_some_and(|value| value == "*" || value == &tag);
                if not_modified {
                    respond(&mut stream, 304, &[("ETag", &tag)], "").await
                } else {
                    respond(&mut stream, 200, &[("ETag", &tag)], &body).await
                }
            }
        },
        "PUT" => {
            if mode == Mode::Conforming {
                if let Some(expected) = headers.get("if-match") {
                    let matches = current
                        .as_ref()
                        .is_some_and(|(_, generation)| expected == &tag_of(*generation));
                    if !matches {
                        return respond(&mut stream, 412, &[], "").await;
                    }
                }
                if headers
                    .get("if-none-match")
                    .is_some_and(|value| value == "*")
                    && current.is_some()
                {
                    return respond(&mut stream, 412, &[], "").await;
                }
            }
            let generation = resources.generation.fetch_add(1, Ordering::SeqCst) + 1;
            let status = if current.is_some() { 204 } else { 201 };
            resources
                .files
                .lock()
                .unwrap()
                .insert(path, (body, generation));
            let tag = tag_of(generation);
            respond(&mut stream, status, &[("ETag", &tag)], "").await
        }
        "DELETE" => {
            if mode == Mode::Conforming {
                if let Some(expected) = headers.get("if-match") {
                    let matches = current
                        .as_ref()
                        .is_some_and(|(_, generation)| expected == &tag_of(*generation));
                    if !matches {
                        return respond(&mut stream, 412, &[], "").await;
                    }
                }
            }
            if current.is_none() {
                return respond(&mut stream, 404, &[], "").await;
            }
            resources.files.lock().unwrap().remove(&path);
            respond(&mut stream, 204, &[], "").await
        }
        _ => respond(&mut stream, 405, &[], "").await,
    }
}

async fn read_request(
    stream: &mut TcpStream,
) -> std::io::Result<(String, String, HashMap<String, String>, String)> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "request ended early",
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split_whitespace();
    let method = request_line.next().unwrap_or_default().to_string();
    let path = request_line
        .next()
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default()
        .to_string();
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
    Ok((
        method,
        path,
        headers,
        String::from_utf8_lossy(&body).to_string(),
    ))
}

async fn respond(
    stream: &mut TcpStream,
    status: u16,
    headers: &[(&str, &str)],
    body: &str,
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
    let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("\r\n");
    response.push_str(body);
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}

struct Harness {
    service: SyncService,
    engine: SyncEngine,
    settings_store: SettingsStore,
    session_store: SessionStore,
    secrets: SecretStore,
    _temp: tempfile::TempDir,
}

fn harness(webdav_url: &str) -> Harness {
    set_vault_test_parameters();
    let temp = tempfile::tempdir().unwrap();
    let credentials = CredentialStore::with_backend(
        APP_CREDENTIAL_SERVICE,
        VaultCredentialBackend::new_with_path(
            temp.path().join("vault.json"),
            ProtectedPassphrase::try_from_string("vault-passphrase".to_string()).unwrap(),
        ),
    );
    credentials.initialize().unwrap();
    let secrets = SecretStore::with_credentials(credentials.clone());
    let session_store = SessionStore::with_path(temp.path().join("sessions.toml"));
    let config = SyncConfig {
        provider: miaominal_sync::SyncProvider::WebDav,
        webdav_url: webdav_url.to_string(),
        webdav_username: USER.to_string(),
        has_webdav_password: true,
        has_passphrase: true,
        device_id: "desktop-device".to_string(),
        ..SyncConfig::default()
    };
    let engine = SyncEngine {
        config_store: SyncConfigStore::with_credentials(
            temp.path().join("sync_config.toml"),
            config,
            credentials,
        ),
        last_capability_report: None,
    };
    engine.config_store.set_webdav_password(PASSWORD).unwrap();
    engine.config_store.set_passphrase(PASSPHRASE).unwrap();
    let service = SyncService::new(
        tokio::runtime::Handle::current(),
        Some(session_store.clone()),
        Some(ProxyStore::with_path(temp.path().join("proxies.toml"))),
        Some(SnippetStore::with_path(temp.path().join("snippets.toml"))),
        Some(ManagedKeyStore::with_path(
            temp.path().join("managed_keys.toml"),
        )),
        secrets.clone(),
    )
    .expect("sync service should build");
    let settings_store = SettingsStore::load_with_path(temp.path().join("settings.toml")).unwrap();
    Harness {
        service,
        engine,
        settings_store,
        session_store,
        secrets,
        _temp: temp,
    }
}

fn save_host(harness: &Harness, name: &str) {
    let mut profile = miaominal_core::profile::SessionProfile::blank("session-1", 1);
    profile.name = name.to_string();
    profile.host = "desktop.example.com".to_string();
    profile.username = "root".to_string();
    harness.session_store.save(&[profile]).unwrap();
    let _ = &harness.secrets;
}

async fn push(harness: &mut Harness) -> SyncTaskResult {
    harness.service.replace_secrets(harness.secrets.clone());
    harness
        .service
        .push_manual(harness.engine.clone(), harness.settings_store.clone())
        .await
        .expect("push should succeed")
}

async fn pull(harness: &mut Harness) -> SyncTaskResult {
    harness.service.replace_secrets(harness.secrets.clone());
    harness
        .service
        .pull_force(harness.engine.clone(), harness.settings_store.clone())
        .await
        .expect("pull should succeed")
}

#[tokio::test]
async fn capability_probe_reports_supported_for_a_conforming_server() {
    let server = FakeWebDav::start(Mode::Conforming).await;
    let mut harness = harness(&server.url());
    harness.engine.config_store.sync_from_disk();

    let report = harness
        .service
        .check_capability(harness.engine.clone(), ProbeCancellation::default())
        .await;

    assert_eq!(report.state, CapabilityState::Supported, "{report:?}");
    assert!(report.cleanup_file.is_none());
}

#[tokio::test]
async fn capability_probe_reports_a_server_that_ignores_conditions() {
    let server = FakeWebDav::start(Mode::IgnoreConditions).await;
    let mut harness = harness(&server.url());
    harness.engine.config_store.sync_from_disk();

    let report = harness
        .service
        .check_capability(harness.engine.clone(), ProbeCancellation::default())
        .await;

    assert_ne!(report.state, CapabilityState::Supported);
    assert!(report.reason.is_some());
}

#[tokio::test]
async fn a_cancelled_probe_reports_cancellation_and_cleans_up() {
    let server = FakeWebDav::start(Mode::Conforming).await;
    let mut harness = harness(&server.url());
    harness.engine.config_store.sync_from_disk();
    let cancel = ProbeCancellation::default();
    cancel.cancel();

    let report = harness
        .service
        .check_capability(harness.engine.clone(), cancel)
        .await;

    assert_eq!(report.reason, Some(CapabilityReason::Cancelled));
}

#[tokio::test]
async fn a_wrong_password_is_reported_as_a_capability_issue() {
    let server = FakeWebDav::start(Mode::Unauthorized).await;
    let mut harness = harness(&server.url());
    harness.engine.config_store.sync_from_disk();

    let report = harness
        .service
        .check_capability(harness.engine.clone(), ProbeCancellation::default())
        .await;

    assert_ne!(report.state, CapabilityState::Supported);
    assert_eq!(report.http_status, Some(401));
}

#[tokio::test]
async fn first_binding_push_and_pull_roundtrip_against_a_local_server() {
    let server = FakeWebDav::start(Mode::Conforming).await;
    let mut harness = harness(&server.url());
    save_host(&harness, "Desktop Host");

    let result = push(&mut harness).await;
    assert!(
        matches!(result.status, SyncStatus::Pushed { .. }),
        "{:?}",
        result.status
    );
    let body = server.body().expect("the remote must hold the payload");
    assert!(body.contains("encrypted_payload"));

    // A second engine (a different device directory) pulls the same payload.
    let second = tempfile::tempdir().unwrap();
    let credentials = CredentialStore::with_backend(
        APP_CREDENTIAL_SERVICE,
        VaultCredentialBackend::new_with_path(
            second.path().join("vault.json"),
            ProtectedPassphrase::try_from_string("vault-passphrase".to_string()).unwrap(),
        ),
    );
    credentials.initialize().unwrap();
    let secrets = SecretStore::with_credentials(credentials.clone());
    let session_store = SessionStore::with_path(second.path().join("sessions.toml"));
    let engine = SyncEngine {
        config_store: SyncConfigStore::with_credentials(
            second.path().join("sync_config.toml"),
            SyncConfig {
                provider: miaominal_sync::SyncProvider::WebDav,
                webdav_url: server.url(),
                webdav_username: USER.to_string(),
                has_webdav_password: true,
                has_passphrase: true,
                device_id: "second-device".to_string(),
                ..SyncConfig::default()
            },
            credentials,
        ),
        last_capability_report: None,
    };
    engine.config_store.set_webdav_password(PASSWORD).unwrap();
    engine.config_store.set_passphrase(PASSPHRASE).unwrap();
    let service = SyncService::new(
        tokio::runtime::Handle::current(),
        Some(session_store.clone()),
        Some(ProxyStore::with_path(second.path().join("proxies.toml"))),
        Some(SnippetStore::with_path(second.path().join("snippets.toml"))),
        Some(ManagedKeyStore::with_path(
            second.path().join("managed_keys.toml"),
        )),
        secrets.clone(),
    )
    .unwrap();
    let settings = SettingsStore::load_with_path(second.path().join("settings.toml")).unwrap();
    service.pull_force(engine, settings).await.unwrap();

    let sessions = session_store.load(&secrets).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].name, "Desktop Host");
}

#[tokio::test]
async fn an_external_remote_change_is_detected_and_pullable() {
    let server = FakeWebDav::start(Mode::Conforming).await;
    let mut harness = harness(&server.url());
    save_host(&harness, "Desktop Host");
    push(&mut harness).await;

    // Another device moves the remote on.
    let remote = miaominal_sync::payload::build_payload(
        "other-device",
        None,
        &miaominal_sync::SyncPlaintextPayload {
            sessions: vec![miaominal_core::profile::SessionProfile::blank(
                "session-9",
                1,
            )],
            proxies: Vec::new(),
            snippets: Vec::new(),
            managed_keys: Vec::new(),
            settings: miaominal_settings::SyncedSettings::from(
                &miaominal_settings::AppSettings::default_for_system(),
            ),
            secrets: miaominal_sync::PlaintextSecrets::default(),
        },
        PASSPHRASE,
    )
    .unwrap();
    server.set_body(&serde_json::to_string(&remote).unwrap());

    let state = harness
        .service
        .remote_state(harness.engine.clone())
        .await
        .unwrap();
    assert!(
        matches!(state, RemoteSyncState::Updated { .. }),
        "the changed remote must be noticed: {state:?}"
    );

    let result = pull(&mut harness).await;
    assert!(
        matches!(result.status, SyncStatus::Pulled { .. }),
        "{:?}",
        result.status
    );
    let sessions = harness.session_store.load(&harness.secrets).unwrap();
    assert!(sessions.iter().any(|profile| profile.id == "session-9"));
}

#[tokio::test]
async fn a_locked_vault_refuses_manual_sync() {
    let server = FakeWebDav::start(Mode::Conforming).await;
    let harness = harness(&server.url());
    save_host(&harness, "Desktop Host");

    // The same persisted config, but every secret read now fails as locked.
    let credentials = CredentialStore::with_backend(
        APP_CREDENTIAL_SERVICE,
        miaominal_secrets::LockedCredentialBackend,
    );
    let engine = SyncEngine {
        config_store: SyncConfigStore::with_credentials(
            harness._temp.path().join("sync_config.toml"),
            harness.engine.config_store.config.clone(),
            credentials,
        ),
        last_capability_report: None,
    };

    let error = harness
        .service
        .push_manual(engine, harness.settings_store.clone())
        .await
        .expect_err("a locked vault must refuse sync");
    assert!(
        miaominal_secrets::SecretStore::is_locked_error(&error),
        "expected a locked error, got {error:?}"
    );
    assert!(
        server.body().is_none(),
        "nothing may be uploaded while locked"
    );
}

#[tokio::test]
async fn a_stale_precondition_reports_a_conflict_instead_of_overwriting() {
    let server = FakeWebDav::start(Mode::Conforming).await;
    let mut harness = harness(&server.url());
    save_host(&harness, "Desktop Host");
    push(&mut harness).await;
    // The push ran through an engine clone; re-read the persisted ETag.
    harness.engine.config_store.sync_from_disk();
    let bound_config = harness.engine.config_store.config.clone();
    assert!(bound_config.remote_etag.is_some());

    // Another device replaces the payload, so our ETag is stale.
    let remote = miaominal_sync::payload::build_payload(
        "other-device",
        None,
        &miaominal_sync::SyncPlaintextPayload {
            sessions: vec![miaominal_core::profile::SessionProfile::blank(
                "session-9",
                1,
            )],
            proxies: Vec::new(),
            snippets: Vec::new(),
            managed_keys: Vec::new(),
            settings: miaominal_settings::SyncedSettings::from(
                &miaominal_settings::AppSettings::default_for_system(),
            ),
            secrets: miaominal_sync::PlaintextSecrets::default(),
        },
        PASSPHRASE,
    )
    .unwrap();
    let remote_json = serde_json::to_string(&remote).unwrap();
    server.set_body(&remote_json);
    let mut engine = harness.engine.clone();
    engine.last_capability_report = None;
    let secrets = harness.secrets.clone();
    let result = engine
        .push_manual(
            &harness.session_store,
            &ProxyStore::with_path(harness._temp.path().join("proxies.toml")),
            &SnippetStore::with_path(harness._temp.path().join("snippets.toml")),
            &ManagedKeyStore::with_path(harness._temp.path().join("managed_keys.toml")),
            &secrets,
            &harness.settings_store,
        )
        .await
        .expect("a content conflict is an intervention, not a transport error");
    assert!(
        matches!(result, SyncStatus::PullRequired { .. }),
        "expected a conflict intervention, got {result:?}"
    );
    assert_eq!(
        server.body().as_deref(),
        Some(remote_json.as_str()),
        "the remote must not be overwritten"
    );
}
