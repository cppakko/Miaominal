use crate::domain::keychain::ManagedKeyRecord;
use crate::domain::known_host::KnownHostEntry;
use crate::domain::profile::SessionProfile;
use crate::domain::snippet::SnippetRecord;
use crate::infra::config_store::store::{SessionStore, SnippetStore};
use crate::infra::keychain_store::ManagedKeyStore;
use crate::infra::known_hosts_store::KnownHostsStore;
use crate::secrets::SecretStore;
use tokio::runtime::Handle as TokioHandle;

pub(crate) struct AppServices {
    pub runtime: TokioHandle,
    pub session_store: Option<SessionStore>,
    pub snippet_store: Option<SnippetStore>,
    pub secrets: SecretStore,
    pub known_hosts: KnownHostsStore,
    pub keychain_store: Option<ManagedKeyStore>,
}

pub(crate) struct LoadedAppData {
    pub services: AppServices,
    pub known_hosts_entries: Vec<KnownHostEntry>,
    pub managed_keys: Vec<ManagedKeyRecord>,
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
    ) -> Self {
        Self {
            runtime,
            session_store,
            snippet_store,
            secrets,
            known_hosts,
            keychain_store,
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
        let selected_profile = (!sessions.is_empty()).then_some(0);

        LoadedAppData {
            services: Self::new(
                runtime,
                session_store,
                snippet_store,
                secrets,
                known_hosts,
                keychain_store,
            ),
            known_hosts_entries,
            managed_keys,
            sessions,
            snippets,
            selected_profile,
            status_message,
        }
    }
}
