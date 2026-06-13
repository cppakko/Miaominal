use miaominal_core::keychain::ManagedKeyRecord;
use miaominal_core::known_host::KnownHostEntry;
use miaominal_core::profile::SessionProfile;
use miaominal_core::snippet::SnippetRecord;
use miaominal_secrets::{APP_CREDENTIAL_SERVICE, CredentialStore, SecretStore};
use miaominal_storage::chat_store::ChatSessionRecord;
use miaominal_storage::config_store::store::{SessionStore, SnippetStore};
use miaominal_storage::keychain_store::ManagedKeyStore;
use miaominal_storage::known_hosts_store::KnownHostsStore;
use tokio::runtime::Handle as TokioHandle;

use crate::ChatService;

pub struct AppServices {
    pub runtime: TokioHandle,
    pub session_store: Option<SessionStore>,
    pub snippet_store: Option<SnippetStore>,
    pub secrets: SecretStore,
    pub known_hosts: KnownHostsStore,
    pub keychain_store: Option<ManagedKeyStore>,
    pub chat_service: Option<ChatService>,
}

pub struct LoadedAppData {
    pub services: AppServices,
    pub known_hosts_entries: Vec<KnownHostEntry>,
    pub managed_keys: Vec<ManagedKeyRecord>,
    pub chat_sessions: Vec<ChatSessionRecord>,
    pub sessions: Vec<SessionProfile>,
    pub snippets: Vec<SnippetRecord>,
    pub selected_profile: Option<usize>,
    pub status_message: String,
}

impl AppServices {
    pub fn new(
        runtime: TokioHandle,
        session_store: Option<SessionStore>,
        snippet_store: Option<SnippetStore>,
        secrets: SecretStore,
        known_hosts: KnownHostsStore,
        keychain_store: Option<ManagedKeyStore>,
        chat_service: Option<ChatService>,
    ) -> Self {
        Self {
            runtime,
            session_store,
            snippet_store,
            secrets,
            known_hosts,
            keychain_store,
            chat_service,
        }
    }

    pub fn load(runtime: TokioHandle, local_vault_enabled: bool) -> LoadedAppData {
        let secrets = if local_vault_enabled {
            SecretStore::new_locked_vault()
        } else {
            SecretStore::new()
        };

        let known_hosts = match KnownHostsStore::new() {
            Ok(store) => store,
            Err(error) => {
                log::warn!("known_hosts unavailable: {error:?}");
                KnownHostsStore::with_path(std::env::temp_dir().join("miaominal_known_hosts"))
            }
        };
        let known_hosts_entries = known_hosts.list().unwrap_or_else(|error| {
            log::warn!("failed to list known_hosts: {error:?}");
            Vec::new()
        });

        let (session_store, sessions, status_message) = match SessionStore::new() {
            Ok(store) => match store.load(&secrets) {
                Ok(sessions) => {
                    let profile_count = sessions.len();
                    let status_message = if profile_count == 0 {
                        "No saved hosts yet.".to_string()
                    } else {
                        format!(
                            "Loaded {profile_count} host profile{}.",
                            if profile_count == 1 { "" } else { "s" }
                        )
                    };
                    (Some(store), sessions, status_message)
                }
                Err(error) => (Some(store), Vec::new(), format!("Load failed: {error}")),
            },
            Err(error) => (
                None,
                Vec::new(),
                format!("Config path unavailable: {error}"),
            ),
        };

        let (snippet_store, snippets) = match SnippetStore::new() {
            Ok(store) => match store.load() {
                Ok(snippets) => (Some(store), snippets),
                Err(error) => {
                    log::warn!("snippet store load failed: {error:?}");
                    (Some(store), Vec::new())
                }
            },
            Err(error) => {
                log::warn!("snippet store unavailable: {error:?}");
                (None, Vec::new())
            }
        };

        let (keychain_store, managed_keys) = match ManagedKeyStore::new() {
            Ok(store) => match store.load() {
                Ok(keys) => (Some(store), keys),
                Err(error) => {
                    log::warn!("managed key store load failed: {error:?}");
                    (Some(store), Vec::new())
                }
            },
            Err(error) => {
                log::warn!("managed key store unavailable: {error:?}");
                (None, Vec::new())
            }
        };
        let chat_credentials = CredentialStore::new_keyring(APP_CREDENTIAL_SERVICE);
        let (chat_service, chat_sessions) = match ChatService::open(&chat_credentials) {
            Ok(service) => {
                let sessions = service.list_sessions().unwrap_or_else(|error| {
                    log::warn!("failed to list chat sessions: {error:?}");
                    Vec::new()
                });
                (Some(service), sessions)
            }
            Err(error) => {
                log::warn!("chat service unavailable: {error:?}");
                (None, Vec::new())
            }
        };
        let selected_profile = (!sessions.is_empty()).then_some(0);

        LoadedAppData {
            services: Self::new(
                runtime,
                session_store,
                snippet_store,
                secrets,
                known_hosts,
                keychain_store,
                chat_service,
            ),
            known_hosts_entries,
            managed_keys,
            chat_sessions,
            sessions,
            snippets,
            selected_profile,
            status_message,
        }
    }
}
