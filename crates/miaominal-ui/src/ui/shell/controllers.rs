use std::{cell::RefCell, rc::Rc};

use gpui::{App, AppContext as _, Context, Entity, Subscription, Window};
use gpui_component::WindowExt as _;

use super::{
    AppView, AuthMethod, DialogOverlaySnapshot, LocalVaultPassphrasePopupMode, LocalVaultStatus,
    ManagedKeySelectItem, PaneId, SecretRevealTarget, SessionProfile, SftpBrowserSide,
    SidebarSection, SplitDirection, SyncProvider, TabId, TabPlacement, TabState,
    WorkspaceTerminalInputExt, ai_provider_select_options, terminal_cell_width_default,
    terminal_line_height_default,
};
use crate::ui::i18n;
use miaominal_secrets::SecretStore;
use miaominal_services::SyncReloadResult;

mod agent;
mod keychain;
mod local_vault_root;
mod root;
mod session;
mod settings;
mod sftp;

pub(in crate::ui::shell) use agent::{
    AgentApprovedToolTask, AgentContinuationPreparation, AgentController, AgentControllerArgs,
    AgentExecMode, AgentFinishStreamOutcome, AgentPromptDraft, AgentPromptDraftOutcome,
    AgentPromptRequestPreparation, AgentStreamTask, AgentStreamTaskRequest,
    AgentToolApprovalCommit, AgentToolContinuation, ChatPanelView, PendingChatSessionDeleteState,
    PendingChatSessionRenameState, PromptHistoryDirection, SessionAgentBackgroundNotificationKind,
    SessionAgentMessage, SessionAgentMessageMotion, SessionAgentMessageRole,
    SessionAgentPanelDragState, SessionAgentTargetCandidate, SessionAgentToolCall,
    SessionAgentToolStatus, split_message_into_blocks,
};
#[cfg(test)]
pub(in crate::ui::shell) use agent::{
    SessionAgentExecutionContext, SessionAgentState, chat_record_from_session_agent_message,
    restored_tool_status_and_note, session_agent_message_from_record, tool_status_as_str,
};
pub(in crate::ui::shell) use keychain::{
    KeychainController, KeychainControllerArgs, KeychainEditorMode, KeychainPageView,
    PendingManagedKeyDeleteState, PendingManagedKeyRenameState,
};
pub(in crate::ui::shell) use local_vault_root::LocalVaultRootExt;
pub(in crate::ui::shell) use session::{
    ClosedSessionTabState, MonitorChartPoint, PendingKnownHostDeleteState,
    PendingPortForwardRuleDeleteState, PendingProfileDeleteState, PendingProfileImportResultState,
    PendingSnippetDeleteState, PortForwardSessionStart, SessionConnectionState, SessionController,
    SessionControllerArgs, SessionEventOutcome, SessionEventTabRemoval, SessionFailureStatus,
    SessionNotificationTone, SessionPortSession, SessionPortSnapshot, SessionPurpose,
    SessionQueryPort, SessionSidePanelView, SessionTabState, SessionTerminalPort,
    SessionTerminalTarget, TerminalLease, TerminalLeaseError, TerminalLeaseGrant,
    TrustedHostFilter,
};
#[cfg(test)]
pub(in crate::ui::shell) use session::{SESSION_MONITOR_HISTORY_LIMIT, SessionMonitoringState};
pub(in crate::ui::shell) use settings::{
    AiProviderSaveDraft, KeyBindingSlot, LocalVaultActionRequest, LocalVaultChangePassphraseResult,
    LocalVaultEnableResult, LocalVaultOperationResult, LocalVaultUnlockResult, OnboardingState,
    OnboardingStep, OnboardingStepTransition, OnboardingStepTransitionPhase,
    PendingAiProviderPopupState, PendingLocalDataResetConfirmState,
    PendingLocalDataResetConfirmationPopupState, PendingLocalVaultDisableConfirmState,
    PendingSyncDirectionState, PendingSyncPassphraseClearConfirmPopupState,
    PendingSyncPassphrasePopupState, PendingSyncProviderConfigPopupState,
    PendingSyncPullConfirmState, PendingWebSearchConfigPopupState, SettingsController,
    SettingsControllerArgs, SettingsForms, SyncProviderConfigSaveDraft, SyncPullConfirmReason,
    WebSearchSaveDraft,
};
pub(in crate::ui::shell) use sftp::{
    LocalSftpEntry, SessionSftpProgressCenterDragState, SftpController, SftpControllerArgs,
    SftpDragSelectionState, SftpPromptKind, SftpPromptState, SftpSplitDivider, SftpSplitDragState,
    SftpTabState, SftpTransferChildStatus, SftpTransferRow, SftpTransferStatus,
};

#[derive(Clone, Debug)]
pub(in crate::ui::shell) enum AppCommand {
    OpenTab(TabOpenRequest),
    CloseTab(TabId),
    TabStatusChanged {
        tab_id: TabId,
        status: String,
    },
    TerminalScrolledToBottom(TabId),
    TerminalFocusReportingRequested,
    WindowActivationChanged {
        active: bool,
    },
    SaveProfileRequested(Box<SessionProfile>),
    SaveSnippetRequested(Box<miaominal_core::snippet::SnippetRecord>),
    ImportProfilesRequested(miaominal_core::profile::ImportSourceKind),
    Feedback(String),
    OverlayDismissed(DialogOverlaySnapshot),
    VaultUnlockRequested(Option<DeferredAppCommand>),
    CredentialsChanged,
    SyncReloaded(Box<SyncReloadResult>),
    ManagedKeysChanged(ManagedKeysChange),
    SessionMonitoringPreferenceChanged(bool),
    SessionEventApplied {
        tab_id: TabId,
        outcome: SessionEventOutcome,
    },
    SidebarSectionRequested(SidebarSection),
    EnsureSessionSftpRequested(TabId),
    TerminalMenuRequested {
        pane_id: Option<PaneId>,
        command: TerminalMenuCommand,
    },
    VaultActionRequested(LocalVaultActionRequest),
    LocalDataResetRequested,
    PersistSftpBrowserHiddenColumns {
        side: SftpBrowserSide,
        hidden_columns: Vec<usize>,
    },
    LocaleRefresh,
    RebuildApplication,
}

impl AppCommand {
    pub(in crate::ui::shell) fn vault_unlock(command: DeferredAppCommand) -> Self {
        Self::VaultUnlockRequested(Some(command))
    }

    pub(in crate::ui::shell) fn vault_unlock_prompt() -> Self {
        Self::VaultUnlockRequested(None)
    }
}

#[derive(Clone, Copy, Debug)]
pub(in crate::ui::shell) enum TerminalMenuCommand {
    Copy,
    Paste,
    Split(SplitDirection),
    OpenSftp,
    ClosePane,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) enum TabOpenRequest {
    NewProfileEditor,
    ProfileEditor {
        profile_id: String,
        open_hosts_tab: bool,
    },
    ProfileConnectionTest {
        profile: Box<SessionProfile>,
    },
    Session {
        profile_id: String,
    },
    Sftp {
        profile_id: String,
        owner: Option<TabId>,
    },
    PortForwarding {
        profile_id: String,
        rule_id: String,
    },
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) enum DeferredAppCommand {
    Session(SessionDeferredCommand),
    Agent(AgentDeferredCommand),
    Sftp(SftpDeferredCommand),
    Settings(SettingsDeferredCommand),
    Keychain(KeychainDeferredCommand),
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) enum SessionDeferredCommand {
    OpenProfile(Box<SessionProfile>),
    SaveProfile,
    SavePortForwardRule,
    SaveSnippet,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) enum AgentDeferredCommand {
    ResumeRequest,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) enum SftpDeferredCommand {
    OpenProfile {
        profile_id: String,
        owner: Option<TabId>,
    },
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) enum SettingsDeferredCommand {
    ResumeSync,
    SaveSyncPassphrase(String),
    OpenSyncProviderConfig(SyncProvider),
    SaveSyncProviderConfig(SyncProviderConfigSaveDraft),
    OpenAiProvider(String),
    SaveAiProvider(AiProviderSaveDraft),
    OpenWebSearchConfig,
    SaveWebSearch(WebSearchSaveDraft),
    ClearSyncPassphrase,
    RevealSecret(SecretRevealTarget),
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) enum KeychainDeferredCommand {
    ImportManagedKey,
    DeployManagedKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum ManagedKeysChange {
    Reloaded,
    Added,
    Removed { key_id: String },
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SyncReloadDomain {
    Settings,
    Sessions,
    Snippets,
    ManagedKeys,
}

const fn sync_reload_domains() -> [SyncReloadDomain; 4] {
    [
        SyncReloadDomain::Settings,
        SyncReloadDomain::Sessions,
        SyncReloadDomain::Snippets,
        SyncReloadDomain::ManagedKeys,
    ]
}

pub(in crate::ui::shell) struct ControllerSet {
    pub session: Entity<SessionController>,
    pub agent: Entity<AgentController>,
    pub sftp: Entity<SftpController>,
    pub settings: Entity<SettingsController>,
    pub keychain: Entity<KeychainController>,
}

impl ControllerSet {
    pub(in crate::ui::shell) fn new(
        session: SessionControllerArgs,
        agent: AgentControllerArgs,
        sftp: SftpControllerArgs,
        keychain: KeychainControllerArgs,
        settings: SettingsControllerArgs,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Self {
        let session = cx.new(|cx| SessionController::new(session, window, cx));
        let session_query = session.read(cx).query_port();
        let session_terminal = session.read(cx).terminal_port();
        Self {
            session,
            agent: cx.new(|cx| {
                AgentController::new(agent, session_query.clone(), session_terminal, window, cx)
            }),
            sftp: cx.new(|cx| SftpController::new(sftp, session_query.clone(), window, cx)),
            settings: cx.new(|cx| SettingsController::new(settings, window, cx)),
            keychain: cx.new(|cx| KeychainController::new(keychain, session_query, window, cx)),
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
                subscriptions.push(cx.observe($controller, |_this, _controller, cx| {
                    cx.notify();
                }));
                subscriptions.push(cx.subscribe_in(
                    $controller,
                    window,
                    |this, _controller, command: &AppCommand, window, cx| {
                        this.handle_app_command_in_window(command, window, cx);
                    },
                ));
            }};
        }

        connect!(&self.session);
        connect!(&self.agent);
        connect!(&self.sftp);
        let observed_settings = Rc::new(RefCell::new(self.settings.read(cx).settings().clone()));
        subscriptions.push(cx.observe(&self.settings, move |this, controller, cx| {
            let next_settings = controller.read(cx).settings().clone();
            let mut previous_settings = observed_settings.borrow_mut();
            let terminal_metrics_changed = previous_settings.font_family
                != next_settings.font_family
                || previous_settings.font_fallbacks != next_settings.font_fallbacks
                || previous_settings.font_size != next_settings.font_size
                || previous_settings.line_height != next_settings.line_height;
            *previous_settings = next_settings;
            drop(previous_settings);
            if terminal_metrics_changed {
                this.invalidate_terminal_metrics();
            }
            cx.notify();
        }));
        subscriptions.push(cx.subscribe_in(
            &self.settings,
            window,
            |this, _controller, command, window, cx| {
                this.handle_app_command_in_window(command, window, cx);
            },
        ));
        subscriptions.push(cx.observe(&self.keychain, |this, controller, cx| {
            let status_message = controller.read(cx).status_message();
            if !status_message.is_empty() {
                this.shell.status_message = status_message.to_string();
            }
            cx.notify();
        }));
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
        local_vault_status: LocalVaultStatus,
        cx: &mut Context<AppView>,
    ) {
        self.session.update(cx, |controller, cx| {
            controller.credentials_changed(secrets.clone(), local_vault_status, cx)
        });
        self.agent.update(cx, |controller, cx| {
            controller.credentials_changed(secrets.clone(), local_vault_status, cx)
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
    fn sync_reload_is_distributed_to_every_owned_domain() {
        assert_eq!(
            sync_reload_domains(),
            [
                SyncReloadDomain::Settings,
                SyncReloadDomain::Sessions,
                SyncReloadDomain::Snippets,
                SyncReloadDomain::ManagedKeys,
            ]
        );
    }
}
