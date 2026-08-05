use miaominal_core::keychain::ManagedKeyRecord;
use miaominal_core::known_host::KnownHostEntry;
use miaominal_core::profile::SessionProfile;
use miaominal_core::proxy::ProxyProfile;
use miaominal_core::snippet::SnippetRecord;
use miaominal_secrets::SecretStore;
use miaominal_settings::{OpenSshIntegrationMode, SshBridgeConfig};
use miaominal_ssh::{SshBridgeEndpoint, SshBridgeStatus};
use miaominal_storage::chat_store::ChatSessionRecord;
use miaominal_storage::config_store::store::{SessionStore, SnippetStore};
use miaominal_storage::keychain_store::ManagedKeyStore;
use miaominal_storage::{ProxyStore, known_hosts_store::KnownHostsStore};
use std::sync::Arc;
use tokio::runtime::Handle as TokioHandle;

use crate::{
    AgentService, ChatService, OpenSshIntegrationService, PortForwardManager, SshBridgeService,
};

#[derive(Clone)]
pub struct AppServices {
    pub runtime: TokioHandle,
    pub session_store: Option<SessionStore>,
    pub proxy_store: Option<ProxyStore>,
    pub snippet_store: Option<SnippetStore>,
    pub secrets: SecretStore,
    pub known_hosts: KnownHostsStore,
    pub keychain_store: Option<ManagedKeyStore>,
    pub agent_service: AgentService,
    pub port_forward_manager: PortForwardManager,
    pub ssh_bridge_service: SshBridgeService,
    pub open_ssh_integration_service: OpenSshIntegrationService,
}

pub struct LoadedAppData {
    pub services: AppServices,
    pub known_hosts_entries: Vec<KnownHostEntry>,
    pub managed_keys: Vec<ManagedKeyRecord>,
    pub chat_service: Option<Arc<ChatService>>,
    pub chat_sessions: Vec<ChatSessionRecord>,
    pub sessions: Vec<SessionProfile>,
    pub proxies: Vec<ProxyProfile>,
    pub snippets: Vec<SnippetRecord>,
    pub selected_profile: Option<usize>,
    pub status_message: String,
}

impl AppServices {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        runtime: TokioHandle,
        session_store: Option<SessionStore>,
        proxy_store: Option<ProxyStore>,
        snippet_store: Option<SnippetStore>,
        secrets: SecretStore,
        known_hosts: KnownHostsStore,
        keychain_store: Option<ManagedKeyStore>,
        ssh_bridge_config: SshBridgeConfig,
    ) -> Self {
        let agent_service =
            AgentService::new(runtime.clone(), secrets.clone(), known_hosts.clone());
        let port_forward_manager =
            PortForwardManager::new(runtime.clone(), secrets.clone(), known_hosts.clone());
        let config_root = miaominal_paths::config_dir().unwrap_or_else(|_| {
            std::env::temp_dir().join(format!("miaominal-{}", std::process::id()))
        });
        let endpoint = SshBridgeEndpoint::derive(&config_root).unwrap_or_else(|error| {
            log::warn!("failed to derive SSH Bridge endpoint: {error:?}");
            SshBridgeEndpoint::derive(
                &std::env::temp_dir().join(format!("miaominal-bridge-{}", std::process::id())),
            )
            .expect("temporary SSH Bridge endpoint should be derivable")
        });
        let instance_id = SshBridgeEndpoint::instance_id(&config_root)
            .unwrap_or_else(|_| format!("process-{}", std::process::id()));
        let ssh_dir = OpenSshIntegrationService::current_user_ssh_dir().unwrap_or_else(|error| {
            log::warn!("failed to locate OpenSSH config directory: {error:?}");
            config_root.join("openssh")
        });
        let bridge_known_hosts_path = ssh_dir
            .join("miaominal")
            .join(&instance_id)
            .join("bridge_known_hosts");
        let ssh_bridge_service = SshBridgeService::new(
            runtime.clone(),
            endpoint,
            instance_id.clone(),
            bridge_known_hosts_path,
            ssh_bridge_config,
            secrets.clone(),
            known_hosts.clone(),
        );
        let open_ssh_integration_service = OpenSshIntegrationService::new(
            ssh_bridge_service.clone(),
            ssh_dir,
            instance_id.clone(),
        );
        Self {
            runtime,
            session_store,
            proxy_store,
            snippet_store,
            secrets,
            known_hosts,
            keychain_store,
            agent_service,
            port_forward_manager,
            ssh_bridge_service,
            open_ssh_integration_service,
        }
    }

    pub fn load(
        runtime: TokioHandle,
        local_vault_enabled: bool,
        open_ssh_integration_mode: OpenSshIntegrationMode,
        ssh_bridge_config: SshBridgeConfig,
    ) -> LoadedAppData {
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
                Ok(mut sessions) => {
                    let stale_forward_state = reset_stale_port_forward_state(&mut sessions);
                    if stale_forward_state && let Err(error) = store.save(&sessions) {
                        log::warn!(
                            "failed to reset stale port-forward enabled state at startup: {error:?}"
                        );
                    }
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
        let chat_result = if miaominal_paths::credential_policy().ok()
            == Some(miaominal_paths::CredentialPolicy::LocalVaultRequired)
        {
            ChatService::open(&secrets.credentials())
        } else {
            ChatService::open_default()
        };

        let (proxy_store, proxies, proxy_warning) = match ProxyStore::new() {
            Ok(store) => match store.load(&secrets) {
                Ok(proxies) => (Some(store), proxies, None),
                Err(error) => {
                    log::warn!("proxy store load failed: {error:?}");
                    (
                        Some(store),
                        Vec::new(),
                        Some(format!("Proxy configuration could not be loaded: {error}")),
                    )
                }
            },
            Err(error) => {
                log::warn!("proxy store unavailable: {error:?}");
                (
                    None,
                    Vec::new(),
                    Some(format!("Proxy storage is unavailable: {error}")),
                )
            }
        };
        let (chat_service, chat_sessions) = match chat_result {
            Ok(service) => {
                let sessions = service.list_sessions().unwrap_or_else(|error| {
                    log::warn!("failed to list chat sessions: {error:?}");
                    Vec::new()
                });
                (Some(Arc::new(service)), sessions)
            }
            Err(error) => {
                log::warn!("chat service unavailable: {error:?}");
                (None, Vec::new())
            }
        };
        let selected_profile = (!sessions.is_empty()).then_some(0);
        let status_message = proxy_warning
            .map(|warning| format!("{status_message} {warning}"))
            .unwrap_or(status_message);

        let services = Self::new(
            runtime,
            session_store,
            proxy_store,
            snippet_store,
            secrets,
            known_hosts,
            keychain_store,
            ssh_bridge_config,
        );
        services
            .ssh_bridge_service
            .refresh_routes(sessions.clone(), proxies.clone());
        services
            .port_forward_manager
            .replace_catalogs(sessions.clone(), proxies.clone());
        if open_ssh_integration_mode == OpenSshIntegrationMode::Bridge {
            services
                .open_ssh_integration_service
                .defer_bridge_activation(sessions.clone(), proxies.clone());
            let service = services.ssh_bridge_service.clone();
            let integration = services.open_ssh_integration_service.clone();
            service.set_desired_enabled(true);
            services.runtime.spawn(async move {
                if let Err(error) = service.reconcile_desired_state().await {
                    log::warn!("failed to restore SSH Bridge: {error:?}");
                    return;
                }
                if matches!(service.status(), SshBridgeStatus::Running { .. })
                    && let Err(error) = integration.set_mode(OpenSshIntegrationMode::Bridge)
                {
                    log::warn!(
                        "failed to activate managed OpenSSH Bridge config after startup: {error:?}"
                    );
                }
            });
        } else if let Err(error) = services.open_ssh_integration_service.sync(
            open_ssh_integration_mode,
            sessions.clone(),
            proxies.clone(),
        ) {
            log::warn!("failed to synchronize managed OpenSSH config: {error:?}");
        }

        LoadedAppData {
            services,
            known_hosts_entries,
            managed_keys,
            chat_service,
            chat_sessions,
            sessions,
            proxies,
            snippets,
            selected_profile,
            status_message,
        }
    }
}

fn reset_stale_port_forward_state(sessions: &mut [SessionProfile]) -> bool {
    let mut changed = false;
    for profile in sessions {
        for rule in &mut profile.port_forwarding_rules {
            changed |= rule.enabled;
            rule.enabled = false;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::profile::{PortForwardKind, PortForwardRule};

    #[test]
    fn startup_reset_clears_only_stale_enabled_forward_flags() {
        let mut profile = SessionProfile::blank("profile", 1);
        profile.port_forwarding_rules = vec![
            PortForwardRule {
                id: "enabled".into(),
                label: String::new(),
                kind: PortForwardKind::Local,
                listen_host: "127.0.0.1".into(),
                listen_port: 1000,
                target_host: "127.0.0.1".into(),
                target_port: 2000,
                enabled: true,
            },
            PortForwardRule {
                id: "disabled".into(),
                label: String::new(),
                kind: PortForwardKind::Remote,
                listen_host: "127.0.0.1".into(),
                listen_port: 3000,
                target_host: "127.0.0.1".into(),
                target_port: 4000,
                enabled: false,
            },
        ];

        assert!(reset_stale_port_forward_state(std::slice::from_mut(
            &mut profile
        )));
        assert!(
            profile
                .port_forwarding_rules
                .iter()
                .all(|rule| !rule.enabled)
        );
        assert!(!reset_stale_port_forward_state(std::slice::from_mut(
            &mut profile
        )));
    }
}
