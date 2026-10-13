use crate::ui::contracts::navigation::{PageEditorSidebarKind, SidebarSection};
use crate::ui::contracts::security::SecretRevealTarget;
use crate::ui::contracts::session::{SessionEventOutcome, SessionNotificationRequest};
use crate::ui::contracts::snippets::SnippetSaveSubmission;
use crate::ui::contracts::workspace::{PaneId, SplitDirection, TabId};
use miaominal_core::known_host::KnownHostEntry;
use miaominal_core::profile::SessionProfile;
use miaominal_core::snippet::SnippetRecord;
use miaominal_services::SyncTaskResult;
use miaominal_sync::SyncProvider;
// Transitional Settings/SFTP/overlay types switch to their pure/opaque contracts in Stage 4.
use crate::ui::shell::controllers::settings::{
    AiProviderSaveDraft, LocalVaultActionRequest, ProxySaveDraft, SyncProviderConfigSaveDraft,
    WebSearchSaveDraft,
};
use crate::ui::shell::sftp_browser::SftpBrowserSide;
use crate::ui::shell::state::DialogOverlaySnapshot;
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
    OverlayDismissed(DialogOverlaySnapshot),
    VaultUnlockRequested(Option<DeferredAppCommand>),
    CredentialsChanged,
    ManualSyncCompleted(Box<SyncTaskResult>),
    ProxiesChanged(Vec<miaominal_core::proxy::ProxyProfile>),
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
    SaveSyncPassphrase(String),
    OpenSyncProviderConfig(SyncProvider),
    SaveSyncProviderConfig(SyncProviderConfigSaveDraft),
    OpenAiProvider(String),
    SaveAiProvider(AiProviderSaveDraft),
    OpenWebSearchConfig,
    SaveWebSearch(WebSearchSaveDraft),
    SaveProxy(ProxySaveDraft),
    ClearSyncPassphrase,
    RevealSecret(SecretRevealTarget),
}

#[derive(Clone, Debug)]
pub(in crate::ui) enum KeychainDeferredCommand {
    ImportManagedKey,
    DeployManagedKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui) enum ManagedKeysChange {
    Reloaded,
    Added,
    Removed { key_id: String },
}
