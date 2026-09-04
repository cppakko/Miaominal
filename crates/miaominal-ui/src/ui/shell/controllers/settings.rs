use super::{AppCommand, SessionQueryPort};
use crate::ui::i18n;
use crate::ui::shell::actions::ai_provider_kind_chat_supported;
use crate::ui::shell::{
    AppNotification, AppNotificationPriority, AppNotificationTone, DialogOverlaySnapshot,
    LocalVaultPassphrasePopupMode, LocalVaultStatus, SecretRevealTarget, SelectOption,
    SettingsDestination, SftpBrowserSide, SidebarSection, ai_provider_kind_label_key,
    ai_provider_select_options, bridge_security_level_label, last_tab_close_behavior_label,
    local_vault_auto_lock_duration_label, localized_profile_import_source_label,
    localized_secret_placeholder, monitor_history_duration_label, new_input_state,
    set_input_placeholder, set_input_value, theme_id_label, web_search_endpoint_placeholder,
    web_search_provider_kind_label_key, window_close_behavior_label,
};
use crate::ui::shell::{
    BridgeSecurityNotificationAction, BridgeSecurityNotificationKey,
    BridgeSecurityNotificationModel, BridgeSecurityNotificationState,
    BridgeSecurityNotificationView, bridge_security_notification_window_options,
};
use anyhow::{Result, anyhow};
use gpui_kit::component::{
    Colorize, IndexPath,
    color_picker::{ColorPickerEvent, ColorPickerState},
    input::{InputEvent, InputState},
    select::{SelectEvent, SelectState},
};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle,
    ScrollStrategy, Subscription, UniformListScrollHandle, Window, WindowHandle, rgb,
};
use miaominal_core::profile::ImportSourceKind;
use miaominal_core::proxy::{ProxyAuthMode, ProxyProfile, ProxyProtocol};
use miaominal_core::ssh_bridge_security::{
    BridgeAuthorizationDecision, BridgePendingPhase, BridgeSecurityLevel, BridgeSecurityPolicy,
    BridgeSecuritySnapshot, DEFAULT_BRIDGE_APPROVAL_TIMEOUT_SECS,
};
use miaominal_secrets::{ProtectedPassphrase, SecretStore};
use miaominal_services::{
    LocalVaultPassphraseChangeOutcome, LocalVaultTransition, OpenSshIntegrationService,
    ProxyPasswordUpdate, ProxyService, SettingsService, SshBridgeService, SyncExecutor,
};
use miaominal_settings::{
    AiProviderKind, AiReasoningEffort, AppLanguage, AppSettings, KeyBinding, LastTabCloseBehavior,
    LocalVaultAutoLockDuration, MonitorHistoryDuration, OpenSshIntegrationMode,
    TerminalKeyBindings, TerminalRightClickBehavior, ThemeId, WebSearchProviderKind,
    WindowCloseBehavior,
};
use miaominal_ssh::{SshBridgeStatus, SshBridgeSyncResult};
use miaominal_storage::{ProxyStore, SettingsStore};
use miaominal_sync::{SyncConfig, SyncProvider, SyncStatus, engine::SyncEngine};
use std::cell::Cell;
use std::time::{Duration, Instant};
use tokio::runtime::Handle as TokioHandle;

fn open_ssh_integration_mode_label(mode: OpenSshIntegrationMode) -> String {
    i18n::string(match mode {
        OpenSshIntegrationMode::Disabled => "settings.openssh_integration.modes.disabled",
        OpenSshIntegrationMode::Direct => "settings.openssh_integration.modes.direct",
        OpenSshIntegrationMode::Bridge => "settings.openssh_integration.modes.bridge",
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SshBridgeLifecycleAction {
    Enable,
    Disable,
}

struct BridgeApprovalNotification;

fn ssh_bridge_lifecycle_action(mode: OpenSshIntegrationMode) -> SshBridgeLifecycleAction {
    match mode {
        OpenSshIntegrationMode::Bridge => SshBridgeLifecycleAction::Enable,
        OpenSshIntegrationMode::Disabled | OpenSshIntegrationMode::Direct => {
            SshBridgeLifecycleAction::Disable
        }
    }
}

fn bridge_security_rank(level: BridgeSecurityLevel) -> u8 {
    match level {
        BridgeSecurityLevel::Standard => 0,
        BridgeSecurityLevel::RequireApproval { .. } => 1,
        BridgeSecurityLevel::RequireSystemAuth => 2,
    }
}

fn bridge_system_auth_decision(
    outcome: crate::ui::bridge_security_platform::SystemAuthVerification,
) -> BridgeAuthorizationDecision {
    use crate::ui::bridge_security_platform::SystemAuthVerification;

    match outcome {
        SystemAuthVerification::Verified => BridgeAuthorizationDecision::SystemAuthVerified,
        SystemAuthVerification::Canceled => BridgeAuthorizationDecision::SystemAuthCancelled,
        SystemAuthVerification::Unavailable => BridgeAuthorizationDecision::SystemAuthUnavailable,
        SystemAuthVerification::Busy
        | SystemAuthVerification::RetriesExhausted
        | SystemAuthVerification::Failed => BridgeAuthorizationDecision::SystemAuthFailed,
    }
}

fn bridge_security_display_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn apply_open_ssh_integration_mode_change(
    settings_store: &mut SettingsStore,
    integration_service: &OpenSshIntegrationService,
    mode: OpenSshIntegrationMode,
) -> Result<SshBridgeSyncResult> {
    let previous = settings_store.settings().open_ssh_integration_mode;
    let result = if mode == OpenSshIntegrationMode::Bridge {
        integration_service.defer_current_bridge_activation()
    } else {
        integration_service.set_mode(mode)?
    };
    let mut next = settings_store.settings().clone();
    next.open_ssh_integration_mode = mode;
    if let Err(error) = settings_store.replace(next) {
        return match integration_service.set_mode(previous) {
            Ok(_) => Err(anyhow!(
                "failed to persist OpenSSH integration mode; restored {previous:?}: {error}"
            )),
            Err(rollback_error) => Err(anyhow!(
                "failed to persist OpenSSH integration mode: {error}; failed to restore {previous:?}: {rollback_error}"
            )),
        };
    }
    Ok(result)
}

fn select_ai_provider_setting(settings: &mut AppSettings, provider_id: &str) -> bool {
    let is_available = settings.ai_providers.iter().any(|provider| {
        provider.id == provider_id
            && provider.enabled
            && ai_provider_kind_chat_supported(provider.kind)
    });
    if !is_available {
        return false;
    }
    settings.selected_ai_provider_id = Some(provider_id.to_string());
    true
}

fn set_ai_provider_reasoning_effort_setting(
    settings: &mut AppSettings,
    provider_id: &str,
    effort: AiReasoningEffort,
) -> bool {
    let Some(provider) = settings
        .ai_providers
        .iter_mut()
        .find(|provider| provider.id == provider_id && provider.enabled)
    else {
        return false;
    };
    provider.reasoning_effort = effort;
    true
}

fn proxy_management_select_options(proxies: &[ProxyProfile]) -> Vec<SelectOption<String>> {
    proxies
        .iter()
        .map(|proxy| SelectOption::new(proxy.id.clone(), proxy_management_label(proxy)))
        .collect()
}

fn proxy_management_label(proxy: &ProxyProfile) -> String {
    let protocol = match proxy.protocol {
        ProxyProtocol::Socks5 => "SOCKS5",
        ProxyProtocol::HttpConnect => "HTTP CONNECT",
    };
    format!(
        "{} · {protocol} · {}:{}",
        proxy.connection_label(),
        proxy.host,
        proxy.port
    )
}

mod ai_providers;
mod local_data_reset;
mod local_vault;
mod proxies;
mod secret_visibility;
mod sync;
mod web_search;

pub(in crate::ui::shell) use ai_providers::AiProviderSaveDraft;
pub(in crate::ui::shell) use local_vault::{
    LocalVaultActionRequest, LocalVaultChangePassphraseResult, LocalVaultEnableResult,
    LocalVaultOperationResult, LocalVaultUnlockResult,
};
pub(in crate::ui::shell) use proxies::ProxySaveDraft;
pub(in crate::ui::shell) use sync::{LocalVaultSyncSecretInputs, SyncProviderConfigSaveDraft};
pub(in crate::ui::shell) use web_search::WebSearchSaveDraft;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum OnboardingStep {
    Welcome,
    Preferences,
    Security,
    Import,
    Finish,
}

impl OnboardingStep {
    const STANDARD_STEPS: [Self; 4] =
        [Self::Welcome, Self::Preferences, Self::Import, Self::Finish];
    const PORTABLE_STEPS: [Self; 5] = [
        Self::Welcome,
        Self::Preferences,
        Self::Security,
        Self::Import,
        Self::Finish,
    ];

    pub(in crate::ui::shell) fn steps(portable: bool) -> &'static [Self] {
        if portable {
            &Self::PORTABLE_STEPS
        } else {
            &Self::STANDARD_STEPS
        }
    }

    pub(in crate::ui::shell) fn index(self, portable: bool) -> Option<usize> {
        Self::steps(portable).iter().position(|step| *step == self)
    }

    pub(in crate::ui::shell) fn next(self, portable: bool) -> Option<Self> {
        let index = self.index(portable)?;
        Self::steps(portable).get(index + 1).copied()
    }
}

fn portable_vault_is_required() -> bool {
    miaominal_paths::credential_policy().ok()
        == Some(miaominal_paths::CredentialPolicy::LocalVaultRequired)
}

fn onboarding_step_is_allowed(
    portable: bool,
    local_vault_status: LocalVaultStatus,
    step: OnboardingStep,
) -> bool {
    let Some(index) = step.index(portable) else {
        return false;
    };
    if !portable || local_vault_status == LocalVaultStatus::Unlocked {
        return true;
    }
    index <= OnboardingStep::Security.index(true).unwrap_or(usize::MAX)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum OnboardingStepTransitionPhase {
    Exiting,
    Entering,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::ui::shell) struct OnboardingStepTransition {
    pub(in crate::ui::shell) phase: OnboardingStepTransitionPhase,
    pub(in crate::ui::shell) started_at: Instant,
    pub(in crate::ui::shell) duration: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::shell) enum KeyBindingSlot {
    NextTab,
    CloseTab,
    ReopenTab,
    OpenSettings,
    Copy,
    Paste,
    Search,
    SplitRight,
    SplitDown,
    ClosePane,
}

impl KeyBindingSlot {
    fn label_key(self) -> &'static str {
        match self {
            Self::NextTab => "settings.key_bindings.slots.next_tab.label",
            Self::CloseTab => "settings.key_bindings.slots.close_tab.label",
            Self::ReopenTab => "settings.key_bindings.slots.reopen_tab.label",
            Self::OpenSettings => "settings.key_bindings.slots.open_settings.label",
            Self::Copy => "settings.key_bindings.slots.copy.label",
            Self::Paste => "settings.key_bindings.slots.paste.label",
            Self::Search => "settings.key_bindings.slots.search.label",
            Self::SplitRight => "settings.key_bindings.slots.split_right.label",
            Self::SplitDown => "settings.key_bindings.slots.split_down.label",
            Self::ClosePane => "settings.key_bindings.slots.close_pane.label",
        }
    }

    fn description_key(self) -> &'static str {
        match self {
            Self::NextTab => "settings.key_bindings.slots.next_tab.description",
            Self::CloseTab => "settings.key_bindings.slots.close_tab.description",
            Self::ReopenTab => "settings.key_bindings.slots.reopen_tab.description",
            Self::OpenSettings => "settings.key_bindings.slots.open_settings.description",
            Self::Copy => "settings.key_bindings.slots.copy.description",
            Self::Paste => "settings.key_bindings.slots.paste.description",
            Self::Search => "settings.key_bindings.slots.search.description",
            Self::SplitRight => "settings.key_bindings.slots.split_right.description",
            Self::SplitDown => "settings.key_bindings.slots.split_down.description",
            Self::ClosePane => "settings.key_bindings.slots.close_pane.description",
        }
    }

    pub(in crate::ui::shell) fn label(self) -> String {
        i18n::string(self.label_key())
    }

    pub(in crate::ui::shell) fn description(self) -> String {
        i18n::string(self.description_key())
    }
}

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingSyncDirectionState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::shell) enum SyncPullConfirmReason {
    Manual,
    RemoteNewer,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingSyncPullConfirmState {
    pub(in crate::ui::shell) reason: SyncPullConfirmReason,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingLocalVaultDisableConfirmState;

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingLocalDataResetConfirmState;

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingLocalDataResetConfirmationPopupState;

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingSyncPassphraseClearConfirmPopupState;

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingSyncPassphrasePopupState;

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingAiProviderPopupState;

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingWebSearchConfigPopupState;

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingSshBridgePolicyDowngradeState {
    pub(in crate::ui::shell) level: BridgeSecurityLevel,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingSyncProviderConfigPopupState {
    pub(in crate::ui::shell) provider: SyncProvider,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui::shell) struct PendingProxyConfigPopupState {
    pub(in crate::ui::shell) proxy_id: String,
    pub(in crate::ui::shell) is_new: bool,
}

#[derive(Clone, Copy)]
pub(in crate::ui::shell) struct OnboardingState {
    pub(in crate::ui::shell) show_onboarding: bool,
    pub(in crate::ui::shell) onboarding_step: OnboardingStep,
    pub(in crate::ui::shell) visible_onboarding_step: OnboardingStep,
    pub(in crate::ui::shell) onboarding_step_transition: Option<OnboardingStepTransition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::shell) enum SyncPassphraseOperation {
    Save,
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::shell) enum SyncProviderConfigSaveOperation {
    GithubGist,
    WebDav,
}

pub(in crate::ui::shell) struct SyncUiState {
    pub(in crate::ui::shell) sync_engine: SyncEngine,
    pub(in crate::ui::shell) sync_status: SyncStatus,
    pub(in crate::ui::shell) active_sync_task: Option<gpui_kit::Task<()>>,
    pub(in crate::ui::shell) sync_provider_config_save_operation:
        Option<SyncProviderConfigSaveOperation>,
    pub(in crate::ui::shell) sync_passphrase_operation: Option<SyncPassphraseOperation>,
    pub(in crate::ui::shell) sync_passphrase_configured: bool,
}

#[derive(Default)]
pub(in crate::ui::shell) struct SecretVisibilityState {
    sync_github_token: bool,
    sync_webdav_password: bool,
    sync_passphrase: bool,
    sync_passphrase_confirmation: bool,
    local_vault_current_passphrase: bool,
    local_vault_passphrase: bool,
    local_vault_passphrase_confirmation: bool,
    web_search_api_key: bool,
    ai_provider_api_keys: std::collections::HashSet<String>,
}

impl SecretVisibilityState {
    pub(in crate::ui::shell) fn is_visible(&self, target: &SecretRevealTarget) -> bool {
        match target {
            SecretRevealTarget::SyncGithubToken => self.sync_github_token,
            SecretRevealTarget::SyncWebdavPassword => self.sync_webdav_password,
            SecretRevealTarget::HostPassword => false,
            SecretRevealTarget::SyncPassphrase => self.sync_passphrase,
            SecretRevealTarget::SyncPassphraseConfirmation => self.sync_passphrase_confirmation,
            SecretRevealTarget::LocalVaultCurrentPassphrase => self.local_vault_current_passphrase,
            SecretRevealTarget::LocalVaultPassphrase => self.local_vault_passphrase,
            SecretRevealTarget::LocalVaultPassphraseConfirmation => {
                self.local_vault_passphrase_confirmation
            }
            SecretRevealTarget::WebSearchApiKey => self.web_search_api_key,
            SecretRevealTarget::AiProviderApiKey(provider_id) => {
                self.ai_provider_api_keys.contains(provider_id)
            }
        }
    }

    pub(in crate::ui::shell) fn set_visible(&mut self, target: SecretRevealTarget, visible: bool) {
        match target {
            SecretRevealTarget::SyncGithubToken => self.sync_github_token = visible,
            SecretRevealTarget::SyncWebdavPassword => self.sync_webdav_password = visible,
            SecretRevealTarget::HostPassword => {}
            SecretRevealTarget::SyncPassphrase => self.sync_passphrase = visible,
            SecretRevealTarget::SyncPassphraseConfirmation => {
                self.sync_passphrase_confirmation = visible;
            }
            SecretRevealTarget::LocalVaultCurrentPassphrase => {
                self.local_vault_current_passphrase = visible;
            }
            SecretRevealTarget::LocalVaultPassphrase => self.local_vault_passphrase = visible,
            SecretRevealTarget::LocalVaultPassphraseConfirmation => {
                self.local_vault_passphrase_confirmation = visible;
            }
            SecretRevealTarget::WebSearchApiKey => self.web_search_api_key = visible,
            SecretRevealTarget::AiProviderApiKey(provider_id) => {
                if visible {
                    self.ai_provider_api_keys.insert(provider_id);
                } else {
                    self.ai_provider_api_keys.remove(&provider_id);
                }
            }
        }
    }

    pub(in crate::ui::shell) fn clear_ai_provider_visibility(&mut self) {
        self.ai_provider_api_keys.clear();
    }
}

#[derive(Clone)]
pub(in crate::ui::shell) struct SettingsForms {
    pub(in crate::ui::shell) language_select: Entity<SelectState<Vec<SelectOption<AppLanguage>>>>,
    pub(in crate::ui::shell) last_tab_close_behavior_select:
        Entity<SelectState<Vec<SelectOption<LastTabCloseBehavior>>>>,
    pub(in crate::ui::shell) window_close_behavior_select:
        Entity<SelectState<Vec<SelectOption<WindowCloseBehavior>>>>,
    pub(in crate::ui::shell) local_vault_auto_lock_duration_select:
        Entity<SelectState<Vec<SelectOption<LocalVaultAutoLockDuration>>>>,
    pub(in crate::ui::shell) monitor_history_select:
        Entity<SelectState<Vec<SelectOption<MonitorHistoryDuration>>>>,
    pub(in crate::ui::shell) terminal_right_click_behavior_select:
        Entity<SelectState<Vec<SelectOption<TerminalRightClickBehavior>>>>,
    pub(in crate::ui::shell) open_ssh_integration_mode_select:
        Entity<SelectState<Vec<SelectOption<OpenSshIntegrationMode>>>>,
    pub(in crate::ui::shell) ssh_bridge_security_level_select:
        Entity<SelectState<Vec<SelectOption<BridgeSecurityLevel>>>>,
    pub(in crate::ui::shell) profile_import_source_select:
        Entity<SelectState<Vec<SelectOption<ImportSourceKind>>>>,
    pub(in crate::ui::shell) sync_provider_select:
        Entity<SelectState<Vec<SelectOption<SyncProvider>>>>,
    pub(in crate::ui::shell) ai_provider_select: Entity<SelectState<Vec<SelectOption<String>>>>,
    pub(in crate::ui::shell) ai_provider_kind_select:
        Entity<SelectState<Vec<SelectOption<AiProviderKind>>>>,
    pub(in crate::ui::shell) web_search_kind_select:
        Entity<SelectState<Vec<SelectOption<WebSearchProviderKind>>>>,
    pub(in crate::ui::shell) proxy_management_select:
        Entity<SelectState<Vec<SelectOption<String>>>>,
    pub(in crate::ui::shell) proxy_management_query_input: Entity<InputState>,
    pub(in crate::ui::shell) proxy_management_scroll_handle: UniformListScrollHandle,
    pub(in crate::ui::shell) proxy_protocol_select:
        Entity<SelectState<Vec<SelectOption<ProxyProtocol>>>>,
    pub(in crate::ui::shell) proxy_auth_mode_select:
        Entity<SelectState<Vec<SelectOption<ProxyAuthMode>>>>,
    pub(in crate::ui::shell) font_family_options: Vec<String>,
    pub(in crate::ui::shell) font_family_query_input: Entity<InputState>,
    pub(in crate::ui::shell) font_family_scroll_handle: UniformListScrollHandle,
    pub(in crate::ui::shell) terminal_font_family_query_input: Entity<InputState>,
    pub(in crate::ui::shell) terminal_font_family_scroll_handle: UniformListScrollHandle,
    pub(in crate::ui::shell) font_fallbacks_input: Entity<InputState>,
    pub(in crate::ui::shell) seed_color_picker: Entity<ColorPickerState>,
    pub(in crate::ui::shell) key_capture_focus: FocusHandle,
    pub(in crate::ui::shell) sync_github_token_input: Entity<InputState>,
    pub(in crate::ui::shell) sync_github_gist_id_input: Entity<InputState>,
    pub(in crate::ui::shell) sync_webdav_url_input: Entity<InputState>,
    pub(in crate::ui::shell) sync_webdav_username_input: Entity<InputState>,
    pub(in crate::ui::shell) sync_webdav_password_input: Entity<InputState>,
    pub(in crate::ui::shell) sync_passphrase_input: Entity<InputState>,
    pub(in crate::ui::shell) sync_passphrase_confirmation_input: Entity<InputState>,
    pub(in crate::ui::shell) local_data_reset_confirmation_input: Entity<InputState>,
    pub(in crate::ui::shell) local_vault_current_passphrase_input: Entity<InputState>,
    pub(in crate::ui::shell) local_vault_passphrase_input: Entity<InputState>,
    pub(in crate::ui::shell) local_vault_passphrase_confirmation_input: Entity<InputState>,
    pub(in crate::ui::shell) ai_provider_name_input: Entity<InputState>,
    pub(in crate::ui::shell) ai_provider_model_input: Entity<InputState>,
    pub(in crate::ui::shell) ai_provider_base_url_input: Entity<InputState>,
    pub(in crate::ui::shell) ai_provider_api_key_input: Entity<InputState>,
    pub(in crate::ui::shell) ai_provider_temperature_input: Entity<InputState>,
    pub(in crate::ui::shell) ai_provider_max_tokens_input: Entity<InputState>,
    pub(in crate::ui::shell) ai_provider_context_window_input: Entity<InputState>,
    pub(in crate::ui::shell) web_search_api_key_input: Entity<InputState>,
    pub(in crate::ui::shell) web_search_endpoint_input: Entity<InputState>,
    pub(in crate::ui::shell) web_search_max_results_input: Entity<InputState>,
    pub(in crate::ui::shell) proxy_name_input: Entity<InputState>,
    pub(in crate::ui::shell) proxy_host_input: Entity<InputState>,
    pub(in crate::ui::shell) proxy_port_input: Entity<InputState>,
    pub(in crate::ui::shell) proxy_username_input: Entity<InputState>,
    pub(in crate::ui::shell) proxy_password_input: Entity<InputState>,
}

pub(in crate::ui::shell) struct SettingsControllerArgs {
    pub runtime: TokioHandle,
    pub proxy_store: Option<ProxyStore>,
    pub proxies: Vec<ProxyProfile>,
    pub settings_store: SettingsStore,
    pub secrets: SecretStore,
    pub sync_engine: SyncEngine,
    pub local_vault_status: LocalVaultStatus,
    pub ssh_bridge_service: SshBridgeService,
    pub open_ssh_integration_service: OpenSshIntegrationService,
    pub sync_executor: Option<SyncExecutor>,
    pub auto_sync: miaominal_services::AutoSyncSnapshot,
}

struct SettingsBootstrap {
    forms: SettingsForms,
    sync: SyncUiState,
    onboarding: OnboardingState,
    local_vault_status: LocalVaultStatus,
}

pub(in crate::ui::shell) struct SettingsController {
    runtime: TokioHandle,
    proxy_store: Option<ProxyStore>,
    proxies: Vec<ProxyProfile>,
    session_query: SessionQueryPort,
    editing_proxy_id: Option<String>,
    proxy_resolve_dns_through_proxy: bool,
    proxy_password_clear_requested: bool,
    settings_store: SettingsStore,
    secrets: SecretStore,
    ssh_bridge_service: SshBridgeService,
    open_ssh_integration_service: OpenSshIntegrationService,
    sync_executor: Option<SyncExecutor>,
    auto_sync_snapshot: miaominal_services::AutoSyncSnapshot,
    ssh_bridge_status: SshBridgeStatus,
    ssh_bridge_sync_result: Option<SshBridgeSyncResult>,
    ssh_bridge_security: BridgeSecuritySnapshot,
    ssh_bridge_notification_main_window: AnyWindowHandle,
    ssh_bridge_notification_state: BridgeSecurityNotificationState,
    ssh_bridge_notification_window: Option<(
        WindowHandle<BridgeSecurityNotificationView>,
        BridgeSecurityNotificationKey,
    )>,
    pending_ssh_bridge_policy_downgrade: Option<PendingSshBridgePolicyDowngradeState>,
    ssh_bridge_settings_instance_generation: Cell<u64>,
    settings_destination_pending: Cell<Option<SettingsDestination>>,
    pub(in crate::ui::shell) forms: SettingsForms,
    sync: SyncUiState,
    onboarding: OnboardingState,
    local_vault_status: LocalVaultStatus,
    local_vault_operation_results: std::collections::VecDeque<LocalVaultOperationResult>,
    local_vault_operation_task: Option<gpui_kit::Task<()>>,
    local_vault_unlock_in_progress: bool,
    local_vault_disable_in_progress: bool,
    local_vault_session_passphrase: Option<ProtectedPassphrase>,
    recording_binding: Option<KeyBindingSlot>,
    pending_preview: Option<String>,
    pending_binding: Option<KeyBinding>,
    editing_ai_provider_id: Option<String>,
    sync_direction: Option<PendingSyncDirectionState>,
    sync_pull_confirm: Option<PendingSyncPullConfirmState>,
    local_vault_disable_confirm: Option<PendingLocalVaultDisableConfirmState>,
    local_data_reset_confirm: Option<PendingLocalDataResetConfirmState>,
    local_data_reset_confirmation_popup: Option<PendingLocalDataResetConfirmationPopupState>,
    sync_passphrase_clear_confirm_popup: Option<PendingSyncPassphraseClearConfirmPopupState>,
    sync_passphrase_popup: Option<PendingSyncPassphrasePopupState>,
    ai_provider_popup: Option<PendingAiProviderPopupState>,
    web_search_config_popup: Option<PendingWebSearchConfigPopupState>,
    sync_provider_config_popup: Option<PendingSyncProviderConfigPopupState>,
    proxy_config_popup: Option<PendingProxyConfigPopupState>,
    local_vault_passphrase_popup: Option<LocalVaultPassphrasePopupMode>,
    sync_provider_config_save_task: Option<gpui_kit::Task<()>>,
    sync_passphrase_task: Option<gpui_kit::Task<()>>,
    ai_provider_save_in_progress: bool,
    ai_provider_save_task: Option<gpui_kit::Task<()>>,
    web_search_save_in_progress: bool,
    web_search_save_task: Option<gpui_kit::Task<()>>,
    ai_provider_api_key_load_in_progress: Option<String>,
    ai_provider_api_key_load_tasks: std::collections::HashMap<u64, gpui_kit::Task<()>>,
    next_ai_provider_api_key_load_task_id: u64,
    local_data_reset_in_progress: bool,
    local_data_reset_task: Option<gpui_kit::Task<()>>,
    secret_visibility: SecretVisibilityState,
    _subscriptions: Vec<Subscription>,
}

impl SettingsController {
    fn font_family_options(current_font_families: &[&str]) -> Vec<String> {
        let mut families = miaominal_settings::available_font_families();
        let default_font_family = miaominal_settings::default_font_family();
        if !families
            .iter()
            .any(|family| family.eq_ignore_ascii_case(&default_font_family))
        {
            families.push(default_font_family);
        }

        for current_font_family in current_font_families {
            let trimmed_current = current_font_family.trim();
            if !trimmed_current.is_empty()
                && !families
                    .iter()
                    .any(|family| family.eq_ignore_ascii_case(trimmed_current))
            {
                families.push(trimmed_current.to_string());
            }
        }

        families.sort_by_cached_key(|family| family.to_ascii_lowercase());
        families.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        families
    }

    fn build_bootstrap(
        settings_store: &SettingsStore,
        proxies: &[ProxyProfile],
        sync_engine: SyncEngine,
        local_vault_status: LocalVaultStatus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> SettingsBootstrap {
        let settings = settings_store.settings();
        let sync_secrets = sync_engine
            .config_store
            .get_secrets()
            .unwrap_or_else(|error| {
                log::warn!("failed to load sync secrets from credential store: {error:?}");
                Default::default()
            });
        let sync_github_token = sync_secrets.github_token.unwrap_or_default();
        let sync_webdav_password = sync_secrets.webdav_password.unwrap_or_default();
        let sync_passphrase = sync_secrets.passphrase.unwrap_or_default();
        let sync_passphrase_configured = sync_engine.config_store.config.has_passphrase;

        let language_options = AppLanguage::supported_languages()
            .into_iter()
            .map(|language| SelectOption::new(language, language.native_name()))
            .collect::<Vec<_>>();
        let selected_language = language_options
            .iter()
            .position(|language| *language.value() == settings.language)
            .map(|index| IndexPath::default().row(index));
        let last_tab_close_behavior_options = LastTabCloseBehavior::all()
            .iter()
            .copied()
            .map(|behavior| SelectOption::new(behavior, last_tab_close_behavior_label(behavior)))
            .collect::<Vec<_>>();
        let selected_last_tab_close_behavior = last_tab_close_behavior_options
            .iter()
            .position(|behavior| *behavior.value() == settings.last_tab_close_behavior)
            .map(|index| IndexPath::default().row(index));
        let window_close_behavior_options = WindowCloseBehavior::all()
            .iter()
            .copied()
            .map(|behavior| SelectOption::new(behavior, window_close_behavior_label(behavior)))
            .collect::<Vec<_>>();
        let selected_window_close_behavior = window_close_behavior_options
            .iter()
            .position(|behavior| *behavior.value() == settings.window_close_behavior)
            .map(|index| IndexPath::default().row(index));
        let local_vault_auto_lock_duration_options = LocalVaultAutoLockDuration::all()
            .iter()
            .copied()
            .map(|duration| {
                SelectOption::new(duration, local_vault_auto_lock_duration_label(duration))
            })
            .collect::<Vec<_>>();
        let selected_local_vault_auto_lock_duration = local_vault_auto_lock_duration_options
            .iter()
            .position(|duration| *duration.value() == settings.local_vault_auto_lock_duration)
            .map(|index| IndexPath::default().row(index));
        let monitor_history_options = MonitorHistoryDuration::all()
            .iter()
            .copied()
            .map(|duration| SelectOption::new(duration, monitor_history_duration_label(duration)))
            .collect::<Vec<_>>();
        let selected_monitor_history = monitor_history_options
            .iter()
            .position(|duration| *duration.value() == settings.monitor_history_duration)
            .map(|index| IndexPath::default().row(index));
        let terminal_right_click_behavior_options = vec![
            SelectOption::new(
                TerminalRightClickBehavior::ContextMenu,
                i18n::string("settings.key_bindings.context_menu_option"),
            ),
            SelectOption::new(
                TerminalRightClickBehavior::CopySelectionOrPaste,
                i18n::string("settings.key_bindings.copy_paste_option"),
            ),
        ];
        let selected_terminal_right_click_behavior = terminal_right_click_behavior_options
            .iter()
            .position(|behavior| *behavior.value() == settings.terminal_right_click_behavior)
            .map(|index| IndexPath::default().row(index));
        let open_ssh_integration_mode_options = [
            OpenSshIntegrationMode::Disabled,
            OpenSshIntegrationMode::Direct,
            OpenSshIntegrationMode::Bridge,
        ]
        .into_iter()
        .map(|mode| SelectOption::new(mode, open_ssh_integration_mode_label(mode)))
        .collect::<Vec<_>>();
        let selected_open_ssh_integration_mode = open_ssh_integration_mode_options
            .iter()
            .position(|mode| *mode.value() == settings.open_ssh_integration_mode)
            .map(|index| IndexPath::default().row(index));
        let ssh_bridge_security_level_options = [
            BridgeSecurityLevel::Standard,
            Self::default_bridge_approval_level(),
            BridgeSecurityLevel::RequireSystemAuth,
        ]
        .into_iter()
        .map(|level| SelectOption::new(level, bridge_security_level_label(level)))
        .collect::<Vec<_>>();
        let profile_import_source_options = [
            ImportSourceKind::OpenSshConfig,
            ImportSourceKind::PuttyRegistry,
            ImportSourceKind::SecureCrtXml,
            ImportSourceKind::FinalShellJson,
        ]
        .into_iter()
        .map(|source| SelectOption::new(source, localized_profile_import_source_label(source)))
        .collect::<Vec<_>>();
        let sync_provider_options = vec![
            SelectOption::new(
                SyncProvider::None,
                i18n::string("settings.sync.providers.none"),
            ),
            SelectOption::new(
                SyncProvider::GithubGist,
                i18n::string("settings.sync.providers.gist"),
            ),
            SelectOption::new(
                SyncProvider::WebDav,
                i18n::string("settings.sync.providers.webdav"),
            ),
        ];
        let selected_sync_provider = sync_provider_options
            .iter()
            .position(|provider| *provider.value() == sync_engine.config_store.config.provider)
            .map(|index| IndexPath::default().row(index));
        let ai_provider_options = ai_provider_select_options(settings);
        let selected_ai_provider = settings
            .selected_ai_provider_id
            .as_ref()
            .and_then(|persisted_id| {
                ai_provider_options
                    .iter()
                    .position(|option| option.value() == persisted_id)
            })
            .map(|index| IndexPath::default().row(index))
            .or_else(|| (!ai_provider_options.is_empty()).then(|| IndexPath::default().row(0)));
        let ai_provider_kind_options = AiProviderKind::all()
            .iter()
            .copied()
            .filter(|kind| ai_provider_kind_chat_supported(*kind))
            .map(|kind| SelectOption::new(kind, i18n::string(ai_provider_kind_label_key(kind))))
            .collect::<Vec<_>>();
        let selected_ai_provider_kind = ai_provider_kind_options
            .iter()
            .position(|provider| *provider.value() == AiProviderKind::OpenAi)
            .map(|index| IndexPath::default().row(index));
        let web_search_kind_options = WebSearchProviderKind::all()
            .iter()
            .copied()
            .map(|kind| {
                SelectOption::new(kind, i18n::string(web_search_provider_kind_label_key(kind)))
            })
            .collect::<Vec<_>>();
        let selected_web_search_kind = web_search_kind_options
            .iter()
            .position(|provider| *provider.value() == settings.web_search.kind)
            .map(|index| IndexPath::default().row(index));
        let proxy_management_options = proxy_management_select_options(proxies);
        let selected_proxy_management =
            (!proxy_management_options.is_empty()).then(|| IndexPath::default().row(0));
        let proxy_protocol_options = vec![
            SelectOption::new(ProxyProtocol::Socks5, "SOCKS5"),
            SelectOption::new(ProxyProtocol::HttpConnect, "HTTP CONNECT"),
        ];
        let proxy_auth_mode_options = vec![
            SelectOption::new(
                ProxyAuthMode::None,
                i18n::string("settings.proxies.auth.none"),
            ),
            SelectOption::new(
                ProxyAuthMode::UsernamePassword,
                i18n::string("settings.proxies.auth.username_password"),
            ),
        ];
        let font_family_options = Self::font_family_options(&[
            settings.font_family.as_str(),
            settings.terminal_font_family.as_str(),
        ]);
        let web_search_config = &settings.web_search;

        let forms = SettingsForms {
            language_select: cx
                .new(|cx| SelectState::new(language_options, selected_language, window, cx)),
            last_tab_close_behavior_select: cx.new(|cx| {
                SelectState::new(
                    last_tab_close_behavior_options,
                    selected_last_tab_close_behavior,
                    window,
                    cx,
                )
            }),
            window_close_behavior_select: cx.new(|cx| {
                SelectState::new(
                    window_close_behavior_options,
                    selected_window_close_behavior,
                    window,
                    cx,
                )
            }),
            local_vault_auto_lock_duration_select: cx.new(|cx| {
                SelectState::new(
                    local_vault_auto_lock_duration_options,
                    selected_local_vault_auto_lock_duration,
                    window,
                    cx,
                )
            }),
            monitor_history_select: cx.new(|cx| {
                SelectState::new(
                    monitor_history_options,
                    selected_monitor_history,
                    window,
                    cx,
                )
            }),
            terminal_right_click_behavior_select: cx.new(|cx| {
                SelectState::new(
                    terminal_right_click_behavior_options,
                    selected_terminal_right_click_behavior,
                    window,
                    cx,
                )
            }),
            open_ssh_integration_mode_select: cx.new(|cx| {
                SelectState::new(
                    open_ssh_integration_mode_options,
                    selected_open_ssh_integration_mode,
                    window,
                    cx,
                )
            }),
            ssh_bridge_security_level_select: cx.new(|cx| {
                SelectState::new(
                    ssh_bridge_security_level_options,
                    Some(IndexPath::default().row(0)),
                    window,
                    cx,
                )
            }),
            profile_import_source_select: cx.new(|cx| {
                SelectState::new(
                    profile_import_source_options,
                    Some(IndexPath::default().row(0)),
                    window,
                    cx,
                )
            }),
            sync_provider_select: cx.new(|cx| {
                SelectState::new(sync_provider_options, selected_sync_provider, window, cx)
            }),
            ai_provider_select: cx
                .new(|cx| SelectState::new(ai_provider_options, selected_ai_provider, window, cx)),
            ai_provider_kind_select: cx.new(|cx| {
                SelectState::new(
                    ai_provider_kind_options,
                    selected_ai_provider_kind,
                    window,
                    cx,
                )
            }),
            web_search_kind_select: cx.new(|cx| {
                SelectState::new(
                    web_search_kind_options,
                    selected_web_search_kind,
                    window,
                    cx,
                )
            }),
            proxy_management_select: cx.new(|cx| {
                SelectState::new(
                    proxy_management_options,
                    selected_proxy_management,
                    window,
                    cx,
                )
            }),
            proxy_management_query_input: new_input_state(
                i18n::string("settings.proxies.picker.search_placeholder"),
                "",
                false,
                window,
                cx,
            ),
            proxy_management_scroll_handle: UniformListScrollHandle::new(),
            proxy_protocol_select: cx.new(|cx| {
                SelectState::new(
                    proxy_protocol_options,
                    Some(IndexPath::default().row(0)),
                    window,
                    cx,
                )
            }),
            proxy_auth_mode_select: cx.new(|cx| {
                SelectState::new(
                    proxy_auth_mode_options,
                    Some(IndexPath::default().row(0)),
                    window,
                    cx,
                )
            }),
            font_family_options,
            font_family_query_input: new_input_state(
                i18n::string("settings.appearance.font_picker.search_placeholder"),
                "",
                false,
                window,
                cx,
            ),
            font_family_scroll_handle: UniformListScrollHandle::new(),
            terminal_font_family_query_input: new_input_state(
                i18n::string("settings.appearance.font_picker.search_placeholder"),
                "",
                false,
                window,
                cx,
            ),
            terminal_font_family_scroll_handle: UniformListScrollHandle::new(),
            font_fallbacks_input: new_input_state(
                "",
                settings.font_fallbacks.join(", "),
                false,
                window,
                cx,
            ),
            seed_color_picker: cx.new(|cx| {
                let seed_color = miaominal_settings::Theme::from_settings(settings)
                    .material
                    .source;
                ColorPickerState::new(window, cx).default_value(rgb(seed_color))
            }),
            key_capture_focus: cx.focus_handle(),
            sync_github_token_input: new_input_state(
                localized_secret_placeholder(
                    sync_engine.config_store.config.has_github_token,
                    "settings.sync.placeholders.github_token",
                ),
                sync_github_token,
                true,
                window,
                cx,
            ),
            sync_github_gist_id_input: new_input_state(
                i18n::string("settings.sync.placeholders.gist_id"),
                sync_engine
                    .config_store
                    .config
                    .gist_id
                    .clone()
                    .unwrap_or_default(),
                false,
                window,
                cx,
            ),
            sync_webdav_url_input: new_input_state(
                i18n::string("settings.sync.placeholders.webdav_url"),
                sync_engine.config_store.config.webdav_url.clone(),
                false,
                window,
                cx,
            ),
            sync_webdav_username_input: new_input_state(
                i18n::string("settings.sync.placeholders.webdav_username"),
                sync_engine.config_store.config.webdav_username.clone(),
                false,
                window,
                cx,
            ),
            sync_webdav_password_input: new_input_state(
                localized_secret_placeholder(
                    sync_engine.config_store.config.has_webdav_password,
                    "settings.sync.placeholders.webdav_password",
                ),
                sync_webdav_password,
                true,
                window,
                cx,
            ),
            sync_passphrase_input: new_input_state(
                i18n::string("settings.sync.placeholders.passphrase"),
                sync_passphrase,
                true,
                window,
                cx,
            ),
            sync_passphrase_confirmation_input: new_input_state(
                i18n::string("settings.sync.placeholders.passphrase"),
                "",
                true,
                window,
                cx,
            ),
            local_data_reset_confirmation_input: new_input_state(
                i18n::string("settings.about.reset_local.popup.placeholder"),
                "",
                false,
                window,
                cx,
            ),
            local_vault_current_passphrase_input: new_input_state(
                i18n::string("settings.sync.placeholders.vault_current_passphrase"),
                "",
                true,
                window,
                cx,
            ),
            local_vault_passphrase_input: new_input_state(
                i18n::string("settings.sync.placeholders.vault_passphrase"),
                "",
                true,
                window,
                cx,
            ),
            local_vault_passphrase_confirmation_input: new_input_state(
                i18n::string("settings.sync.placeholders.vault_passphrase_confirmation"),
                "",
                true,
                window,
                cx,
            ),
            ai_provider_name_input: new_input_state(
                i18n::string("settings.ai_providers.placeholders.name"),
                "",
                false,
                window,
                cx,
            ),
            ai_provider_model_input: new_input_state(
                i18n::string("settings.ai_providers.placeholders.model"),
                AiProviderKind::OpenAi.default_model(),
                false,
                window,
                cx,
            ),
            ai_provider_base_url_input: new_input_state(
                i18n::string("settings.ai_providers.placeholders.base_url"),
                "",
                false,
                window,
                cx,
            ),
            ai_provider_api_key_input: new_input_state(
                i18n::string("settings.ai_providers.placeholders.api_key"),
                "",
                true,
                window,
                cx,
            ),
            ai_provider_temperature_input: new_input_state(
                i18n::string("settings.ai_providers.placeholders.temperature"),
                "",
                false,
                window,
                cx,
            ),
            ai_provider_max_tokens_input: new_input_state(
                i18n::string("settings.ai_providers.placeholders.max_tokens"),
                "",
                false,
                window,
                cx,
            ),
            ai_provider_context_window_input: new_input_state(
                i18n::string("settings.ai_providers.placeholders.context_window"),
                "",
                false,
                window,
                cx,
            ),
            web_search_api_key_input: new_input_state(
                localized_secret_placeholder(
                    web_search_config.has_api_key,
                    "settings.web_search.placeholders.api_key",
                ),
                "",
                true,
                window,
                cx,
            ),
            web_search_endpoint_input: new_input_state(
                web_search_endpoint_placeholder(web_search_config.kind),
                web_search_config.endpoint.clone(),
                false,
                window,
                cx,
            ),
            web_search_max_results_input: new_input_state(
                i18n::string("settings.web_search.placeholders.max_results"),
                web_search_config.max_results.to_string(),
                false,
                window,
                cx,
            ),
            proxy_name_input: new_input_state(
                i18n::string("settings.proxies.placeholders.name"),
                "",
                false,
                window,
                cx,
            ),
            proxy_host_input: new_input_state(
                i18n::string("settings.proxies.placeholders.host"),
                "",
                false,
                window,
                cx,
            ),
            proxy_port_input: new_input_state(
                i18n::string("settings.proxies.placeholders.port"),
                "1080",
                false,
                window,
                cx,
            ),
            proxy_username_input: new_input_state(
                i18n::string("settings.proxies.placeholders.username"),
                "",
                false,
                window,
                cx,
            ),
            proxy_password_input: new_input_state(
                i18n::string("settings.proxies.placeholders.password"),
                "",
                true,
                window,
                cx,
            ),
        };

        SettingsBootstrap {
            forms,
            sync: SyncUiState {
                sync_engine,
                sync_status: SyncStatus::Idle,
                active_sync_task: None,
                sync_provider_config_save_operation: None,
                sync_passphrase_operation: None,
                sync_passphrase_configured,
            },
            onboarding: OnboardingState {
                show_onboarding: settings.should_show_onboarding(),
                onboarding_step: OnboardingStep::Welcome,
                visible_onboarding_step: OnboardingStep::Welcome,
                onboarding_step_transition: None,
            },
            local_vault_status,
        }
    }

    pub(in crate::ui::shell) fn refresh_localized_placeholders(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (input, key) in [
            (
                &self.forms.font_family_query_input,
                "settings.appearance.font_picker.search_placeholder",
            ),
            (
                &self.forms.terminal_font_family_query_input,
                "settings.appearance.font_picker.search_placeholder",
            ),
            (
                &self.forms.ai_provider_name_input,
                "settings.ai_providers.placeholders.name",
            ),
            (
                &self.forms.ai_provider_model_input,
                "settings.ai_providers.placeholders.model",
            ),
            (
                &self.forms.ai_provider_base_url_input,
                "settings.ai_providers.placeholders.base_url",
            ),
            (
                &self.forms.ai_provider_api_key_input,
                "settings.ai_providers.placeholders.api_key",
            ),
            (
                &self.forms.ai_provider_temperature_input,
                "settings.ai_providers.placeholders.temperature",
            ),
            (
                &self.forms.ai_provider_max_tokens_input,
                "settings.ai_providers.placeholders.max_tokens",
            ),
            (
                &self.forms.ai_provider_context_window_input,
                "settings.ai_providers.placeholders.context_window",
            ),
            (
                &self.forms.sync_github_gist_id_input,
                "settings.sync.placeholders.gist_id",
            ),
            (
                &self.forms.sync_webdav_url_input,
                "settings.sync.placeholders.webdav_url",
            ),
            (
                &self.forms.sync_webdav_username_input,
                "settings.sync.placeholders.webdav_username",
            ),
            (
                &self.forms.sync_passphrase_input,
                "settings.sync.placeholders.passphrase",
            ),
            (
                &self.forms.local_vault_current_passphrase_input,
                "settings.sync.placeholders.vault_current_passphrase",
            ),
            (
                &self.forms.local_vault_passphrase_input,
                "settings.sync.placeholders.vault_passphrase",
            ),
            (
                &self.forms.local_vault_passphrase_confirmation_input,
                "settings.sync.placeholders.vault_passphrase_confirmation",
            ),
        ] {
            set_input_placeholder(input, i18n::string(key), window, cx);
        }
        let sync_config = &self.sync.sync_engine.config_store.config;
        set_input_placeholder(
            &self.forms.sync_github_token_input,
            localized_secret_placeholder(
                sync_config.has_github_token,
                "settings.sync.placeholders.github_token",
            ),
            window,
            cx,
        );
        set_input_placeholder(
            &self.forms.sync_webdav_password_input,
            localized_secret_placeholder(
                sync_config.has_webdav_password,
                "settings.sync.placeholders.webdav_password",
            ),
            window,
            cx,
        );
    }

    pub(in crate::ui::shell) fn emit(&mut self, command: AppCommand, cx: &mut Context<Self>) {
        cx.emit(command);
    }

    fn close_bridge_security_notification_window(&mut self, cx: &mut App) {
        if let Some((handle, _)) = self.ssh_bridge_notification_window.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }

    fn push_bridge_security_in_app_notification(
        &self,
        model: &BridgeSecurityNotificationModel,
        cx: &mut Context<Self>,
    ) {
        let vault_unlock = model.phase == BridgePendingPhase::AwaitingVaultUnlock;
        let mut message = i18n::string_args(
            if vault_unlock {
                "settings.openssh_integration.security.in_app_vault_request"
            } else {
                "settings.openssh_integration.security.in_app_request"
            },
            &[
                ("profile", &model.profile_name),
                ("source", &model.source_summary),
            ],
        );
        if model.additional_count > 0 {
            let count = model.additional_count.to_string();
            message.push(' ');
            message.push_str(&i18n::string_args(
                "settings.openssh_integration.security.notification_additional",
                &[("count", &count)],
            ));
        }
        let controller = cx.entity().clone();
        let notification = AppNotification::new(
            AppNotificationTone::Warning,
            AppNotificationPriority::High,
            i18n::string(if vault_unlock {
                "settings.openssh_integration.security.in_app_vault_title"
            } else {
                "settings.openssh_integration.security.in_app_title"
            }),
            message,
        )
        .toast_action(
            i18n::string(if vault_unlock {
                "settings.openssh_integration.security.unlock_vault"
            } else {
                "settings.openssh_integration.security.view"
            }),
            move |_, cx| {
                controller.update(cx, |controller, cx| {
                    if vault_unlock {
                        controller.unlock_ssh_bridge_vault(cx);
                    } else {
                        controller.request_ssh_bridge_security_page();
                    }
                    cx.emit(AppCommand::SidebarSectionRequested(
                        SidebarSection::Settings,
                    ));
                    cx.notify();
                });
            },
        )
        .id1::<BridgeApprovalNotification>(model.key.request_id.clone());
        let _ = self
            .ssh_bridge_notification_main_window
            .update(cx, move |_, window, cx| {
                crate::ui::shell::push_app_notification(window, notification, cx);
            });
    }

    fn open_bridge_security_notification_window(
        &mut self,
        model: BridgeSecurityNotificationModel,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let display = self
            .ssh_bridge_notification_main_window
            .update(cx, |_, window, cx| window.display(cx))
            .ok()
            .flatten()
            .or_else(|| cx.primary_display());
        let options = bridge_security_notification_window_options(display.as_deref());
        let controller = cx.weak_entity();
        let view_model = model.clone();
        let handle = cx.open_window(options, move |_, cx| {
            cx.new(|cx| BridgeSecurityNotificationView::new(view_model, controller.clone(), cx))
        })?;
        if let Err(error) = handle
            .update(cx, |_, window, _| {
                crate::ui::bridge_security_platform::configure_notification_window(window)
            })
            .and_then(|result| result)
        {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
            return Err(error);
        }
        self.ssh_bridge_notification_window = Some((handle, model.key));
        Ok(())
    }

    fn sync_bridge_security_notification(
        &mut self,
        snapshot: &BridgeSecuritySnapshot,
        bridge_running: bool,
        app_foreground: bool,
        cx: &mut Context<Self>,
    ) {
        if !bridge_running {
            self.close_bridge_security_notification_window(cx);
            self.ssh_bridge_notification_state.clear();
            return;
        }
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let Some(mut model) = self.ssh_bridge_notification_state.reconcile(snapshot, now) else {
            self.close_bridge_security_notification_window(cx);
            self.ssh_bridge_notification_state.clear();
            return;
        };
        model.vault_locked = self.local_vault_status == LocalVaultStatus::Locked;

        if app_foreground {
            self.close_bridge_security_notification_window(cx);
            if self
                .ssh_bridge_notification_state
                .should_present(&model.key)
            {
                self.push_bridge_security_in_app_notification(&model, cx);
                self.ssh_bridge_notification_state.mark_presented(model.key);
            }
            return;
        }

        if let Some((handle, key)) = self.ssh_bridge_notification_window.as_ref() {
            let handle = *handle;
            if *key == model.key {
                let updated_model = model.clone();
                if handle
                    .update(cx, |view, _, cx| view.set_model(updated_model, cx))
                    .is_ok()
                {
                    return;
                }
                self.ssh_bridge_notification_window = None;
            } else {
                self.close_bridge_security_notification_window(cx);
            }
        }

        if !self
            .ssh_bridge_notification_state
            .should_present(&model.key)
        {
            return;
        }

        let key = model.key.clone();
        if let Err(error) = self.open_bridge_security_notification_window(model.clone(), cx) {
            log::warn!("failed to show SSH Bridge notification window: {error:?}");
            self.push_bridge_security_in_app_notification(&model, cx);
        }
        self.ssh_bridge_notification_state.mark_presented(key);
    }

    pub(in crate::ui::shell) fn dismiss_bridge_security_notification(
        &mut self,
        key: BridgeSecurityNotificationKey,
        cx: &mut Context<Self>,
    ) {
        self.ssh_bridge_notification_window = None;
        self.ssh_bridge_notification_state.dismiss(key);
        cx.notify();
    }

    pub(in crate::ui::shell) fn handle_bridge_security_notification_action(
        &mut self,
        key: BridgeSecurityNotificationKey,
        action: BridgeSecurityNotificationAction,
        cx: &mut Context<Self>,
    ) {
        self.ssh_bridge_notification_window = None;
        let activates_main_window = matches!(
            action,
            BridgeSecurityNotificationAction::OpenSecurity
                | BridgeSecurityNotificationAction::UnlockVault
                | BridgeSecurityNotificationAction::ApproveAndUnlock
        );
        if activates_main_window {
            cx.activate(true);
            let _ = self
                .ssh_bridge_notification_main_window
                .update(cx, |_, window, _| window.activate_window());
        }
        let request_still_pending = self
            .ssh_bridge_service
            .security_snapshot()
            .pending
            .iter()
            .any(|request| request.request_id == key.request_id && request.phase == key.phase);
        if !request_still_pending {
            self.request_ssh_bridge_security_page();
            cx.emit(AppCommand::SidebarSectionRequested(
                SidebarSection::Settings,
            ));
            cx.emit(AppCommand::Feedback(i18n::string(
                "settings.openssh_integration.security.request_expired",
            )));
            cx.notify();
            return;
        }
        match action {
            BridgeSecurityNotificationAction::OpenSecurity => {
                self.request_ssh_bridge_security_page();
            }
            BridgeSecurityNotificationAction::UnlockVault => {
                self.unlock_ssh_bridge_vault(cx);
            }
            BridgeSecurityNotificationAction::Approve => {
                self.approve_ssh_bridge_request(key.request_id, cx);
            }
            BridgeSecurityNotificationAction::ApproveAndUnlock => {
                self.approve_ssh_bridge_request(key.request_id, cx);
                self.unlock_ssh_bridge_vault(cx);
            }
            BridgeSecurityNotificationAction::Reject => {
                self.reject_ssh_bridge_request(key.request_id, cx);
            }
        }
        if activates_main_window {
            cx.emit(AppCommand::SidebarSectionRequested(
                SidebarSection::Settings,
            ));
        }
        cx.notify();
    }

    fn request_ssh_bridge_security_page(&self) {
        self.request_settings_destination(SettingsDestination::SshBridgeSecurity);
    }

    pub(in crate::ui::shell) fn request_settings_destination(
        &self,
        destination: SettingsDestination,
    ) {
        let mut generation = self
            .ssh_bridge_settings_instance_generation
            .get()
            .wrapping_add(1);
        if generation == 0 {
            generation = 1;
        }
        self.ssh_bridge_settings_instance_generation.set(generation);
        self.settings_destination_pending.set(Some(destination));
    }

    pub(in crate::ui::shell) fn take_settings_render_request(
        &self,
    ) -> (u64, Option<SettingsDestination>) {
        (
            self.ssh_bridge_settings_instance_generation.get(),
            self.settings_destination_pending.take(),
        )
    }

    pub(in crate::ui::shell) fn new(
        args: SettingsControllerArgs,
        session_query: SessionQueryPort,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let bootstrap = Self::build_bootstrap(
            &args.settings_store,
            &args.proxies,
            args.sync_engine.clone(),
            args.local_vault_status,
            window,
            cx,
        );
        let forms = bootstrap.forms;
        let last_tab_close_behavior_select = forms.last_tab_close_behavior_select.clone();
        let window_close_behavior_select = forms.window_close_behavior_select.clone();
        let local_vault_auto_lock_duration_select =
            forms.local_vault_auto_lock_duration_select.clone();
        let monitor_history_select = forms.monitor_history_select.clone();
        let terminal_right_click_behavior_select =
            forms.terminal_right_click_behavior_select.clone();
        let open_ssh_integration_mode_select = forms.open_ssh_integration_mode_select.clone();
        let ssh_bridge_security_level_select = forms.ssh_bridge_security_level_select.clone();
        let sync_provider_select = forms.sync_provider_select.clone();
        let ai_provider_select = forms.ai_provider_select.clone();
        let ai_provider_kind_select = forms.ai_provider_kind_select.clone();
        let language_select = forms.language_select.clone();
        let font_family_query_input = forms.font_family_query_input.clone();
        let terminal_font_family_query_input = forms.terminal_font_family_query_input.clone();
        let font_fallbacks_input = forms.font_fallbacks_input.clone();
        let seed_color_picker = forms.seed_color_picker.clone();
        let web_search_kind_select = forms.web_search_kind_select.clone();
        let proxy_management_query_input = forms.proxy_management_query_input.clone();
        let proxy_protocol_select = forms.proxy_protocol_select.clone();
        let proxy_auth_mode_select = forms.proxy_auth_mode_select.clone();

        let last_tab_close_behavior_subscription = cx.subscribe(
            &last_tab_close_behavior_select,
            |this: &mut Self, _, event, cx| {
                let SelectEvent::Confirm(selected) = event;
                let Some(behavior) = selected.as_ref().copied() else {
                    return;
                };
                if this
                    .settings_store
                    .update(|settings| settings.last_tab_close_behavior = behavior)
                {
                    let message = match behavior {
                        LastTabCloseBehavior::ExitApplication => {
                            i18n::string("status.last_tab_close_behavior_exit")
                        }
                        LastTabCloseBehavior::OpenNewHomeTab => {
                            i18n::string("status.last_tab_close_behavior_open_home")
                        }
                    };
                    cx.emit(AppCommand::Feedback(message));
                    cx.notify();
                }
            },
        );
        let window_close_behavior_subscription = cx.subscribe(
            &window_close_behavior_select,
            |this: &mut Self, _, event, cx| {
                let SelectEvent::Confirm(selected) = event;
                let Some(behavior) = selected.as_ref().copied() else {
                    return;
                };
                if this
                    .settings_store
                    .update(|settings| settings.window_close_behavior = behavior)
                {
                    crate::ui::sync_system_tray(cx);
                    let message = match behavior {
                        WindowCloseBehavior::ExitApplication => {
                            i18n::string("status.window_close_behavior_exit")
                        }
                        WindowCloseBehavior::MinimizeToTray => {
                            i18n::string("status.window_close_behavior_minimize")
                        }
                    };
                    cx.emit(AppCommand::Feedback(message));
                    cx.notify();
                }
            },
        );
        let local_vault_auto_lock_duration_subscription = cx.subscribe(
            &local_vault_auto_lock_duration_select,
            |this: &mut Self, _, event, cx| {
                let SelectEvent::Confirm(selected) = event;
                let Some(duration) = selected.as_ref().copied() else {
                    return;
                };
                if this
                    .settings_store
                    .update(|settings| settings.local_vault_auto_lock_duration = duration)
                {
                    cx.emit(AppCommand::Feedback(i18n::string(
                        "status.local_vault_auto_lock_duration_changed",
                    )));
                    cx.notify();
                }
            },
        );
        let monitor_history_subscription =
            cx.subscribe(&monitor_history_select, |this: &mut Self, _, event, cx| {
                let SelectEvent::Confirm(selected) = event;
                let Some(duration) = selected.as_ref().copied() else {
                    return;
                };
                if this
                    .settings_store
                    .update(|settings| settings.monitor_history_duration = duration)
                {
                    cx.emit(AppCommand::Feedback(i18n::string(
                        "status.monitor_history_duration_changed",
                    )));
                    cx.notify();
                }
            });
        let terminal_right_click_behavior_subscription = cx.subscribe(
            &terminal_right_click_behavior_select,
            |this: &mut Self, _, event, cx| {
                let SelectEvent::Confirm(selected) = event;
                let Some(behavior) = selected.as_ref().copied() else {
                    return;
                };
                if this
                    .settings_store
                    .update(|settings| settings.terminal_right_click_behavior = behavior)
                {
                    let message = match behavior {
                        TerminalRightClickBehavior::ContextMenu => {
                            i18n::string("status.right_click_context_menu")
                        }
                        TerminalRightClickBehavior::CopySelectionOrPaste => {
                            i18n::string("status.right_click_copy_paste")
                        }
                    };
                    cx.emit(AppCommand::Feedback(message));
                    cx.notify();
                }
            },
        );
        let open_ssh_integration_mode_subscription = cx.subscribe(
            &open_ssh_integration_mode_select,
            |this: &mut Self, _, event, cx| {
                let SelectEvent::Confirm(selected) = event;
                if let Some(mode) = selected.as_ref().copied() {
                    this.set_open_ssh_integration_mode(mode, cx);
                }
            },
        );
        let ssh_bridge_security_level_subscription = cx.subscribe(
            &ssh_bridge_security_level_select,
            |this: &mut Self, _, event, cx| {
                let SelectEvent::Confirm(selected) = event;
                if let Some(level) = selected.as_ref().copied() {
                    this.request_ssh_bridge_security_policy(level, cx);
                }
            },
        );
        let sync_provider_select_subscription =
            cx.subscribe(&sync_provider_select, |this: &mut Self, _, event, cx| {
                let SelectEvent::Confirm(selected) = event;
                if let Some(provider) = selected.as_ref().copied() {
                    this.select_sync_provider(provider, cx);
                }
            });
        let ai_provider_select_subscription =
            cx.subscribe(&ai_provider_select, |this: &mut Self, _, event, cx| {
                let SelectEvent::Confirm(selected) = event;
                let Some(provider_id) = selected.as_ref().map(|item| (*item).clone()) else {
                    return;
                };
                this.persist_selected_ai_provider(provider_id, cx);
            });
        let ai_provider_kind_subscription =
            cx.subscribe(&ai_provider_kind_select, |this: &mut Self, _, event, cx| {
                let SelectEvent::Confirm(selected) = event;
                if let Some(kind) = selected.as_ref().copied()
                    && this.editing_ai_provider_id.is_none()
                {
                    cx.emit(AppCommand::Feedback(i18n::string_args(
                        "settings.ai_providers.status.kind_selected",
                        &[("kind", &i18n::string(ai_provider_kind_label_key(kind)))],
                    )));
                    cx.notify();
                }
            });
        let language_select_subscription =
            cx.subscribe(&language_select, |this: &mut Self, _, event, cx| {
                let SelectEvent::Confirm(selected) = event;
                if let Some(language) = selected.as_ref().copied() {
                    this.set_language(language, cx);
                }
            });
        let font_family_query_subscription = cx.subscribe(
            &font_family_query_input,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this.forms
                        .font_family_scroll_handle
                        .scroll_to_item(0, ScrollStrategy::Top);
                    cx.notify();
                }
                InputEvent::Focus | InputEvent::Blur => cx.notify(),
                _ => {}
            },
        );
        let terminal_font_family_query_subscription = cx.subscribe(
            &terminal_font_family_query_input,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this.forms
                        .terminal_font_family_scroll_handle
                        .scroll_to_item(0, ScrollStrategy::Top);
                    cx.notify();
                }
                InputEvent::Focus | InputEvent::Blur => cx.notify(),
                _ => {}
            },
        );
        let font_fallbacks_subscription = cx.subscribe(
            &font_fallbacks_input,
            |this: &mut Self, input, event: &InputEvent, cx| {
                if matches!(
                    event,
                    InputEvent::Change | InputEvent::PressEnter { .. } | InputEvent::Blur
                ) {
                    this.update_font_fallbacks(input.read(cx).value().to_string(), cx);
                }
            },
        );
        let seed_color_subscription = cx.subscribe(
            &seed_color_picker,
            |this: &mut Self, _, event: &ColorPickerEvent, cx| {
                let ColorPickerEvent::Change(Some(color)) = event else {
                    return;
                };
                this.update_seed_color(color.to_hex(), cx);
            },
        );
        let web_search_kind_subscription =
            cx.subscribe(&web_search_kind_select, |this: &mut Self, _, event, cx| {
                let SelectEvent::Confirm(selected) = event;
                if let Some(kind) = selected.as_ref().copied() {
                    this.on_web_search_kind_changed(kind, cx);
                }
            });
        let proxy_management_query_subscription = cx.subscribe(
            &proxy_management_query_input,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this.forms
                        .proxy_management_scroll_handle
                        .scroll_to_item(0, ScrollStrategy::Top);
                    cx.notify();
                }
                InputEvent::Focus | InputEvent::Blur => cx.notify(),
                _ => {}
            },
        );
        let proxy_protocol_subscription =
            cx.subscribe(&proxy_protocol_select, |this: &mut Self, _, event, cx| {
                let SelectEvent::Confirm(selected) = event;
                if this.editing_proxy_id.is_some()
                    && let Some(protocol) = selected.as_ref().copied()
                {
                    let port_input = this.forms.proxy_port_input.clone();
                    let current_port = port_input.read(cx).value().trim().to_string();
                    if matches!(current_port.as_str(), "1080" | "8080")
                        && let Some(window_handle) = cx.active_window()
                    {
                        let port = protocol.default_port().to_string();
                        let _ = window_handle.update(cx, move |_, window, cx| {
                            set_input_value(&port_input, port, window, cx);
                        });
                    }
                    cx.notify();
                }
            });
        let proxy_auth_mode_subscription =
            cx.subscribe(&proxy_auth_mode_select, |this: &mut Self, _, event, cx| {
                let SelectEvent::Confirm(selected) = event;
                if this.editing_proxy_id.is_some() && selected.is_some() {
                    cx.notify();
                }
            });

        let ssh_bridge_status = args.ssh_bridge_service.status();
        let ssh_bridge_sync_result = args.open_ssh_integration_service.last_sync_result();
        let ssh_bridge_security = args.ssh_bridge_service.security_snapshot();
        let ssh_bridge_notification_main_window = window.window_handle();
        let mut controller = Self {
            runtime: args.runtime,
            proxy_store: args.proxy_store,
            proxies: args.proxies,
            session_query,
            editing_proxy_id: None,
            proxy_resolve_dns_through_proxy: true,
            proxy_password_clear_requested: false,
            settings_store: args.settings_store,
            secrets: args.secrets,
            ssh_bridge_service: args.ssh_bridge_service,
            open_ssh_integration_service: args.open_ssh_integration_service,
            sync_executor: args.sync_executor,
            auto_sync_snapshot: args.auto_sync,
            ssh_bridge_status,
            ssh_bridge_sync_result,
            ssh_bridge_security,
            ssh_bridge_notification_main_window,
            ssh_bridge_notification_state: BridgeSecurityNotificationState::default(),
            ssh_bridge_notification_window: None,
            pending_ssh_bridge_policy_downgrade: None,
            ssh_bridge_settings_instance_generation: Cell::new(0),
            settings_destination_pending: Cell::new(None),
            forms,
            sync: bootstrap.sync,
            onboarding: bootstrap.onboarding,
            local_vault_status: bootstrap.local_vault_status,
            local_vault_operation_results: std::collections::VecDeque::new(),
            local_vault_operation_task: None,
            local_vault_unlock_in_progress: false,
            local_vault_disable_in_progress: false,
            local_vault_session_passphrase: None,
            recording_binding: None,
            pending_preview: None,
            pending_binding: None,
            editing_ai_provider_id: None,
            sync_direction: None,
            sync_pull_confirm: None,
            local_vault_disable_confirm: None,
            local_data_reset_confirm: None,
            local_data_reset_confirmation_popup: None,
            sync_passphrase_clear_confirm_popup: None,
            sync_passphrase_popup: None,
            ai_provider_popup: None,
            web_search_config_popup: None,
            sync_provider_config_popup: None,
            proxy_config_popup: None,
            local_vault_passphrase_popup: None,
            sync_provider_config_save_task: None,
            sync_passphrase_task: None,
            ai_provider_save_in_progress: false,
            ai_provider_save_task: None,
            web_search_save_in_progress: false,
            web_search_save_task: None,
            ai_provider_api_key_load_in_progress: None,
            ai_provider_api_key_load_tasks: std::collections::HashMap::new(),
            next_ai_provider_api_key_load_task_id: 0,
            local_data_reset_in_progress: false,
            local_data_reset_task: None,
            secret_visibility: SecretVisibilityState::default(),
            _subscriptions: vec![
                last_tab_close_behavior_subscription,
                window_close_behavior_subscription,
                local_vault_auto_lock_duration_subscription,
                monitor_history_subscription,
                terminal_right_click_behavior_subscription,
                open_ssh_integration_mode_subscription,
                ssh_bridge_security_level_subscription,
                sync_provider_select_subscription,
                ai_provider_select_subscription,
                ai_provider_kind_subscription,
                language_select_subscription,
                font_family_query_subscription,
                terminal_font_family_query_subscription,
                font_fallbacks_subscription,
                seed_color_subscription,
                web_search_kind_subscription,
                proxy_management_query_subscription,
                proxy_protocol_subscription,
                proxy_auth_mode_subscription,
            ],
        };
        controller.sync_ssh_bridge_security_level_select(window, cx);
        controller
    }

    pub(in crate::ui::shell) fn apply_application_bridge_snapshot(
        &mut self,
        status: SshBridgeStatus,
        sync_result: Option<SshBridgeSyncResult>,
        security: BridgeSecuritySnapshot,
        app_foreground: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bridge_running = matches!(status, SshBridgeStatus::Running { .. });
        self.sync_bridge_security_notification(&security, bridge_running, app_foreground, cx);
        let changed = status != self.ssh_bridge_status
            || sync_result != self.ssh_bridge_sync_result
            || security != self.ssh_bridge_security;
        self.ssh_bridge_status = status;
        self.ssh_bridge_sync_result = sync_result;
        if security != self.ssh_bridge_security {
            self.ssh_bridge_security = security;
            self.sync_ssh_bridge_security_level_select(window, cx);
        }
        if changed {
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn settings_store(&self) -> SettingsStore {
        self.settings_store.clone()
    }

    pub(in crate::ui::shell) fn runtime(&self) -> TokioHandle {
        self.runtime.clone()
    }

    pub(in crate::ui::shell) fn secrets(&self) -> SecretStore {
        self.secrets.clone()
    }

    pub(in crate::ui::shell) fn settings(&self) -> &AppSettings {
        self.settings_store.settings()
    }

    pub(in crate::ui::shell) fn ssh_bridge_status(&self) -> &SshBridgeStatus {
        &self.ssh_bridge_status
    }

    pub(in crate::ui::shell) fn ssh_bridge_sync_result(&self) -> Option<&SshBridgeSyncResult> {
        self.ssh_bridge_sync_result.as_ref()
    }

    pub(in crate::ui::shell) fn ssh_bridge_endpoint(&self) -> String {
        self.ssh_bridge_service.endpoint().to_string()
    }

    pub(in crate::ui::shell) fn open_ssh_config_path(&self) -> String {
        self.ssh_bridge_sync_result
            .as_ref()
            .map(|result| result.config_path.display().to_string())
            .unwrap_or_else(|| {
                self.open_ssh_integration_service
                    .config_path()
                    .display()
                    .to_string()
            })
    }

    pub(in crate::ui::shell) fn ssh_bridge_validation_diagnostics(&self) -> Vec<String> {
        self.ssh_bridge_service.route_refresh().diagnostics
    }

    pub(in crate::ui::shell) fn ssh_bridge_security(&self) -> &BridgeSecuritySnapshot {
        &self.ssh_bridge_security
    }

    pub(in crate::ui::shell) fn ssh_bridge_security_policy(&self) -> BridgeSecurityPolicy {
        self.ssh_bridge_service.security_policy()
    }

    pub(in crate::ui::shell) fn request_ssh_bridge_security_policy(
        &mut self,
        level: BridgeSecurityLevel,
        cx: &mut Context<Self>,
    ) {
        let current = self.ssh_bridge_service.security_policy().level;
        if bridge_security_rank(level) < bridge_security_rank(current) {
            self.pending_ssh_bridge_policy_downgrade =
                Some(PendingSshBridgePolicyDowngradeState { level });
            cx.notify();
            return;
        }
        self.apply_ssh_bridge_security_policy(level, cx);
    }

    fn apply_ssh_bridge_security_policy(
        &mut self,
        level: BridgeSecurityLevel,
        cx: &mut Context<Self>,
    ) {
        match self.ssh_bridge_service.set_security_policy(level) {
            Ok(_) => {
                self.ssh_bridge_security = self.ssh_bridge_service.security_snapshot();
                self.sync_ssh_bridge_security_level_select_via_active_window(cx);
                cx.emit(AppCommand::Feedback(i18n::string(
                    "settings.openssh_integration.security.policy_saved",
                )));
            }
            Err(error) => cx.emit(AppCommand::Feedback(i18n::string_args(
                "settings.openssh_integration.security.policy_failed",
                &[("error", &error.to_string())],
            ))),
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn pending_ssh_bridge_policy_downgrade(
        &self,
    ) -> Option<PendingSshBridgePolicyDowngradeState> {
        self.pending_ssh_bridge_policy_downgrade
    }

    fn sync_ssh_bridge_security_level_select(&mut self, window: &mut Window, cx: &mut App) {
        let level = self.ssh_bridge_service.security_policy().level;
        let system_auth_available = self.ssh_bridge_security.system_auth_available;
        let select = self.forms.ssh_bridge_security_level_select.clone();
        let options = [
            BridgeSecurityLevel::Standard,
            Self::default_bridge_approval_level(),
            BridgeSecurityLevel::RequireSystemAuth,
        ]
        .into_iter()
        .map(|level| {
            SelectOption::new(level, bridge_security_level_label(level)).disabled(
                matches!(level, BridgeSecurityLevel::RequireSystemAuth) && !system_auth_available,
            )
        })
        .collect::<Vec<_>>();
        select.update(cx, |select, cx| {
            select.set_items(options, window, cx);
            let index = match level {
                BridgeSecurityLevel::Standard => 0,
                BridgeSecurityLevel::RequireApproval { .. } => 1,
                BridgeSecurityLevel::RequireSystemAuth => 2,
            };
            select.set_selected_index(Some(IndexPath::default().row(index)), window, cx);
        });
    }

    fn sync_ssh_bridge_security_level_select_via_active_window(&mut self, cx: &mut App) {
        let Some(window) = cx.active_window() else {
            return;
        };
        let _ = window.update(cx, |_, window, cx| {
            self.sync_ssh_bridge_security_level_select(window, cx);
        });
    }

    pub(in crate::ui::shell) fn confirm_ssh_bridge_policy_downgrade(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        if let Some(prompt) = self.pending_ssh_bridge_policy_downgrade.take() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::SshBridgePolicyDowngrade(prompt),
            ));
            self.apply_ssh_bridge_security_policy(prompt.level, cx);
        }
    }

    pub(in crate::ui::shell) fn cancel_ssh_bridge_policy_downgrade(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        if let Some(prompt) = self.pending_ssh_bridge_policy_downgrade.take() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::SshBridgePolicyDowngrade(prompt),
            ));
        }
        self.sync_ssh_bridge_security_level_select_via_active_window(cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn approve_ssh_bridge_request(
        &mut self,
        request_id: String,
        cx: &mut Context<Self>,
    ) {
        self.decide_ssh_bridge_request(request_id, BridgeAuthorizationDecision::Approve, cx);
    }

    pub(in crate::ui::shell) fn reject_ssh_bridge_request(
        &mut self,
        request_id: String,
        cx: &mut Context<Self>,
    ) {
        self.decide_ssh_bridge_request(request_id, BridgeAuthorizationDecision::Reject, cx);
    }

    pub(in crate::ui::shell) fn cancel_ssh_bridge_request(
        &mut self,
        request_id: String,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.ssh_bridge_service.cancel_pending_request(&request_id) {
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "settings.openssh_integration.security.decision_failed",
                &[("error", &error.to_string())],
            )));
        }
        self.ssh_bridge_security = self.ssh_bridge_service.security_snapshot();
        cx.notify();
    }

    pub(in crate::ui::shell) fn unlock_ssh_bridge_vault(&mut self, cx: &mut Context<Self>) {
        self.request_ssh_bridge_security_page();
        if !self.local_vault_unlock_in_progress && self.local_vault_passphrase_popup.is_none() {
            cx.emit(AppCommand::vault_unlock_prompt());
        }
        cx.notify();
    }

    fn decide_ssh_bridge_request(
        &mut self,
        request_id: String,
        decision: BridgeAuthorizationDecision,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self
            .ssh_bridge_service
            .decide_authorization(&request_id, decision)
        {
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "settings.openssh_integration.security.decision_failed",
                &[("error", &error.to_string())],
            )));
        }
        self.ssh_bridge_security = self.ssh_bridge_service.security_snapshot();
        cx.notify();
    }

    pub(in crate::ui::shell) fn authenticate_ssh_bridge_request(
        &mut self,
        request_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self
            .ssh_bridge_security
            .pending
            .iter()
            .find(|request| request.request_id == request_id)
        else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "settings.openssh_integration.security.request_expired",
            )));
            return;
        };
        let source = bridge_security_display_text(
            request
                .peer
                .application_source_path()
                .unwrap_or("unknown process"),
        );
        let profile_name = bridge_security_display_text(&request.profile_name);
        let reason = format!(
            "Allow SSH Bridge request {} to profile {} from {}",
            request.request_id, profile_name, source
        );
        let service = self.ssh_bridge_service.clone();
        cx.spawn(async move |_this, _cx| {
            let outcome = crate::ui::bridge_security_platform::verify_system_auth(&reason).await;
            let decision = bridge_system_auth_decision(outcome);
            if let Err(error) = service.decide_authorization(&request_id, decision) {
                log::debug!("failed to finish SSH Bridge system authentication: {error:?}");
            }
        })
        .detach();
    }

    pub(in crate::ui::shell) fn default_bridge_approval_level() -> BridgeSecurityLevel {
        BridgeSecurityLevel::RequireApproval {
            timeout_secs: DEFAULT_BRIDGE_APPROVAL_TIMEOUT_SECS,
        }
    }

    pub(in crate::ui::shell) fn set_open_ssh_integration_mode(
        &mut self,
        mode: OpenSshIntegrationMode,
        cx: &mut Context<Self>,
    ) {
        let previous = self.settings_store.settings().open_ssh_integration_mode;
        if previous == mode {
            return;
        }
        let result = match apply_open_ssh_integration_mode_change(
            &mut self.settings_store,
            &self.open_ssh_integration_service,
            mode,
        ) {
            Ok(result) => result,
            Err(error) => {
                self.ssh_bridge_sync_result = self.open_ssh_integration_service.last_sync_result();
                let message = error.to_string();
                cx.emit(AppCommand::Feedback(i18n::string_args(
                    "settings.openssh_integration.notifications.sync_failed",
                    &[("error", &message)],
                )));
                if let Some(window_handle) = cx.active_window() {
                    let select = self.forms.open_ssh_integration_mode_select.clone();
                    let _ = window_handle.update(cx, move |_, window, cx| {
                        select.update(cx, |select, cx| {
                            select.set_selected_value(&previous, window, cx);
                        });
                    });
                }
                cx.notify();
                return;
            }
        };

        self.ssh_bridge_sync_result = Some(result);
        let bridge = self.ssh_bridge_service.clone();
        let integration = self.open_ssh_integration_service.clone();
        bridge.set_desired_enabled(matches!(
            ssh_bridge_lifecycle_action(mode),
            SshBridgeLifecycleAction::Enable
        ));
        self.runtime.spawn(async move {
            if let Err(error) = bridge.reconcile_desired_state().await {
                log::warn!("failed to reconcile SSH Bridge lifecycle: {error:?}");
                return;
            }
            if mode == OpenSshIntegrationMode::Bridge
                && matches!(bridge.status(), SshBridgeStatus::Running { .. })
                && let Err(error) = integration.set_mode(OpenSshIntegrationMode::Bridge)
            {
                log::warn!(
                    "failed to activate managed OpenSSH Bridge config after startup: {error:?}"
                );
            }
        });
        cx.emit(AppCommand::Feedback(i18n::string_args(
            "settings.openssh_integration.notifications.mode_changed",
            &[("mode", &open_ssh_integration_mode_label(mode))],
        )));
        cx.notify();
    }

    pub(in crate::ui::shell) fn proxies(&self) -> &[ProxyProfile] {
        &self.proxies
    }

    pub(in crate::ui::shell) fn replace_proxies(
        &mut self,
        proxies: Vec<ProxyProfile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected_proxy_id = self.selected_proxy_management_id(cx);
        self.proxies = proxies;
        self.refresh_proxy_management_select(selected_proxy_id.as_deref(), window, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn persist_agent_mode_preference(
        &mut self,
        mode: miaominal_settings::AiAgentMode,
        cx: &mut Context<Self>,
    ) {
        if self
            .settings_store
            .update(|settings| settings.agent_mode = mode)
        {
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn forms(&self) -> SettingsForms {
        self.forms.clone()
    }

    fn persist_selected_ai_provider(
        &mut self,
        provider_id: String,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut selected = false;
        if self.settings_store.update(|settings| {
            selected = select_ai_provider_setting(settings, &provider_id);
        }) {
            cx.notify();
        }
        selected
    }

    pub(in crate::ui::shell) fn select_ai_provider(
        &mut self,
        provider_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.persist_selected_ai_provider(provider_id.clone(), cx) {
            return false;
        }
        self.forms.ai_provider_select.update(cx, |select, cx| {
            select.set_selected_value(&provider_id, window, cx);
        });
        true
    }

    pub(in crate::ui::shell) fn set_ai_provider_reasoning_effort(
        &mut self,
        provider_id: &str,
        effort: AiReasoningEffort,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut updated = false;
        let changed = self.settings_store.update(|settings| {
            updated = set_ai_provider_reasoning_effort_setting(settings, provider_id, effort);
        });
        if changed {
            cx.notify();
        }
        updated
    }

    pub(in crate::ui::shell) fn request_profile_import(&self, cx: &mut Context<Self>) {
        let source = self
            .forms
            .profile_import_source_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(ImportSourceKind::OpenSshConfig);
        cx.emit(AppCommand::ImportProfilesRequested(source));
    }

    pub(in crate::ui::shell) fn onboarding_state(&self) -> OnboardingState {
        self.onboarding
    }

    pub(in crate::ui::shell) fn replace_onboarding_state(&mut self, onboarding: OnboardingState) {
        self.onboarding = onboarding;
    }

    pub(in crate::ui::shell) fn show_onboarding(&self) -> bool {
        self.onboarding.show_onboarding
    }

    pub(in crate::ui::shell) fn local_vault_status(&self) -> LocalVaultStatus {
        self.local_vault_status
    }

    pub(in crate::ui::shell) fn set_local_vault_status(&mut self, status: LocalVaultStatus) {
        self.local_vault_status = status;
    }

    pub(in crate::ui::shell) fn open_onboarding(&mut self, cx: &mut Context<Self>) {
        self.onboarding.show_onboarding = true;
        self.reset_onboarding_steps();
        cx.notify();
    }

    pub(in crate::ui::shell) fn finish_onboarding(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.can_finish_onboarding() {
            return false;
        }
        self.onboarding.show_onboarding = false;
        self.reset_onboarding_steps();
        let mut settings_store = self.settings_store.clone();
        settings_store.update(|settings| settings.mark_current_onboarding_completed());
        self.replace_settings_store(settings_store, cx);
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn advance_onboarding_step(&mut self, cx: &mut Context<Self>) -> bool {
        let portable = portable_vault_is_required();
        let Some(next_step) = self.onboarding.onboarding_step.next(portable) else {
            return false;
        };
        if !onboarding_step_is_allowed(portable, self.local_vault_status, next_step) {
            return false;
        }
        self.onboarding.onboarding_step = next_step;
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn set_onboarding_step(
        &mut self,
        step: OnboardingStep,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.onboarding.onboarding_step == step || !self.can_visit_onboarding_step(step) {
            return false;
        }
        self.onboarding.onboarding_step = step;
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn onboarding_steps(&self) -> &'static [OnboardingStep] {
        OnboardingStep::steps(portable_vault_is_required())
    }

    pub(in crate::ui::shell) fn can_visit_onboarding_step(&self, step: OnboardingStep) -> bool {
        onboarding_step_is_allowed(portable_vault_is_required(), self.local_vault_status, step)
    }

    pub(in crate::ui::shell) fn can_advance_onboarding(&self) -> bool {
        let portable = portable_vault_is_required();
        self.onboarding
            .onboarding_step
            .next(portable)
            .is_some_and(|step| onboarding_step_is_allowed(portable, self.local_vault_status, step))
    }

    pub(in crate::ui::shell) fn can_finish_onboarding(&self) -> bool {
        !portable_vault_is_required() || self.local_vault_status == LocalVaultStatus::Unlocked
    }

    fn reset_onboarding_steps(&mut self) {
        self.onboarding.onboarding_step = OnboardingStep::Welcome;
        self.onboarding.visible_onboarding_step = OnboardingStep::Welcome;
        self.onboarding.onboarding_step_transition = None;
    }

    pub(in crate::ui::shell) fn sync_engine(&self) -> &SyncEngine {
        &self.sync.sync_engine
    }

    pub(in crate::ui::shell) fn replace_sync_engine(&mut self, sync_engine: SyncEngine) {
        self.sync.sync_engine = sync_engine;
    }

    pub(in crate::ui::shell) fn sync_config(&self) -> &SyncConfig {
        &self.sync.sync_engine.config_store.config
    }

    pub(in crate::ui::shell) fn sync_status(&self) -> &SyncStatus {
        &self.sync.sync_status
    }

    pub(in crate::ui::shell) fn auto_sync_enabled(&self) -> bool {
        self.sync_config().auto_sync_enabled
    }

    pub(in crate::ui::shell) fn auto_sync_snapshot(&self) -> &miaominal_services::AutoSyncSnapshot {
        &self.auto_sync_snapshot
    }

    pub(in crate::ui::shell) fn apply_auto_sync_snapshot(
        &mut self,
        snapshot: miaominal_services::AutoSyncSnapshot,
        sync_engine: SyncEngine,
        cx: &mut Context<Self>,
    ) {
        self.replace_sync_engine(sync_engine);
        self.auto_sync_snapshot = snapshot;
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_auto_sync_enabled(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) =
            SettingsService::set_auto_sync_enabled(&mut self.sync.sync_engine, enabled)
        {
            log::warn!("failed to persist auto-sync preference: {error:?}");
            return;
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_sync_provider(
        &mut self,
        provider: SyncProvider,
    ) -> anyhow::Result<()> {
        SettingsService::set_sync_provider(&mut self.sync.sync_engine, provider)
    }

    pub(in crate::ui::shell) fn local_vault_unlock_in_progress(&self) -> bool {
        self.local_vault_unlock_in_progress
    }

    pub(in crate::ui::shell) fn local_vault_disable_in_progress(&self) -> bool {
        self.local_vault_disable_in_progress
    }

    pub(in crate::ui::shell) fn set_local_vault_session_passphrase(
        &mut self,
        passphrase: Option<ProtectedPassphrase>,
    ) {
        let previous = std::mem::replace(&mut self.local_vault_session_passphrase, passphrase);
        if let Some(previous) = previous
            && self
                .local_vault_session_passphrase
                .as_ref()
                .is_none_or(|current| !previous.shares_allocation_with(current))
        {
            previous.revoke();
        }
    }

    pub(in crate::ui::shell) fn editing_ai_provider_id(&self) -> Option<&str> {
        self.editing_ai_provider_id.as_deref()
    }

    pub(in crate::ui::shell) fn sync_direction(&self) -> Option<PendingSyncDirectionState> {
        self.sync_direction
    }

    pub(in crate::ui::shell) fn sync_pull_confirm(&self) -> Option<PendingSyncPullConfirmState> {
        self.sync_pull_confirm
    }

    pub(in crate::ui::shell) fn local_vault_disable_confirm(
        &self,
    ) -> Option<PendingLocalVaultDisableConfirmState> {
        self.local_vault_disable_confirm
    }

    pub(in crate::ui::shell) fn local_data_reset_confirm(
        &self,
    ) -> Option<PendingLocalDataResetConfirmState> {
        self.local_data_reset_confirm
    }

    pub(in crate::ui::shell) fn set_local_data_reset_confirm(
        &mut self,
        prompt: Option<PendingLocalDataResetConfirmState>,
    ) {
        self.local_data_reset_confirm = prompt;
    }

    pub(in crate::ui::shell) fn local_data_reset_confirmation_popup(
        &self,
    ) -> Option<PendingLocalDataResetConfirmationPopupState> {
        self.local_data_reset_confirmation_popup
    }

    pub(in crate::ui::shell) fn sync_passphrase_clear_confirm_popup(
        &self,
    ) -> Option<PendingSyncPassphraseClearConfirmPopupState> {
        self.sync_passphrase_clear_confirm_popup
    }

    pub(in crate::ui::shell) fn sync_passphrase_popup(
        &self,
    ) -> Option<PendingSyncPassphrasePopupState> {
        self.sync_passphrase_popup
    }

    pub(in crate::ui::shell) fn ai_provider_popup(&self) -> Option<PendingAiProviderPopupState> {
        self.ai_provider_popup
    }

    pub(in crate::ui::shell) fn web_search_config_popup(
        &self,
    ) -> Option<PendingWebSearchConfigPopupState> {
        self.web_search_config_popup
    }

    pub(in crate::ui::shell) fn sync_provider_config_popup(
        &self,
    ) -> Option<PendingSyncProviderConfigPopupState> {
        self.sync_provider_config_popup
    }

    pub(in crate::ui::shell) fn proxy_config_popup(&self) -> Option<PendingProxyConfigPopupState> {
        self.proxy_config_popup.clone()
    }

    pub(in crate::ui::shell) fn local_vault_passphrase_popup(
        &self,
    ) -> Option<LocalVaultPassphrasePopupMode> {
        self.local_vault_passphrase_popup
    }

    pub(in crate::ui::shell) fn ai_provider_save_in_progress(&self) -> bool {
        self.ai_provider_save_in_progress
    }

    pub(in crate::ui::shell) fn web_search_save_in_progress(&self) -> bool {
        self.web_search_save_in_progress
    }

    pub(in crate::ui::shell) fn local_data_reset_in_progress(&self) -> bool {
        self.local_data_reset_in_progress
    }

    pub(in crate::ui::shell) fn replace_settings_store(
        &mut self,
        settings_store: SettingsStore,
        cx: &mut Context<Self>,
    ) {
        self.settings_store = settings_store;
        cx.notify();
    }

    pub(in crate::ui::shell) fn replace_application_settings(
        &mut self,
        settings_store: SettingsStore,
        sync_engine: SyncEngine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_settings_store(settings_store, cx);
        self.replace_sync_engine(sync_engine);
        let settings = self.settings_store.settings().clone();

        self.forms.language_select.update(cx, |select, cx| {
            select.set_selected_value(&settings.language, window, cx);
        });
        self.forms
            .last_tab_close_behavior_select
            .update(cx, |select, cx| {
                select.set_selected_value(&settings.last_tab_close_behavior, window, cx);
            });
        self.forms
            .window_close_behavior_select
            .update(cx, |select, cx| {
                select.set_selected_value(&settings.window_close_behavior, window, cx);
            });
        self.forms
            .local_vault_auto_lock_duration_select
            .update(cx, |select, cx| {
                select.set_selected_value(&settings.local_vault_auto_lock_duration, window, cx);
            });
        self.forms.monitor_history_select.update(cx, |select, cx| {
            select.set_selected_value(&settings.monitor_history_duration, window, cx);
        });
        self.forms
            .terminal_right_click_behavior_select
            .update(cx, |select, cx| {
                select.set_selected_value(&settings.terminal_right_click_behavior, window, cx);
            });
        self.forms
            .open_ssh_integration_mode_select
            .update(cx, |select, cx| {
                select.set_selected_value(&settings.open_ssh_integration_mode, window, cx);
            });
        self.forms.sync_provider_select.update(cx, |select, cx| {
            select.set_selected_value(
                &self.sync.sync_engine.config_store.config.provider,
                window,
                cx,
            );
        });

        let ai_provider_options = ai_provider_select_options(&settings);
        let selected_ai_provider = settings
            .selected_ai_provider_id
            .as_ref()
            .filter(|selected| {
                ai_provider_options
                    .iter()
                    .any(|option| option.value() == *selected)
            })
            .cloned()
            .or_else(|| {
                ai_provider_options
                    .first()
                    .map(|option| option.value().clone())
            });
        self.forms.ai_provider_select.update(cx, |select, cx| {
            select.set_items(ai_provider_options, window, cx);
            if let Some(selected) = selected_ai_provider.as_ref() {
                select.set_selected_value(selected, window, cx);
            } else {
                select.set_selected_index(None, window, cx);
            }
        });
        self.forms.web_search_kind_select.update(cx, |select, cx| {
            select.set_selected_value(&settings.web_search.kind, window, cx);
        });
        set_input_value(
            &self.forms.font_fallbacks_input,
            settings.font_fallbacks.join(", "),
            window,
            cx,
        );
        let source = miaominal_settings::Theme::from_settings(&settings)
            .material
            .source;
        self.forms.seed_color_picker.update(cx, |picker, cx| {
            picker.set_value(rgb(source), window, cx);
        });
        self.sync_ssh_bridge_security_level_select(window, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn update_font_family(
        &mut self,
        value: String,
        cx: &mut Context<Self>,
    ) -> bool {
        let trimmed = value.trim();
        let next = if trimmed.is_empty() {
            miaominal_settings::default_font_family()
        } else {
            trimmed.to_string()
        };

        let changed = self
            .settings_store
            .update(|settings| settings.font_family = next.clone());
        if changed {
            miaominal_settings::sync_component_theme(cx);
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "status.interface_font_set",
                &[("font", &next)],
            )));
            cx.notify();
        }
        changed
    }

    pub(in crate::ui::shell) fn reset_font_family(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let default_font = miaominal_settings::default_font_family();
        let changed = self
            .settings_store
            .update(|settings| settings.font_family = default_font.clone());
        set_input_value(&self.forms.font_family_query_input, "", window, cx);
        if changed {
            miaominal_settings::sync_component_theme(cx);
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "status.interface_font_reset",
                &[("font", &default_font)],
            )));
            cx.notify();
        }
        changed
    }

    pub(in crate::ui::shell) fn update_terminal_font_family(
        &mut self,
        value: String,
        cx: &mut Context<Self>,
    ) -> bool {
        let trimmed = value.trim();
        let next = if trimmed.is_empty() {
            miaominal_settings::default_font_family()
        } else {
            trimmed.to_string()
        };

        let changed = self
            .settings_store
            .update(|settings| settings.terminal_font_family = next.clone());
        if changed {
            miaominal_settings::sync_component_theme(cx);
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "status.terminal_font_set",
                &[("font", &next)],
            )));
            cx.notify();
        }
        changed
    }

    pub(in crate::ui::shell) fn reset_terminal_font_family(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let default_font = miaominal_settings::default_font_family();
        let changed = self.settings_store.update(|settings| {
            settings.terminal_font_family = default_font.clone();
        });
        set_input_value(&self.forms.terminal_font_family_query_input, "", window, cx);
        if changed {
            miaominal_settings::sync_component_theme(cx);
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "status.terminal_font_reset",
                &[("font", &default_font)],
            )));
            cx.notify();
        }
        changed
    }

    pub(in crate::ui::shell) fn update_font_fallbacks(
        &mut self,
        value: String,
        cx: &mut Context<Self>,
    ) -> bool {
        let fallbacks = value
            .split(',')
            .map(|fallback| fallback.trim().to_string())
            .filter(|fallback| !fallback.is_empty())
            .collect();
        let changed = self
            .settings_store
            .update(|settings| settings.font_fallbacks = fallbacks);
        if changed {
            cx.notify();
        }
        changed
    }

    pub(in crate::ui::shell) fn reset_font_fallbacks(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let defaults = miaominal_settings::default_font_fallbacks();
        let value = defaults.join(", ");
        let changed = self
            .settings_store
            .update(|settings| settings.font_fallbacks = defaults);
        set_input_value(&self.forms.font_fallbacks_input, value, window, cx);
        if changed {
            cx.notify();
        }
        changed
    }

    pub(in crate::ui::shell) fn adjust_font_size(
        &mut self,
        delta: f32,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(target) = SettingsService::adjust_font_size(&mut self.settings_store, delta)
        else {
            return false;
        };
        miaominal_settings::sync_component_theme(cx);
        let value = format!("{target:.1}");
        cx.emit(AppCommand::Feedback(i18n::string_args(
            "status.font_size",
            &[("value", &value)],
        )));
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn adjust_line_height(
        &mut self,
        delta: f32,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(target) = SettingsService::adjust_line_height(&mut self.settings_store, delta)
        else {
            return false;
        };
        miaominal_settings::sync_component_theme(cx);
        let value = format!("{target:.1}");
        cx.emit(AppCommand::Feedback(i18n::string_args(
            "status.line_height",
            &[("value", &value)],
        )));
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn update_seed_color(
        &mut self,
        normalized: String,
        cx: &mut Context<Self>,
    ) -> bool {
        let changed = self
            .settings_store
            .update(|settings| settings.seed_color = normalized.clone());
        if changed {
            miaominal_settings::sync_component_theme(cx);
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "status.theme_seed",
                &[("value", &normalized)],
            )));
            cx.notify();
        }
        changed
    }

    pub(in crate::ui::shell) fn reset_seed_color(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let default_seed = crate::ui::theme::DEFAULT_SEED_COLOR.to_string();
        let changed = self
            .settings_store
            .update(|settings| settings.seed_color = default_seed.clone());
        let default_color =
            miaominal_settings::Theme::from_settings(self.settings_store.settings())
                .material
                .source;
        self.forms.seed_color_picker.update(cx, |picker, cx| {
            picker.set_value(rgb(default_color), window, cx);
        });
        if changed {
            miaominal_settings::sync_component_theme(cx);
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "status.theme_seed_reset",
                &[("value", &default_seed)],
            )));
            cx.notify();
        }
        changed
    }

    pub(in crate::ui::shell) fn set_theme(&mut self, theme_id: ThemeId, cx: &mut Context<Self>) {
        if self
            .settings_store
            .update(|settings| settings.theme_id = theme_id)
        {
            miaominal_settings::sync_component_theme(cx);
            let theme = theme_id_label(theme_id);
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "status.theme_changed",
                &[("theme", &theme)],
            )));
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn set_language(
        &mut self,
        language: AppLanguage,
        cx: &mut Context<Self>,
    ) {
        if self
            .settings_store
            .update(|settings| settings.language = language)
        {
            i18n::set_language(language);
            crate::ui::sync_system_tray(cx);
            cx.emit(AppCommand::LocaleRefresh);
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "status.language_changed",
                &[("language", language.native_name())],
            )));
            cx.notify();
        }
    }

    fn on_web_search_kind_changed(&mut self, kind: WebSearchProviderKind, cx: &mut Context<Self>) {
        if let Some(window_handle) = cx.active_window()
            && let Err(error) = window_handle.update(cx, |_, window, cx| {
                set_input_value(&self.forms.web_search_api_key_input, "", window, cx);
                set_input_placeholder(
                    &self.forms.web_search_endpoint_input,
                    web_search_endpoint_placeholder(kind),
                    window,
                    cx,
                );
                let api_key_placeholder = if self.settings_store.settings().web_search.has_api_key {
                    i18n::string("placeholders.saved.keep_existing")
                } else {
                    i18n::string("settings.web_search.placeholders.api_key")
                };
                set_input_placeholder(
                    &self.forms.web_search_api_key_input,
                    api_key_placeholder,
                    window,
                    cx,
                );
            })
        {
            log::debug!("failed to update web search form after provider change: {error:?}");
        }
        self.secret_visibility
            .set_visible(SecretRevealTarget::WebSearchApiKey, false);
        cx.emit(AppCommand::Feedback(i18n::string_args(
            "settings.web_search.status.kind_selected",
            &[(
                "kind",
                &i18n::string(web_search_provider_kind_label_key(kind)),
            )],
        )));
        cx.notify();
    }

    pub(in crate::ui::shell) fn adjust_recent_connections_count(
        &mut self,
        delta: i16,
        cx: &mut Context<Self>,
    ) {
        let current = self.settings_store.settings().recent_connections_count as i16;
        let next = (current + delta).clamp(
            miaominal_settings::RECENT_CONNECTIONS_COUNT_MIN as i16,
            miaominal_settings::RECENT_CONNECTIONS_COUNT_MAX as i16,
        ) as u8;
        if self
            .settings_store
            .update(|settings| settings.recent_connections_count = next)
        {
            let message = if next == 0 {
                i18n::string("status.recent_connections_hidden")
            } else {
                let count = next.to_string();
                i18n::string_args("status.recent_connections_show_count", &[("count", &count)])
            };
            cx.emit(AppCommand::Feedback(message));
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn set_auto_collect_session_monitoring(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let changed = self
            .settings_store
            .update(|settings| settings.auto_collect_session_monitoring = enabled);
        if changed {
            let message = if enabled {
                i18n::string("status.auto_collect_session_monitoring_enabled")
            } else {
                i18n::string("status.auto_collect_session_monitoring_disabled")
            };
            cx.emit(AppCommand::Feedback(message));
            cx.notify();
        }
        changed
    }

    pub(in crate::ui::shell) fn recording_binding(&self) -> Option<KeyBindingSlot> {
        self.recording_binding
    }

    pub(in crate::ui::shell) fn pending_preview(&self) -> Option<&str> {
        self.pending_preview.as_deref()
    }

    pub(in crate::ui::shell) fn pending_binding(&self) -> Option<&KeyBinding> {
        self.pending_binding.as_ref()
    }

    pub(in crate::ui::shell) fn begin_recording_key_binding(
        &mut self,
        slot: KeyBindingSlot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.recording_binding = Some(slot);
        self.pending_preview = None;
        self.pending_binding = None;
        self.forms.key_capture_focus.focus(window, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn commit_recorded_key_binding(
        &mut self,
        binding: KeyBinding,
        cx: &mut Context<Self>,
    ) {
        self.pending_preview = None;
        self.pending_binding = None;
        let Some(slot) = self.recording_binding.take() else {
            return;
        };
        let changed = self.settings_store.update(|settings| match slot {
            KeyBindingSlot::NextTab => settings.key_bindings.next_tab = binding.clone(),
            KeyBindingSlot::CloseTab => settings.key_bindings.close_tab = binding.clone(),
            KeyBindingSlot::ReopenTab => settings.key_bindings.reopen_tab = binding.clone(),
            KeyBindingSlot::OpenSettings => settings.key_bindings.open_settings = binding.clone(),
            KeyBindingSlot::Copy => settings.key_bindings.copy = binding.clone(),
            KeyBindingSlot::Paste => settings.key_bindings.paste = binding.clone(),
            KeyBindingSlot::Search => settings.key_bindings.search = binding.clone(),
            KeyBindingSlot::SplitRight => settings.key_bindings.split_right = binding.clone(),
            KeyBindingSlot::SplitDown => settings.key_bindings.split_down = binding.clone(),
            KeyBindingSlot::ClosePane => settings.key_bindings.close_pane = binding.clone(),
        });
        if changed {
            let name = slot.label();
            let binding = binding.display();
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "status.key_binding_updated",
                &[("name", &name), ("binding", &binding)],
            )));
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn cancel_recording_key_binding(&mut self, cx: &mut Context<Self>) {
        self.pending_preview = None;
        self.pending_binding = None;
        if self.recording_binding.take().is_some() {
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn accept_pending_key_binding(&mut self, cx: &mut Context<Self>) {
        let Some(binding) = self.pending_binding.take() else {
            return;
        };
        self.commit_recorded_key_binding(binding, cx);
    }

    pub(in crate::ui::shell) fn update_key_preview(
        &mut self,
        preview: String,
        binding: Option<KeyBinding>,
        cx: &mut Context<Self>,
    ) {
        self.pending_preview = Some(preview);
        self.pending_binding = binding;
        cx.notify();
    }

    pub(in crate::ui::shell) fn reset_key_binding(
        &mut self,
        slot: KeyBindingSlot,
        cx: &mut Context<Self>,
    ) {
        let defaults = TerminalKeyBindings::default();
        let default_binding = match slot {
            KeyBindingSlot::NextTab => defaults.next_tab,
            KeyBindingSlot::CloseTab => defaults.close_tab,
            KeyBindingSlot::ReopenTab => defaults.reopen_tab,
            KeyBindingSlot::OpenSettings => defaults.open_settings,
            KeyBindingSlot::Copy => defaults.copy,
            KeyBindingSlot::Paste => defaults.paste,
            KeyBindingSlot::Search => defaults.search,
            KeyBindingSlot::SplitRight => defaults.split_right,
            KeyBindingSlot::SplitDown => defaults.split_down,
            KeyBindingSlot::ClosePane => defaults.close_pane,
        };
        let changed = self.settings_store.update(|settings| match slot {
            KeyBindingSlot::NextTab => settings.key_bindings.next_tab = default_binding.clone(),
            KeyBindingSlot::CloseTab => settings.key_bindings.close_tab = default_binding.clone(),
            KeyBindingSlot::ReopenTab => settings.key_bindings.reopen_tab = default_binding.clone(),
            KeyBindingSlot::OpenSettings => {
                settings.key_bindings.open_settings = default_binding.clone()
            }
            KeyBindingSlot::Copy => settings.key_bindings.copy = default_binding.clone(),
            KeyBindingSlot::Paste => settings.key_bindings.paste = default_binding.clone(),
            KeyBindingSlot::Search => settings.key_bindings.search = default_binding.clone(),
            KeyBindingSlot::SplitRight => {
                settings.key_bindings.split_right = default_binding.clone()
            }
            KeyBindingSlot::SplitDown => settings.key_bindings.split_down = default_binding.clone(),
            KeyBindingSlot::ClosePane => settings.key_bindings.close_pane = default_binding.clone(),
        });
        if changed {
            let name = slot.label();
            let binding = default_binding.display();
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "status.key_binding_reset",
                &[("name", &name), ("binding", &binding)],
            )));
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_terminal_shift_right_click_context_menu(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .settings_store
            .update(|settings| settings.terminal_shift_right_click_context_menu = enabled);
        if changed {
            let message = if enabled {
                i18n::string("status.shift_right_click_enabled")
            } else {
                i18n::string("status.shift_right_click_disabled")
            };
            cx.emit(AppCommand::Feedback(message));
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn set_terminal_free_type_mode(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .settings_store
            .update(|settings| settings.terminal_free_type_mode = enabled);
        if changed {
            let message = if enabled {
                i18n::string("status.free_type_mode_enabled")
            } else {
                i18n::string("status.free_type_mode_disabled")
            };
            cx.emit(AppCommand::Feedback(message));
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn persist_sftp_browser_hidden_columns(
        &mut self,
        side: SftpBrowserSide,
        hidden_columns: Vec<usize>,
        cx: &mut Context<Self>,
    ) {
        let changed = match side {
            SftpBrowserSide::Local => self
                .settings_store
                .update(|settings| settings.local_sftp_hidden_columns = hidden_columns),
            SftpBrowserSide::Remote => self
                .settings_store
                .update(|settings| settings.remote_sftp_hidden_columns = hidden_columns),
        };

        if changed {
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn update_credentials(
        &mut self,
        secrets: SecretStore,
        local_vault_status: LocalVaultStatus,
        cx: &mut Context<Self>,
    ) {
        self.secrets = secrets;
        self.ssh_bridge_service
            .replace_secrets(self.secrets.clone());
        self.local_vault_status = local_vault_status;
        if let Some(proxy_store) = &self.proxy_store {
            match proxy_store.load(&self.secrets) {
                Ok(proxies) if proxies != self.proxies => {
                    self.proxies = proxies.clone();
                    cx.emit(AppCommand::ProxiesChanged(proxies));
                }
                Ok(_) => {}
                Err(error) => {
                    log::warn!("failed to refresh proxies after credentials changed: {error:?}");
                }
            }
        }
        cx.notify();
    }
}

impl EventEmitter<AppCommand> for SettingsController {}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::profile::SessionProfile;
    use miaominal_settings::{AiProviderConfig, SshBridgeConfig};
    use miaominal_ssh::SshBridgeEndpoint;
    use miaominal_storage::{BridgeAuditLog, BridgeSecuritySettingsStore, KnownHostsStore};
    use tokio::runtime::Runtime;

    #[test]
    fn system_auth_results_keep_cancelled_and_unavailable_semantics() {
        use crate::ui::bridge_security_platform::SystemAuthVerification;

        assert_eq!(
            bridge_system_auth_decision(SystemAuthVerification::Verified),
            BridgeAuthorizationDecision::SystemAuthVerified
        );
        assert_eq!(
            bridge_system_auth_decision(SystemAuthVerification::Canceled),
            BridgeAuthorizationDecision::SystemAuthCancelled
        );
        assert_eq!(
            bridge_system_auth_decision(SystemAuthVerification::Unavailable),
            BridgeAuthorizationDecision::SystemAuthUnavailable
        );
        for outcome in [
            SystemAuthVerification::Busy,
            SystemAuthVerification::RetriesExhausted,
            SystemAuthVerification::Failed,
        ] {
            assert_eq!(
                bridge_system_auth_decision(outcome),
                BridgeAuthorizationDecision::SystemAuthFailed
            );
        }
    }

    #[test]
    fn portable_onboarding_inserts_security_before_import() {
        assert_eq!(
            OnboardingStep::steps(false),
            &[
                OnboardingStep::Welcome,
                OnboardingStep::Preferences,
                OnboardingStep::Import,
                OnboardingStep::Finish,
            ]
        );
        assert_eq!(
            OnboardingStep::steps(true),
            &[
                OnboardingStep::Welcome,
                OnboardingStep::Preferences,
                OnboardingStep::Security,
                OnboardingStep::Import,
                OnboardingStep::Finish,
            ]
        );
        assert_eq!(
            OnboardingStep::Preferences.next(false),
            Some(OnboardingStep::Import)
        );
        assert_eq!(
            OnboardingStep::Preferences.next(true),
            Some(OnboardingStep::Security)
        );
    }

    #[test]
    fn portable_onboarding_blocks_steps_after_security_until_vault_is_unlocked() {
        for status in [LocalVaultStatus::Disabled, LocalVaultStatus::Locked] {
            assert!(onboarding_step_is_allowed(
                true,
                status,
                OnboardingStep::Security
            ));
            assert!(!onboarding_step_is_allowed(
                true,
                status,
                OnboardingStep::Import
            ));
            assert!(!onboarding_step_is_allowed(
                true,
                status,
                OnboardingStep::Finish
            ));
        }

        assert!(onboarding_step_is_allowed(
            true,
            LocalVaultStatus::Unlocked,
            OnboardingStep::Finish
        ));
        assert!(onboarding_step_is_allowed(
            false,
            LocalVaultStatus::Disabled,
            OnboardingStep::Finish
        ));
        assert!(!onboarding_step_is_allowed(
            false,
            LocalVaultStatus::Unlocked,
            OnboardingStep::Security
        ));
    }

    #[test]
    fn bridge_lifecycle_follows_open_ssh_integration_mode() {
        assert_eq!(
            ssh_bridge_lifecycle_action(OpenSshIntegrationMode::Bridge),
            SshBridgeLifecycleAction::Enable
        );
        for mode in [
            OpenSshIntegrationMode::Disabled,
            OpenSshIntegrationMode::Direct,
        ] {
            assert_eq!(
                ssh_bridge_lifecycle_action(mode),
                SshBridgeLifecycleAction::Disable
            );
        }
    }

    #[test]
    fn failed_mode_persistence_rolls_back_projection_and_in_memory_settings() {
        let runtime = Runtime::new().unwrap();
        let root = tempfile::tempdir().unwrap();
        let ssh = tempfile::tempdir().unwrap();
        let settings_directory = tempfile::tempdir().unwrap();
        let endpoint = SshBridgeEndpoint::derive(root.path()).unwrap();
        let instance_id = SshBridgeEndpoint::instance_id(root.path()).unwrap();
        let bridge = SshBridgeService::new_with_stores(
            runtime.handle().clone(),
            endpoint,
            instance_id.clone(),
            ssh.path()
                .join("miaominal")
                .join(&instance_id)
                .join("bridge_known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(root.path().join("upstream_known_hosts")),
            BridgeSecuritySettingsStore::open(&root.path().join("settings.toml"))
                .map_err(|error| format!("{error:#}")),
            BridgeAuditLog::open(&root.path().join("ssh_bridge_audit.log"))
                .map_err(|error| format!("{error:#}")),
        );
        let integration =
            OpenSshIntegrationService::new(bridge, ssh.path().to_path_buf(), instance_id);
        let mut profile = SessionProfile::blank("production", 1);
        profile.name = "Production".into();
        profile.host = "example.com".into();
        profile.username = "akko".into();
        integration
            .sync(OpenSshIntegrationMode::Disabled, vec![profile], vec![])
            .unwrap();

        let settings_path = settings_directory.path().join("settings.toml");
        let mut settings_store = SettingsStore::load_with_path(settings_path.clone()).unwrap();
        std::fs::create_dir(&settings_path).unwrap();
        let error = apply_open_ssh_integration_mode_change(
            &mut settings_store,
            &integration,
            OpenSshIntegrationMode::Bridge,
        )
        .expect_err("a settings path occupied by a directory must fail persistence");

        assert!(error.to_string().contains("restored Disabled"));
        assert_eq!(
            settings_store.settings().open_ssh_integration_mode,
            OpenSshIntegrationMode::Disabled
        );
        assert_eq!(integration.mode(), OpenSshIntegrationMode::Disabled);
        assert!(!integration.config_path().exists());
        assert!(!ssh.path().join("config").exists());
    }

    #[test]
    fn failed_bridge_disable_persistence_restores_running_host_key_sidecar() {
        let runtime = Runtime::new().unwrap();
        let root = tempfile::tempdir().unwrap();
        let ssh = tempfile::tempdir().unwrap();
        let settings_directory = tempfile::tempdir().unwrap();
        let endpoint = SshBridgeEndpoint::derive(root.path()).unwrap();
        let instance_id = SshBridgeEndpoint::instance_id(root.path()).unwrap();
        let known_hosts_path = ssh
            .path()
            .join("miaominal")
            .join(&instance_id)
            .join("bridge_known_hosts");
        let bridge = SshBridgeService::new_with_stores(
            runtime.handle().clone(),
            endpoint,
            instance_id.clone(),
            known_hosts_path.clone(),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(root.path().join("upstream_known_hosts")),
            BridgeSecuritySettingsStore::open(&root.path().join("settings.toml"))
                .map_err(|error| format!("{error:#}")),
            BridgeAuditLog::open(&root.path().join("ssh_bridge_audit.log"))
                .map_err(|error| format!("{error:#}")),
        );
        let integration =
            OpenSshIntegrationService::new(bridge.clone(), ssh.path().to_path_buf(), instance_id);
        let mut profile = SessionProfile::blank("production", 1);
        profile.name = "Production".into();
        profile.host = "example.com".into();
        profile.username = "akko".into();
        runtime.block_on(bridge.enable()).unwrap();
        integration
            .sync(OpenSshIntegrationMode::Bridge, vec![profile], vec![])
            .unwrap();
        let original_sidecar = std::fs::read(&known_hosts_path).unwrap();

        let settings_path = settings_directory.path().join("settings.toml");
        let mut settings_store = SettingsStore::load_with_path(settings_path.clone()).unwrap();
        let mut bridge_settings = settings_store.settings().clone();
        bridge_settings.open_ssh_integration_mode = OpenSshIntegrationMode::Bridge;
        settings_store.replace(bridge_settings).unwrap();
        std::fs::remove_file(&settings_path).unwrap();
        std::fs::create_dir(&settings_path).unwrap();

        let error = apply_open_ssh_integration_mode_change(
            &mut settings_store,
            &integration,
            OpenSshIntegrationMode::Disabled,
        )
        .expect_err("a settings path occupied by a directory must fail persistence");

        assert!(error.to_string().contains("restored Bridge"));
        assert_eq!(
            settings_store.settings().open_ssh_integration_mode,
            OpenSshIntegrationMode::Bridge
        );
        assert_eq!(integration.mode(), OpenSshIntegrationMode::Bridge);
        assert_eq!(std::fs::read(&known_hosts_path).unwrap(), original_sidecar);
        runtime.block_on(bridge.disable());
    }

    #[test]
    fn provider_selection_preserves_each_provider_reasoning_effort() {
        let mut openai = AiProviderConfig::new(AiProviderKind::OpenAi);
        openai.id = "openai".to_string();
        let mut anthropic = AiProviderConfig::new(AiProviderKind::Anthropic);
        anthropic.id = "anthropic".to_string();
        let mut settings = AppSettings {
            ai_providers: vec![openai, anthropic],
            ..AppSettings::default()
        };

        assert!(select_ai_provider_setting(&mut settings, "openai"));
        assert!(set_ai_provider_reasoning_effort_setting(
            &mut settings,
            "openai",
            AiReasoningEffort::Low
        ));
        assert!(select_ai_provider_setting(&mut settings, "anthropic"));
        assert!(set_ai_provider_reasoning_effort_setting(
            &mut settings,
            "anthropic",
            AiReasoningEffort::High
        ));
        assert!(select_ai_provider_setting(&mut settings, "openai"));

        assert_eq!(settings.selected_ai_provider_id.as_deref(), Some("openai"));
        assert_eq!(
            settings.ai_providers[0].reasoning_effort,
            AiReasoningEffort::Low
        );
        assert_eq!(
            settings.ai_providers[1].reasoning_effort,
            AiReasoningEffort::High
        );
    }
}
