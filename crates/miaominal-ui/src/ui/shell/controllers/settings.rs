use super::AppCommand;
use crate::ui::i18n;
use crate::ui::shell::actions::ai_provider_kind_chat_supported;
use crate::ui::shell::{
    LocalVaultPassphrasePopupMode, LocalVaultStatus, SecretRevealTarget, SelectOption,
    SftpBrowserSide, ai_provider_kind_label_key, ai_provider_select_options,
    last_tab_close_behavior_label, local_vault_auto_lock_duration_label,
    localized_profile_import_source_label, localized_secret_placeholder,
    monitor_history_duration_label, new_input_state, set_input_placeholder, set_input_value,
    theme_id_label, web_search_endpoint_placeholder, web_search_provider_kind_label_key,
};
use anyhow::Result;
use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, Subscription, Window, rgb,
};
use gpui_component::{
    Colorize, IndexPath,
    color_picker::{ColorPickerEvent, ColorPickerState},
    input::{InputEvent, InputState},
    select::{SearchableVec, SelectEvent, SelectState},
};
use miaominal_core::profile::ImportSourceKind;
use miaominal_secrets::{ProtectedPassphrase, SecretStore};
use miaominal_services::{
    LocalVaultPassphraseChangeOutcome, LocalVaultTransition, SettingsService, SyncService,
};
use miaominal_settings::{
    AiProviderKind, AppLanguage, AppSettings, KeyBinding, LastTabCloseBehavior,
    LocalVaultAutoLockDuration, MonitorHistoryDuration, TerminalKeyBindings,
    TerminalRightClickBehavior, ThemeId, WebSearchProviderKind,
};
use miaominal_storage::{
    SettingsStore,
    config_store::store::{SessionStore, SnippetStore},
    keychain_store::ManagedKeyStore,
};
use miaominal_sync::{SyncConfig, SyncProvider, SyncStatus, engine::SyncEngine};
use std::time::{Duration, Instant};
use tokio::runtime::Handle as TokioHandle;

mod ai_providers;
mod local_data_reset;
mod local_vault;
mod secret_visibility;
mod sync;
mod web_search;

pub(in crate::ui::shell) use ai_providers::AiProviderSaveDraft;
pub(in crate::ui::shell) use local_vault::{
    LocalVaultActionRequest, LocalVaultChangePassphraseResult, LocalVaultEnableResult,
    LocalVaultOperationResult, LocalVaultUnlockResult,
};
pub(in crate::ui::shell) use sync::{LocalVaultSyncSecretInputs, SyncProviderConfigSaveDraft};
pub(in crate::ui::shell) use web_search::WebSearchSaveDraft;

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
pub(in crate::ui::shell) struct PendingSyncProviderConfigPopupState {
    pub(in crate::ui::shell) provider: SyncProvider,
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
    pub(in crate::ui::shell) active_sync_task: Option<gpui::Task<()>>,
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
    pub(in crate::ui::shell) local_vault_auto_lock_duration_select:
        Entity<SelectState<Vec<SelectOption<LocalVaultAutoLockDuration>>>>,
    pub(in crate::ui::shell) monitor_history_select:
        Entity<SelectState<Vec<SelectOption<MonitorHistoryDuration>>>>,
    pub(in crate::ui::shell) terminal_right_click_behavior_select:
        Entity<SelectState<Vec<SelectOption<TerminalRightClickBehavior>>>>,
    pub(in crate::ui::shell) profile_import_source_select:
        Entity<SelectState<Vec<SelectOption<ImportSourceKind>>>>,
    pub(in crate::ui::shell) sync_provider_select:
        Entity<SelectState<Vec<SelectOption<SyncProvider>>>>,
    pub(in crate::ui::shell) ai_provider_select: Entity<SelectState<Vec<SelectOption<String>>>>,
    pub(in crate::ui::shell) ai_provider_kind_select:
        Entity<SelectState<Vec<SelectOption<AiProviderKind>>>>,
    pub(in crate::ui::shell) web_search_kind_select:
        Entity<SelectState<Vec<SelectOption<WebSearchProviderKind>>>>,
    pub(in crate::ui::shell) font_family_select: Entity<SelectState<SearchableVec<String>>>,
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
}

pub(in crate::ui::shell) struct SettingsControllerArgs {
    pub runtime: TokioHandle,
    pub session_store: Option<SessionStore>,
    pub snippet_store: Option<SnippetStore>,
    pub keychain_store: Option<ManagedKeyStore>,
    pub settings_store: SettingsStore,
    pub secrets: SecretStore,
}

struct SettingsBootstrap {
    forms: SettingsForms,
    sync: SyncUiState,
    onboarding: OnboardingState,
    local_vault_status: LocalVaultStatus,
}

pub(in crate::ui::shell) struct SettingsController {
    runtime: TokioHandle,
    session_store: Option<SessionStore>,
    snippet_store: Option<SnippetStore>,
    keychain_store: Option<ManagedKeyStore>,
    settings_store: SettingsStore,
    secrets: SecretStore,
    pub(in crate::ui::shell) forms: SettingsForms,
    sync: SyncUiState,
    onboarding: OnboardingState,
    local_vault_status: LocalVaultStatus,
    local_vault_operation_results: std::collections::VecDeque<LocalVaultOperationResult>,
    local_vault_operation_task: Option<gpui::Task<()>>,
    local_vault_unlock_in_progress: bool,
    local_vault_disable_in_progress: bool,
    local_vault_session_passphrase: Option<ProtectedPassphrase>,
    local_vault_auto_lock_task: Option<gpui::Task<()>>,
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
    local_vault_passphrase_popup: Option<LocalVaultPassphrasePopupMode>,
    sync_provider_config_save_task: Option<gpui::Task<()>>,
    sync_passphrase_task: Option<gpui::Task<()>>,
    ai_provider_save_in_progress: bool,
    ai_provider_save_task: Option<gpui::Task<()>>,
    web_search_save_in_progress: bool,
    web_search_save_task: Option<gpui::Task<()>>,
    ai_provider_api_key_load_in_progress: Option<String>,
    ai_provider_api_key_load_tasks: std::collections::HashMap<u64, gpui::Task<()>>,
    next_ai_provider_api_key_load_task_id: u64,
    local_data_reset_in_progress: bool,
    local_data_reset_task: Option<gpui::Task<()>>,
    secret_visibility: SecretVisibilityState,
    _subscriptions: Vec<Subscription>,
}

impl SettingsController {
    fn font_family_options(current_font_family: &str) -> Vec<String> {
        let mut families = miaominal_settings::available_font_families();
        let default_font_family = miaominal_settings::default_font_family();
        if !families
            .iter()
            .any(|family| family.eq_ignore_ascii_case(&default_font_family))
        {
            families.push(default_font_family);
        }

        let trimmed_current = current_font_family.trim();
        if !trimmed_current.is_empty()
            && !families
                .iter()
                .any(|family| family.eq_ignore_ascii_case(trimmed_current))
        {
            families.push(trimmed_current.to_string());
        }

        families.sort_by_cached_key(|family| family.to_ascii_lowercase());
        families.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        families
    }

    fn build_bootstrap(
        settings_store: &SettingsStore,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> SettingsBootstrap {
        let settings = settings_store.settings();
        let local_vault_enabled = settings.local_vault_enabled;
        let sync_engine = if local_vault_enabled {
            SyncEngine::new_locked_vault()
        } else {
            SyncEngine::new()
        };
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
        let current_font_family = settings.font_family.clone();
        let font_family_options = Self::font_family_options(&current_font_family);
        let default_font_family = miaominal_settings::default_font_family();
        let font_family_select = cx.new(|cx| {
            let mut state =
                SelectState::new(SearchableVec::new(font_family_options), None, window, cx)
                    .searchable(true);
            let selected = if current_font_family.trim().is_empty() {
                default_font_family
            } else {
                current_font_family
            };
            state.set_selected_value(&selected, window, cx);
            state
        });
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
            font_family_select,
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
            local_vault_status: if local_vault_enabled {
                LocalVaultStatus::Locked
            } else {
                LocalVaultStatus::Disabled
            },
        }
    }

    pub(in crate::ui::shell) fn refresh_localized_placeholders(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (input, key) in [
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

    pub(in crate::ui::shell) fn new(
        args: SettingsControllerArgs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let bootstrap = Self::build_bootstrap(&args.settings_store, window, cx);
        let forms = bootstrap.forms;
        let last_tab_close_behavior_select = forms.last_tab_close_behavior_select.clone();
        let local_vault_auto_lock_duration_select =
            forms.local_vault_auto_lock_duration_select.clone();
        let monitor_history_select = forms.monitor_history_select.clone();
        let terminal_right_click_behavior_select =
            forms.terminal_right_click_behavior_select.clone();
        let sync_provider_select = forms.sync_provider_select.clone();
        let ai_provider_select = forms.ai_provider_select.clone();
        let ai_provider_kind_select = forms.ai_provider_kind_select.clone();
        let language_select = forms.language_select.clone();
        let font_family_select = forms.font_family_select.clone();
        let font_fallbacks_input = forms.font_fallbacks_input.clone();
        let seed_color_picker = forms.seed_color_picker.clone();
        let web_search_kind_select = forms.web_search_kind_select.clone();

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
                    this.sync_local_vault_auto_lock_task(cx);
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
                if this.settings_store.update(|settings| {
                    settings.selected_ai_provider_id = Some(provider_id);
                }) {
                    cx.notify();
                }
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
        let font_family_subscription =
            cx.subscribe(&font_family_select, |this: &mut Self, _, event, cx| {
                let SelectEvent::Confirm(selected) = event;
                if let Some(font_family) = selected.as_deref() {
                    this.update_font_family(font_family.to_string(), cx);
                }
            });
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

        Self {
            runtime: args.runtime,
            session_store: args.session_store,
            snippet_store: args.snippet_store,
            keychain_store: args.keychain_store,
            settings_store: args.settings_store,
            secrets: args.secrets,
            forms,
            sync: bootstrap.sync,
            onboarding: bootstrap.onboarding,
            local_vault_status: bootstrap.local_vault_status,
            local_vault_operation_results: std::collections::VecDeque::new(),
            local_vault_operation_task: None,
            local_vault_unlock_in_progress: false,
            local_vault_disable_in_progress: false,
            local_vault_session_passphrase: None,
            local_vault_auto_lock_task: None,
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
                local_vault_auto_lock_duration_subscription,
                monitor_history_subscription,
                terminal_right_click_behavior_subscription,
                sync_provider_select_subscription,
                ai_provider_select_subscription,
                ai_provider_kind_subscription,
                language_select_subscription,
                font_family_subscription,
                font_fallbacks_subscription,
                seed_color_subscription,
                web_search_kind_subscription,
            ],
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

    pub(in crate::ui::shell) fn sync_service(&self) -> Result<SyncService> {
        SyncService::new(
            self.runtime.clone(),
            self.session_store.clone(),
            self.snippet_store.clone(),
            self.keychain_store.clone(),
            self.secrets.clone(),
        )
    }

    pub(in crate::ui::shell) fn settings(&self) -> &AppSettings {
        self.settings_store.settings()
    }

    pub(in crate::ui::shell) fn forms(&self) -> SettingsForms {
        self.forms.clone()
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

    pub(in crate::ui::shell) fn finish_onboarding(&mut self, cx: &mut Context<Self>) {
        self.onboarding.show_onboarding = false;
        self.reset_onboarding_steps();
        let mut settings_store = self.settings_store.clone();
        settings_store.update(|settings| settings.mark_current_onboarding_completed());
        self.replace_settings_store(settings_store, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn advance_onboarding_step(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(next_step) = self.onboarding.onboarding_step.next() else {
            return false;
        };
        self.onboarding.onboarding_step = next_step;
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn set_onboarding_step(
        &mut self,
        step: OnboardingStep,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.onboarding.onboarding_step == step {
            return false;
        }
        self.onboarding.onboarding_step = step;
        cx.notify();
        true
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
        let auto_lock_duration_changed = self
            .settings_store
            .settings()
            .local_vault_auto_lock_duration
            != settings_store.settings().local_vault_auto_lock_duration;
        self.settings_store = settings_store;
        if auto_lock_duration_changed {
            self.sync_local_vault_auto_lock_task(cx);
        }
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
                "status.font_set",
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
        self.forms.font_family_select.update(cx, |select, cx| {
            select.set_selected_value(&default_font, window, cx);
        });
        if changed {
            miaominal_settings::sync_component_theme(cx);
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "status.font_reset",
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
        self.local_vault_status = local_vault_status;
        cx.notify();
    }
}

impl EventEmitter<AppCommand> for SettingsController {}
