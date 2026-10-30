use crate::ui::contracts::keychain::{ManagedKeyDeployRequest, ManagedKeyImportRequest};
use crate::ui::contracts::navigation::{PageEditorSidebarKind, SidebarSection};
use crate::ui::contracts::overlay::OverlaySnapshot;
use crate::ui::contracts::preferences::SftpBrowserSide;
use crate::ui::contracts::session::{SessionEventOutcome, SessionNotificationRequest};
use crate::ui::contracts::settings::{
    AiProviderSaveDraft, LocalVaultActionRequest, ProxySaveDraft, SecretRevealRequest,
    SyncPassphraseSaveDraft, SyncProviderConfigSaveDraft, WebSearchSaveDraft,
};
use crate::ui::contracts::workspace::{PaneId, SplitDirection, TabId};
use miaominal_core::profile::SessionProfile;
use miaominal_services::SyncTaskResult;
use miaominal_sync::SyncProvider;
#[derive(Clone, Debug)]
pub(in crate::ui) enum AppCommand {
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
    SaveSnippetRequested(Box<crate::ui::contracts::snippets::SnippetSaveSubmission>),
    ImportProfilesRequested(miaominal_core::profile::ImportSourceKind),
    Feedback(String),
    KnownHostsChanged,
    OverlayDismissed(OverlaySnapshot),
    VaultUnlockRequested(Option<DeferredAppCommand>),
    CredentialsChanged,
    ManualSyncCompleted(Box<SyncTaskResult>),
    ProxiesChanged,
    ManagedKeysChanged(ManagedKeysChange),
    SessionMonitoringPreferenceChanged(bool),
    AgentModePreferenceChanged(miaominal_settings::AiAgentMode),
    SessionNotificationRequested(SessionNotificationRequest),
    SessionEventApplied {
        tab_id: TabId,
        outcome: SessionEventOutcome,
    },
    SidebarSectionRequested(SidebarSection),
    SidebarEditorStateChanged(Option<PageEditorSidebarKind>),
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
    PersistRemotePathFavorites {
        profile_id: String,
        favorites: Vec<String>,
    },
    LocaleRefresh,
    RebuildApplication,
    ChatHistoryReopenRequested,
}

impl AppCommand {
    pub(in crate::ui) fn vault_unlock(command: DeferredAppCommand) -> Self {
        Self::VaultUnlockRequested(Some(command))
    }

    pub(in crate::ui) fn vault_unlock_prompt() -> Self {
        Self::VaultUnlockRequested(None)
    }
}

#[derive(Clone, Copy, Debug)]
pub(in crate::ui) enum TerminalMenuCommand {
    Copy,
    Paste,
    Split(SplitDirection),
    OpenSftp,
    ClosePane,
}

#[derive(Clone, Debug)]
pub(in crate::ui) enum TabOpenRequest {
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
    Rdp {
        profile_id: String,
    },
    RdpConnect {
        profile_id: String,
        options: miaominal_services::RdpLaunchOptions,
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
pub(in crate::ui) enum DeferredAppCommand {
    SaveHostProfile,
    RevealHostPassword(u64),
    SaveSnippet,
    SavePortForwardRule,
    Session(SessionDeferredCommand),
    Agent(AgentDeferredCommand),
    Sftp(SftpDeferredCommand),
    Settings(SettingsDeferredCommand),
    Keychain(KeychainDeferredCommand),
}

#[derive(Clone, Debug)]
pub(in crate::ui) enum SessionDeferredCommand {
    OpenProfile(Box<SessionProfile>),
    RestartRdp(TabId),
    OpenRdp {
        profile_id: String,
        options: miaominal_services::RdpLaunchOptions,
    },
}

#[derive(Clone, Debug)]
pub(in crate::ui) enum AgentDeferredCommand {
    ResumeRequest,
}

#[derive(Clone, Debug)]
pub(in crate::ui) enum SftpDeferredCommand {
    OpenProfile {
        profile_id: String,
        owner: Option<TabId>,
    },
}

#[derive(Clone, Debug)]
pub(in crate::ui) enum SettingsDeferredCommand {
    CheckAutoSync(bool),
    ResumeSync,
    SaveSyncPassphrase(SyncPassphraseSaveDraft),
    OpenSyncProviderConfig(SyncProvider),
    SaveSyncProviderConfig(SyncProviderConfigSaveDraft),
    OpenAiProvider(String),
    SaveAiProvider(AiProviderSaveDraft),
    OpenWebSearchConfig,
    SaveWebSearch(WebSearchSaveDraft),
    SaveProxy(ProxySaveDraft),
    ClearSyncPassphrase(u64),
    RevealSecret(SecretRevealRequest),
}

#[derive(Clone, Debug)]
pub(in crate::ui) enum KeychainDeferredCommand {
    ImportManagedKey(ManagedKeyImportRequest),
    DeployManagedKey(Box<ManagedKeyDeployRequest>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui) enum ManagedKeysChange {
    Reloaded,
    Added,
    Removed { key_id: String },
}
