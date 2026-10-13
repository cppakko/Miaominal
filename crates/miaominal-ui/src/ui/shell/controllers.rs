use std::{cell::RefCell, rc::Rc};

use gpui_kit::{App, AppContext as _, Context, Entity, Subscription, Window};

use super::{
    AppView, AuthMethod, LocalVaultPassphrasePopupMode, LocalVaultStatus, ManagedKeySelectItem,
    SessionProfile, SidebarSection, SyncProvider, TabId, TabPlacement, TabState,
    WorkspaceTerminalInputExt, terminal_cell_width_default, terminal_line_height_default,
};
use crate::ui::i18n;
use miaominal_core::known_host::KnownHostEntry;
use miaominal_secrets::SecretStore;
use miaominal_settings::AppSettings;

use crate::ui::features::{agent, keychain, settings, sftp};
mod local_vault_root;
mod root;

pub(in crate::ui::shell) use crate::ui::contracts::session::{
    ClosedSessionTabState, SessionEventOutcome, SessionEventTabRemoval, SessionNotificationRequest,
    SessionNotificationTone,
};
pub(in crate::ui::shell) use crate::ui::contracts::settings::LocalVaultActionRequest;
pub(in crate::ui::shell) use crate::ui::contracts::sftp::{
    TransferredSftpTab, TransferredSftpWindowUi,
};
pub(in crate::ui::shell) use crate::ui::features::session::{
    PortForwardSessionStart, SessionController, SessionControllerArgs, SessionPortSnapshot,
    SessionPurpose, SessionSidePanelView, TransferredSessionTab, TransferredSessionWorkspaceUi,
};
pub(in crate::ui::shell) use agent::{
    AgentController, AgentControllerArgs, PromptHistoryDirection,
};
pub(in crate::ui::shell) use keychain::{KeychainController, KeychainControllerArgs};
pub(in crate::ui::shell) use local_vault_root::LocalVaultRootExt;
pub(in crate::ui::shell) use settings::{
    LocalVaultChangePassphraseResult, LocalVaultEnableResult, LocalVaultOperationResult,
    LocalVaultUnlockResult, SettingsController, SettingsControllerArgs,
};
pub(in crate::ui::shell) use sftp::{SftpController, SftpControllerArgs};

pub(in crate::ui::shell) use crate::ui::contracts::commands::*;
fn clear_managed_key_profile_references(profiles: &mut [SessionProfile], key_id: &str) -> bool {
    let mut changed = false;
    for profile in profiles {
        if profile.managed_key_id == key_id {
            profile.managed_key_id.clear();
            if profile.auth_method == Some(AuthMethod::ManagedKey) {
                profile.auth_method = Some(AuthMethod::Password);
            }
            changed = true;
        }
    }
    changed
}

fn terminal_metrics_changed(previous: &AppSettings, next: &AppSettings) -> bool {
    previous.terminal_font_family != next.terminal_font_family
        || previous.font_fallbacks != next.font_fallbacks
        || previous.terminal_font_size != next.terminal_font_size
        || previous.line_height != next.line_height
}

pub(in crate::ui::shell) struct ControllerSet {
    pub session: Entity<SessionController>,
    pub hosts: Entity<super::HostsController>,
    pub forwarding: Entity<super::ForwardingController>,
    pub snippets: Entity<super::SnippetsController>,
    pub trusted_hosts: Entity<super::TrustedHostsController>,
    pub agent: Entity<AgentController>,
    pub sftp: Entity<SftpController>,
    pub settings: Entity<SettingsController>,
    pub keychain: Entity<KeychainController>,
}

pub(in crate::ui::shell) struct ControllerSetArgs {
    pub session: SessionControllerArgs,
    pub hosts: super::HostsControllerArgs,
    pub snippet_store: Option<miaominal_storage::config_store::store::SnippetStore>,
    pub known_hosts_entries: Vec<KnownHostEntry>,
    pub agent: AgentControllerArgs,
    pub sftp: SftpControllerArgs,
    pub keychain: KeychainControllerArgs,
    pub settings: SettingsControllerArgs,
}

impl ControllerSet {
    pub(in crate::ui::shell) fn new(
        args: ControllerSetArgs,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Self {
        let ControllerSetArgs {
            session,
            hosts,
            snippet_store,
            known_hosts_entries,
            agent,
            sftp,
            keychain,
            settings,
        } = args;
        let hosts = cx.new(|cx| super::HostsController::new(hosts, window, cx));
        let catalog = crate::ui::application::application_state(cx)
            .read(cx)
            .catalog_query();
        let trusted_hosts = cx.new(|cx| {
            super::TrustedHostsController::new(
                session.known_hosts.clone(),
                known_hosts_entries,
                catalog,
                window,
                cx,
            )
        });
        let snippets = cx.new(|cx| {
            super::SnippetsController::new(
                crate::ui::application::snippet_catalog_port(cx),
                snippet_store,
                None,
                window,
                cx,
            )
        });
        let forwarding_services = (
            crate::ui::application::application_state(cx)
                .read(cx)
                .snapshot()
                .services
                .session_store,
            session.secrets.clone(),
            session.local_vault_status,
            session.port_forward_manager.clone(),
        );
        let session = cx.new(|cx| SessionController::new(session, window, cx));
        let session_query = session.read(cx).query_port();
        let catalog_query = crate::ui::application::application_state(cx)
            .read(cx)
            .catalog_query();
        let session_terminal = session.read(cx).terminal_port();
        let forwarding = cx.new(|cx| {
            super::ForwardingController::new(
                super::ForwardingControllerArgs {
                    catalog: catalog_query.clone(),
                    profile_commit: crate::ui::application::profile_commit_port(cx),
                    store: forwarding_services.0,
                    secrets: forwarding_services.1,
                    vault_status: forwarding_services.2,
                    manager: Some(forwarding_services.3),
                    runtime: SessionController::forwarding_port(&session),
                },
                window,
                cx,
            )
        });
        Self {
            forwarding,
            snippets,
            trusted_hosts,
            hosts,
            session,
            agent: cx.new(|cx| {
                AgentController::new(
                    agent,
                    catalog_query.clone(),
                    session_query.clone(),
                    session_terminal,
                    window,
                    cx,
                )
            }),
            sftp: cx.new(|cx| {
                SftpController::new(
                    sftp,
                    catalog_query.clone(),
                    session_query.clone(),
                    window,
                    cx,
                )
            }),
            settings: cx
                .new(|cx| SettingsController::new(settings, catalog_query.clone(), window, cx)),
            keychain: cx.new(|cx| KeychainController::new(keychain, catalog_query, window, cx)),
        }
    }

    pub(in crate::ui::shell) fn root_subscriptions(
        &self,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Vec<Subscription> {
        let mut subscriptions = Vec::with_capacity(10);
        macro_rules! connect {
            ($controller:expr) => {{
                subscriptions.push(cx.observe_in(
                    $controller,
                    window,
                    |this, _controller, window, cx| {
                        this.sync_window_overlays(window, cx);
                        cx.notify();
                    },
                ));
                subscriptions.push(cx.subscribe_in(
                    $controller,
                    window,
                    |this, _controller, command: &AppCommand, window, cx| {
                        this.handle_app_command_in_window(command, window, cx);
                    },
                ));
            }};
        }

        subscriptions.push(cx.observe_in(
            &self.session,
            window,
            |this, _controller, window, cx| {
                this.sync_window_overlays(window, cx);
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &self.session,
            window,
            |this, _controller, command: &AppCommand, window, cx| {
                this.handle_app_command_in_window(command, window, cx);
            },
        ));
        subscriptions.push(
            cx.observe_in(&self.agent, window, |this, _controller, window, cx| {
                this.sync_window_overlays(window, cx);
                this.publish_chat_application_state(cx);
                cx.notify();
            }),
        );
        subscriptions.push(cx.subscribe_in(
            &self.agent,
            window,
            |this, _controller, command: &AppCommand, window, cx| {
                this.handle_app_command_in_window(command, window, cx);
            },
        ));
        connect!(&self.hosts);
        connect!(&self.trusted_hosts);
        connect!(&self.snippets);
        connect!(&self.forwarding);
        connect!(&self.sftp);
        let observed_settings = Rc::new(RefCell::new(self.settings.read(cx).settings().clone()));
        subscriptions.push(cx.observe_in(
            &self.settings,
            window,
            move |this, controller, window, cx| {
                this.sync_window_overlays(window, cx);
                let next_settings = controller.read(cx).settings().clone();
                let mut previous_settings = observed_settings.borrow_mut();
                let metrics_changed = terminal_metrics_changed(&previous_settings, &next_settings);
                let free_type_disabled = previous_settings.terminal_free_type_mode
                    && !next_settings.terminal_free_type_mode;
                *previous_settings = next_settings;
                drop(previous_settings);
                if metrics_changed {
                    this.invalidate_terminal_metrics();
                }
                if free_type_disabled {
                    this.clear_terminal_free_type_interactions(cx);
                    this.controllers
                        .session
                        .read(cx)
                        .clear_all_terminal_free_type_selections();
                }
                this.publish_settings_application_state(cx);
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &self.settings,
            window,
            |this, _controller, command, window, cx| {
                this.handle_app_command_in_window(command, window, cx);
            },
        ));
        subscriptions.push(cx.observe_in(
            &self.keychain,
            window,
            |this, controller, window, cx| {
                this.sync_window_overlays(window, cx);
                let status_message = controller.read(cx).status_message();
                if !status_message.is_empty() {
                    this.shell.status_message = status_message.to_string();
                }

                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &self.keychain,
            window,
            |this, _controller, command, window, cx| {
                this.handle_app_command_in_window(command, window, cx);
            },
        ));
        subscriptions
    }

    pub(in crate::ui::shell) fn broadcast_credentials_changed(
        &self,
        secrets: SecretStore,
        agent_service: miaominal_services::AgentService,
        local_vault_status: LocalVaultStatus,
        cx: &mut Context<AppView>,
    ) {
        self.forwarding.update(cx, |controller, cx| {
            controller.credentials_changed(secrets.clone(), local_vault_status, cx);
        });
        self.hosts.update(cx, |controller, cx| {
            controller.credentials_changed(secrets.clone(), local_vault_status, cx);
        });
        self.session.update(cx, |controller, cx| {
            controller.credentials_changed(secrets.clone(), local_vault_status, cx)
        });
        self.agent.update(cx, |controller, cx| {
            controller.credentials_changed(secrets.clone(), agent_service, local_vault_status, cx)
        });
        self.sftp.update(cx, |controller, cx| {
            controller.credentials_changed(secrets.clone(), cx)
        });
        self.settings.update(cx, |controller, cx| {
            controller.update_credentials(secrets.clone(), local_vault_status, cx)
        });
        self.keychain.update(cx, |controller, cx| {
            controller.update_credentials(secrets, local_vault_status, cx);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_key_delete_clears_only_matching_profile_references() {
        let mut matching = SessionProfile::blank("matching", 1);
        matching.auth_method = Some(AuthMethod::ManagedKey);
        matching.managed_key_id = "key-1".to_string();
        let mut unrelated = SessionProfile::blank("unrelated", 2);
        unrelated.auth_method = Some(AuthMethod::ManagedKey);
        unrelated.managed_key_id = "key-2".to_string();
        let mut profiles = vec![matching, unrelated];

        assert!(clear_managed_key_profile_references(&mut profiles, "key-1"));
        assert!(profiles[0].managed_key_id.is_empty());
        assert_eq!(profiles[0].auth_method, Some(AuthMethod::Password));
        assert_eq!(profiles[1].managed_key_id, "key-2");
        assert_eq!(profiles[1].auth_method, Some(AuthMethod::ManagedKey));
    }

    #[test]
    fn managed_key_delete_reports_no_change_for_unknown_key() {
        let mut profile = SessionProfile::blank("profile", 1);
        profile.managed_key_id = "key-1".to_string();

        assert!(!clear_managed_key_profile_references(
            std::slice::from_mut(&mut profile),
            "missing"
        ));
        assert_eq!(profile.managed_key_id, "key-1");
    }

    #[test]
    fn interface_font_change_does_not_invalidate_terminal_metrics() {
        let previous = AppSettings::default();
        let mut next = previous.clone();
        next.font_family = "Segoe UI".into();

        assert!(!terminal_metrics_changed(&previous, &next));
    }

    #[test]
    fn terminal_font_change_invalidates_terminal_metrics() {
        let previous = AppSettings::default();
        let mut next = previous.clone();
        next.terminal_font_family = "JetBrains Mono".into();

        assert!(terminal_metrics_changed(&previous, &next));
    }

    #[test]
    fn interface_font_size_change_does_not_invalidate_terminal_metrics() {
        let previous = AppSettings::default();
        let mut next = previous.clone();
        next.interface_font_size += miaominal_settings::STEP;

        assert!(!terminal_metrics_changed(&previous, &next));
    }

    #[test]
    fn terminal_font_size_change_invalidates_terminal_metrics() {
        let previous = AppSettings::default();
        let mut next = previous.clone();
        next.terminal_font_size += miaominal_settings::STEP;

        assert!(terminal_metrics_changed(&previous, &next));
    }
}
