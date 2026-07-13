use crate::ui::assets::AppIcon;
use anyhow::{Context as AnyhowContext, Result, anyhow, bail};
use futures::StreamExt;
use gpui::{
    AnyElement, App, Bounds, ClickEvent, ClipboardItem, Context, CursorStyle, Div, ElementId,
    Entity, ExternalPaths, FocusHandle, Focusable, FontWeight, InteractiveElement, KeyDownEvent,
    KeyUpEvent, ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, Render, ScrollDelta, ScrollHandle, ScrollWheelEvent, SharedString, Stateful,
    Styled, Subscription, WeakEntity, Window, WindowControlArea, canvas, div, prelude::*, px, rgb,
};
use gpui_component::{
    Colorize, Sizable as _, VirtualListScrollHandle,
    button::{Button, ButtonVariants as _},
    color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
    h_flex,
    input::TabSize,
    input::{InputEvent, InputState},
    menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem},
    scroll::ScrollableElement,
    select::{SearchableVec, SelectEvent, SelectItem, SelectState},
    stepper::{Stepper, StepperItem},
    table::{Column, TableDelegate, TableEvent, TableState},
    v_flex,
};
use gpui_component::{Icon, IconName, Root};
use miaominal_core::keychain::ManagedKeyRecord;
use miaominal_core::profile::{
    AuthMethod, PortForwardKind, PortForwardRule, SessionEnvironmentVariable, SessionProfile,
    ShellType,
};
use miaominal_core::snippet::SnippetRecord;
use miaominal_settings::{self, KeyBinding};
use miaominal_sftp::{
    self, SftpCommandSender, SftpEntry, SftpEvent, SftpEventReceiver, SftpTransferChild,
    SftpTransferChildState, SftpTransferChildUpdate, TransferChildId, TransferDirection,
    TransferId,
};
use miaominal_ssh::{
    self, HostKeyDecision, HostKeyPrompt, KbiChallenge, SessionCommandSender, SessionEvent,
    SessionEventReceiver, SessionMonitorSnapshot,
};
use miaominal_storage::SettingsStore;
use miaominal_sync::engine::SyncEngine;
use miaominal_sync::{SyncProvider, SyncStatus};
use miaominal_terminal::{
    MouseEncoding, MouseProtocol, MouseReportButton, MouseReportKind, MouseReportModifiers,
    TerminalInputModes, TerminalScroll, TerminalState, encode_mouse_report, sanitize_paste,
    terminal_cell_width_default, terminal_line_height_default,
};
use tokio::runtime::Handle as TokioHandle;

mod actions;
#[path = "shell/app_view.rs"]
mod app_view;
mod bootstrap;
mod bootstrap_form_factory;
mod bootstrap_loaders;
mod bootstrap_subscriptions;
mod containers;
mod controllers;
mod forms;
mod layout;
mod metrics;
mod navigation;
mod pages;
mod panes;
mod render;
mod session_agent_stream_batch;
mod session_agent_view;
mod settings_labels;
mod sftp_browser;
mod state;
mod support;
mod system_file_icons;
mod workspace;

pub use app_view::AppView;

pub(crate) use crate::ui::components::{
    BasicDialogActionTone, BasicDialogHeaderAlignment, BasicDialogIcon,
    EDITOR_FOOTER_ACTION_HEIGHT, HintedInput, IconTileTone, SearchInputStyle, TextInputSurface,
    badge, basic_dialog_action_button, basic_dialog_panel, bottom_popup_panel, card_surface,
    editor_button_with_id, editor_footer_actions, fab_button, fab_icon_button, field_label,
    icon_button, icon_button_with_tooltip, icon_tile, list_item_card, md3_select,
    page_muted_icon_tile, page_primary_icon_tile, page_section_title, page_view_mode_toolbar_item,
    pill_label, search_filter_input, setting_field_with_reset_action,
    surface_secret_text_input_stack, surface_text_editor, surface_text_editor_stack,
    surface_text_input, surface_text_input_stack,
};
pub(crate) use crate::ui::utils::{
    format_byte_size, format_local_timestamp, truncate_with_ellipsis,
};
pub(in crate::ui::shell) use actions::{
    PromptHistoryDirection, SessionAgentTargetCandidate, ValidationFailure,
    ValidationNotificationKind, ai_provider_kind_label_key, ai_provider_select_options,
    web_search_provider_kind_label_key,
};
use containers::{AppDataState, AppViewSubscriptions, EditorOverlayState, PanelViewState};
use controllers::ControllerSet;
use forms::{
    ChatSearchForms, HostEditorForms, HostsForms, KeychainForms, PanelForms, PortForwardingForms,
    SettingsForms, SftpBrowserForms, SnippetsForms, TerminalSearchForms, TrustedHostsForms,
    WorkspaceAgentForms, WorkspaceForms, WorkspaceSnippetsForms,
};
pub(in crate::ui::shell) use forms::{KeyBindingSlot, SelectOption};
pub(in crate::ui::shell) use metrics::*;
pub(in crate::ui::shell) use miaominal_services::AppServices;
pub(in crate::ui::shell) use navigation::SidebarSection;
use panes::{PaneCloseAnimation, PaneSplitAnimation, PaneSplitAnimationKind, ParkedPane};
pub(in crate::ui::shell) use panes::{PaneId, TerminalHoveredLink, TerminalScrollbarDrag};
pub(in crate::ui::shell) use settings_labels::*;
pub(in crate::ui::shell) use sftp_browser::{
    SftpBrowserSelectionModifiers, SftpBrowserSide, SftpBrowserTableDelegate, SftpBrowserTableRow,
};
pub(in crate::ui::shell) use state::{
    AgentExecMode, ChatPanelView, ClosedSessionTabState, ClosedTabBundle, DialogOverlaySnapshot,
    DialogState, DraggedTab, ExitingDialogState, HostEditorEnvironmentVariableRow,
    InlineRenameState, LocalSftpEntry, MonitorChartPoint, OnboardingState, PanelState,
    PendingAiProviderPopupState, PendingChatSessionDeleteState, PendingChatSessionRenameState,
    PendingKnownHostDeleteState, PendingLocalVaultDisableConfirmState,
    PendingManagedKeyDeleteState, PendingPortForwardRuleDeleteState, PendingProfileDeleteState,
    PendingSnippetDeleteState, PendingSyncDirectionState, PendingSyncPassphrasePopupState,
    PendingSyncProviderConfigPopupState, PendingSyncPullConfirmState,
    PendingWebSearchConfigPopupState, SecretVisibilityState, SessionAgentAutoScrollState,
    SessionAgentMessage, SessionAgentMessageMotion, SessionAgentMessageRole,
    SessionAgentPanelDragState, SessionAgentState, SessionAgentToolCall, SessionAgentToolStatus,
    SessionConnectionState, SessionMonitoringState, SessionPurpose,
    SessionSftpProgressCenterDragState, SessionSidePanelView, SessionTabState,
    SftpDragSelectionState, SftpEditSession, SftpPromptKind, SftpPromptState, SftpSplitDivider,
    SftpSplitDragState, SftpTabState, SftpTransferChildStatus, SftpTransferRow, SftpTransferStatus,
    ShellState, SyncProviderConfigSaveOperation, SyncPullConfirmReason, SyncUiState, TabKind,
    TabState, TrustedHostFilter, WorkspaceState, split_message_into_blocks,
    trailing_at_mention_query,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};
pub(in crate::ui::shell) use system_file_icons::render_system_file_icon;
use workspace::{PaneLayout, SplitAxis, SplitDirection, TabWorkspaceState};

use support::{
    CONTAINER_TRANSITION_DURATION, OVERLAY_ENTER_DURATION, TerminalKeyAction, TerminalKeyEvent,
    TerminalKeyPhase, TerminalScrollbarMetrics, classify_terminal_key,
    container_transition_animation, list_enter_animation, new_input_state, overlay_enter_animation,
    render_basic_dialog, render_basic_dialog_with_config, render_bottom_popup,
    render_terminal_canvas_for_pane, set_code_editor_input_placeholder, set_input_placeholder,
    set_input_value, short_feedback_animation, terminal_cell_width, terminal_line_height,
    terminal_scrollbar_metrics, terminal_scrollbar_offset_for_pointer,
};
pub(in crate::ui::shell) use support::{GroupAccentPalette, group_accent_palette};

pub(in crate::ui::shell) fn color_with_alpha(color: u32, alpha: u8) -> gpui::Rgba {
    gpui::rgba(((color & 0x00ff_ffff) << 8) | alpha as u32)
}

const APP_TITLE: &str = "Miaominal";
pub(in crate::ui::shell) const SESSION_MONITOR_PANEL_WIDTH: f32 = 356.0;
pub(in crate::ui::shell) const SESSION_SFTP_PROGRESS_DEFAULT_FLEX: f32 = 0.26;
pub(in crate::ui::shell) const TERMINAL_SCROLLBAR_IDLE_HIDE_DELAY: Duration =
    Duration::from_millis(1200);
pub(in crate::ui::shell) const KEYCHAIN_DEPLOY_DEFAULT_LOCATION: &str = ".ssh";
pub(in crate::ui::shell) const KEYCHAIN_DEPLOY_DEFAULT_FILENAME: &str = "authorized_keys";
pub(in crate::ui::shell) const KEYCHAIN_DEPLOY_DEFAULT_COMMAND: &str = "if test ! -e $1;\nthen mkdir -p $1;\nchmod 700 $1;\nfi;\nif test ! -e \"$1/$2\";\nthen touch \"$1/$2\";\nchmod 600 \"$1/$2\";\nfi;\necho $3 >> \"$1/$2\";";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::shell) enum ProfileViewMode {
    Grid,
    List,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::shell) enum KeychainPageView {
    ManagedKeys,
    AgentIdentities,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum KeychainEditorMode {
    Import,
    Deploy,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::shell) enum PrimaryViewKind {
    Sidebar(SidebarSection),
    Terminal(usize),
    Sftp(usize),
}

#[derive(Clone, Copy)]
pub(in crate::ui::shell) struct PrimaryViewTransition {
    pub(in crate::ui::shell) from: PrimaryViewKind,
    pub(in crate::ui::shell) to: PrimaryViewKind,
    pub(in crate::ui::shell) started_at: Instant,
    pub(in crate::ui::shell) duration: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum TopbarTabVisualKind {
    Hosts,
    Session,
    Sftp,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) struct TopbarTabSnapshot {
    pub(in crate::ui::shell) tab_id: usize,
    pub(in crate::ui::shell) visible_index: usize,
    pub(in crate::ui::shell) title: String,
    pub(in crate::ui::shell) kind: TopbarTabVisualKind,
    pub(in crate::ui::shell) status_color: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::ui::shell) struct TopbarTabEnterTransition {
    pub(in crate::ui::shell) tab_id: usize,
    pub(in crate::ui::shell) started_at: Instant,
    pub(in crate::ui::shell) duration: Duration,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) struct TopbarTabExitTransition {
    pub(in crate::ui::shell) snapshot: TopbarTabSnapshot,
    pub(in crate::ui::shell) started_at: Instant,
    pub(in crate::ui::shell) duration: Duration,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::ui::shell) struct TopbarActiveTabTransition {
    pub(in crate::ui::shell) from_tab_id: Option<usize>,
    pub(in crate::ui::shell) to_tab_id: Option<usize>,
    pub(in crate::ui::shell) started_at: Instant,
    pub(in crate::ui::shell) duration: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum PageEditorSidebarKind {
    Hosts,
    PortForwarding,
    Snippets,
    Keychain,
    KnownHosts,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum PageEditorSidebarTransitionPhase {
    Entering,
    Exiting,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::ui::shell) struct PageEditorSidebarTransition {
    pub(in crate::ui::shell) kind: PageEditorSidebarKind,
    pub(in crate::ui::shell) phase: PageEditorSidebarTransitionPhase,
    pub(in crate::ui::shell) started_at: Instant,
    pub(in crate::ui::shell) duration: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum OnboardingStep {
    Welcome,
    Preferences,
    Import,
    Finish,
}

impl OnboardingStep {
    pub(in crate::ui::shell) const ALL: [Self; 4] =
        [Self::Welcome, Self::Preferences, Self::Import, Self::Finish];

    pub(in crate::ui::shell) const fn index(self) -> usize {
        match self {
            Self::Welcome => 0,
            Self::Preferences => 1,
            Self::Import => 2,
            Self::Finish => 3,
        }
    }

    pub(in crate::ui::shell) fn next(self) -> Option<Self> {
        Self::ALL.get(self.index() + 1).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum OnboardingStepTransitionPhase {
    Exiting,
    Entering,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum LocalVaultStatus {
    Disabled,
    Locked,
    Unlocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum LocalVaultPassphrasePopupMode {
    PrimaryAction,
    ChangePassphrase,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum SecretRevealTarget {
    SyncGithubToken,
    SyncWebdavPassword,
    HostPassword,
    SyncPassphrase,
    SyncPassphraseConfirmation,
    LocalVaultPassphrase,
    LocalVaultPassphraseConfirmation,
    AiProviderApiKey(String),
    WebSearchApiKey,
}

impl SecretRevealTarget {
    pub(in crate::ui::shell) fn uses_stored_secret(&self) -> bool {
        matches!(
            self,
            Self::SyncGithubToken
                | Self::SyncWebdavPassword
                | Self::HostPassword
                | Self::AiProviderApiKey(_)
                | Self::WebSearchApiKey
        )
    }
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) enum PendingLocalVaultUnlockAction {
    OpenSession(Box<SessionProfile>),
    OpenSyncNow,
    DeployManagedKey,
    SaveProfile,
    ImportManagedKey,
    SavePortForwardRule,
    SaveSnippet,
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
pub(in crate::ui::shell) enum SyncProviderConfigSaveDraft {
    GithubGist {
        token: String,
        gist_id: Option<String>,
    },
    WebDav {
        url: String,
        username: String,
        password: String,
    },
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) struct AiProviderSaveDraft {
    pub(in crate::ui::shell) provider: miaominal_settings::AiProviderConfig,
    pub(in crate::ui::shell) api_key: String,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) struct WebSearchSaveDraft {
    pub(in crate::ui::shell) config: miaominal_settings::WebSearchConfig,
    pub(in crate::ui::shell) api_key: String,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::ui::shell) struct OnboardingStepTransition {
    pub(in crate::ui::shell) phase: OnboardingStepTransitionPhase,
    pub(in crate::ui::shell) started_at: Instant,
    pub(in crate::ui::shell) duration: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum WorkspaceSidePanelTransitionPhase {
    Entering,
    Exiting,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::ui::shell) struct WorkspaceSidePanelTransition {
    pub(in crate::ui::shell) phase: WorkspaceSidePanelTransitionPhase,
    pub(in crate::ui::shell) started_at: Instant,
    pub(in crate::ui::shell) duration: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum SftpProgressCenterTransitionPhase {
    Entering,
    Exiting,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::ui::shell) struct SftpProgressCenterTransition {
    pub(in crate::ui::shell) phase: SftpProgressCenterTransitionPhase,
    pub(in crate::ui::shell) started_at: Instant,
    pub(in crate::ui::shell) duration: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) struct ForwardProfileSelectItem {
    id: String,
    title: SharedString,
    summary: SharedString,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) struct ProxyJumpCandidateSelectItem {
    id: String,
    title: SharedString,
    summary: SharedString,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) struct ManagedKeySelectItem {
    id: String,
    title: SharedString,
    summary: SharedString,
}

impl ProxyJumpCandidateSelectItem {
    pub(in crate::ui::shell) fn new(profile: &SessionProfile) -> Self {
        Self {
            id: profile.id.clone(),
            title: SharedString::from(profile.name.clone()),
            summary: SharedString::from(profile.summary()),
        }
    }
}

impl ForwardProfileSelectItem {
    pub(in crate::ui::shell) fn new(profile: &SessionProfile) -> Self {
        Self {
            id: profile.id.clone(),
            title: SharedString::from(profile.name.clone()),
            summary: SharedString::from(profile.summary()),
        }
    }
}

impl ManagedKeySelectItem {
    pub(in crate::ui::shell) fn new(key: &ManagedKeyRecord) -> Self {
        Self {
            id: key.id.clone(),
            title: SharedString::from(key.name.clone()),
            summary: SharedString::from(format!("{}  {}", key.id, key.algorithm)),
        }
    }
}

impl SelectItem for ForwardProfileSelectItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let roles = miaominal_settings::current_theme().material.roles;

        v_flex()
            .w_full()
            .gap_1()
            .child(
                div()
                    .text_size(miaominal_settings::FontSize::Subheading.scaled())
                    .text_color(rgb(roles.on_surface))
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .text_size(miaominal_settings::FontSize::Body.scaled())
                    .text_color(rgb(roles.on_surface_variant))
                    .child(self.summary.clone()),
            )
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }

    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return true;
        }

        format!("{} {}", self.title.as_ref(), self.summary.as_ref())
            .to_ascii_lowercase()
            .contains(&query)
    }
}

impl SelectItem for ProxyJumpCandidateSelectItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let roles = miaominal_settings::current_theme().material.roles;

        v_flex()
            .w_full()
            .gap_1()
            .child(
                div()
                    .text_size(miaominal_settings::FontSize::Subheading.scaled())
                    .text_color(rgb(roles.on_surface))
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .text_size(miaominal_settings::FontSize::Body.scaled())
                    .text_color(rgb(roles.on_surface_variant))
                    .child(self.summary.clone()),
            )
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }

    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return true;
        }

        format!("{} {}", self.title.as_ref(), self.summary.as_ref())
            .to_ascii_lowercase()
            .contains(&query)
    }
}

impl SelectItem for ManagedKeySelectItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let roles = miaominal_settings::current_theme().material.roles;

        v_flex()
            .w_full()
            .gap_1()
            .child(
                div()
                    .text_size(miaominal_settings::FontSize::Subheading.scaled())
                    .text_color(rgb(roles.on_surface))
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .text_size(miaominal_settings::FontSize::Body.scaled())
                    .text_color(rgb(roles.on_surface_variant))
                    .child(self.summary.clone()),
            )
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }

    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return true;
        }

        format!("{} {}", self.title.as_ref(), self.summary.as_ref())
            .to_ascii_lowercase()
            .contains(&query)
    }
}
