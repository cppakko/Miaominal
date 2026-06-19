use super::*;
use crate::ui::i18n;
use miaominal_core::profile::ImportSourceKind;
use miaominal_settings::{
    AiProviderKind, AppLanguage, LastTabCloseBehavior, LocalVaultAutoLockDuration,
    MonitorHistoryDuration, TerminalRightClickBehavior, WebSearchProviderKind,
};
use miaominal_sync::SyncProvider;

#[derive(Clone, Debug)]
pub(in crate::ui::shell) struct SelectOption<T> {
    title: SharedString,
    value: T,
}

impl<T> SelectOption<T> {
    pub(in crate::ui::shell) fn new(value: T, title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            value,
        }
    }

    pub(in crate::ui::shell) fn value(&self) -> &T {
        &self.value
    }
}

impl<T: Clone + PartialEq> SelectItem for SelectOption<T> {
    type Value = T;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

pub(in crate::ui::shell) struct TerminalSearchForms {
    pub(in crate::ui::shell) input: Entity<InputState>,
    pub(in crate::ui::shell) open: bool,
    pub(in crate::ui::shell) visible: bool,
    pub(in crate::ui::shell) visibility: f32,
    pub(in crate::ui::shell) animation: Option<TerminalSearchAnimation>,
    pub(in crate::ui::shell) total: usize,
    pub(in crate::ui::shell) current: Option<usize>,
    pub(in crate::ui::shell) status: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::ui::shell) struct TerminalSearchAnimation {
    pub(in crate::ui::shell) started_at: Instant,
    pub(in crate::ui::shell) duration: Duration,
    pub(in crate::ui::shell) from: f32,
    pub(in crate::ui::shell) to: f32,
}

pub(in crate::ui::shell) struct SftpBrowserForms {
    pub(in crate::ui::shell) local_path_input: Entity<InputState>,
    pub(in crate::ui::shell) remote_path_input: Entity<InputState>,
    pub(in crate::ui::shell) local_path_editing: bool,
    pub(in crate::ui::shell) remote_path_editing: bool,
    pub(in crate::ui::shell) remote_path_submit_pending: bool,
    pub(in crate::ui::shell) local_table: Entity<TableState<SftpBrowserTableDelegate>>,
    pub(in crate::ui::shell) remote_table: Entity<TableState<SftpBrowserTableDelegate>>,
    pub(in crate::ui::shell) prompt_input: Entity<InputState>,
    pub(in crate::ui::shell) inline_rename_input: Entity<InputState>,
}

pub(in crate::ui::shell) struct WorkspaceForms {
    pub(in crate::ui::shell) rename_input: Entity<InputState>,
    pub(in crate::ui::shell) search: TerminalSearchForms,
    pub(in crate::ui::shell) chat_search: ChatSearchForms,
    pub(in crate::ui::shell) agent: WorkspaceAgentForms,
    pub(in crate::ui::shell) snippets_panel: WorkspaceSnippetsForms,
    pub(in crate::ui::shell) sftp_browser: SftpBrowserForms,
}

pub(in crate::ui::shell) struct WorkspaceAgentForms {
    pub(in crate::ui::shell) prompt_input: Entity<InputState>,
    pub(in crate::ui::shell) title_input: Entity<InputState>,
    pub(in crate::ui::shell) editing_title: bool,
}

pub(in crate::ui::shell) struct WorkspaceSnippetsForms {
    pub(in crate::ui::shell) filter_input: Entity<InputState>,
    pub(in crate::ui::shell) selected_package_filter: Option<String>,
}

pub(in crate::ui::shell) struct HostEditorForms {
    pub(in crate::ui::shell) name_input: Entity<InputState>,
    pub(in crate::ui::shell) group_input: Entity<InputState>,
    pub(in crate::ui::shell) group_select: Entity<SelectState<SearchableVec<String>>>,
    pub(in crate::ui::shell) managed_key_select:
        Entity<SelectState<SearchableVec<ManagedKeySelectItem>>>,
    pub(in crate::ui::shell) proxy_jump_select:
        Entity<SelectState<SearchableVec<ProxyJumpCandidateSelectItem>>>,
    pub(in crate::ui::shell) charset_select: Entity<SelectState<SearchableVec<String>>>,
    pub(in crate::ui::shell) creating_new_group: bool,
    pub(in crate::ui::shell) tags_input: Entity<InputState>,
    pub(in crate::ui::shell) host_input: Entity<InputState>,
    pub(in crate::ui::shell) port_input: Entity<InputState>,
    pub(in crate::ui::shell) username_input: Entity<InputState>,
    pub(in crate::ui::shell) password_input: Entity<InputState>,
    pub(in crate::ui::shell) private_key_input: Entity<InputState>,
    pub(in crate::ui::shell) agent_identity_input: Entity<InputState>,
    pub(in crate::ui::shell) certificate_input: Entity<InputState>,
    pub(in crate::ui::shell) passphrase_input: Entity<InputState>,
    pub(in crate::ui::shell) startup_command_input: Entity<InputState>,
    pub(in crate::ui::shell) proxy_jump_profile_ids: Vec<String>,
    pub(in crate::ui::shell) selected_proxy_jump_hop: Option<usize>,
    pub(in crate::ui::shell) environment_variable_rows: Vec<HostEditorEnvironmentVariableRow>,
    pub(in crate::ui::shell) shell_type: ShellType,
    pub(in crate::ui::shell) editing_auth_method: AuthMethod,
    pub(in crate::ui::shell) agent_forwarding_enabled: bool,
}

pub(in crate::ui::shell) struct HostsForms {
    pub(in crate::ui::shell) filter_input: Entity<InputState>,
}

pub(in crate::ui::shell) struct TrustedHostsForms {
    pub(in crate::ui::shell) filter_input: Entity<InputState>,
}

pub(in crate::ui::shell) struct KeychainForms {
    pub(in crate::ui::shell) filter_input: Entity<InputState>,
    pub(in crate::ui::shell) name_input: Entity<InputState>,
    pub(in crate::ui::shell) import_path_input: Entity<InputState>,
    pub(in crate::ui::shell) import_private_key_input: Entity<InputState>,
    pub(in crate::ui::shell) import_public_key_input: Entity<InputState>,
    pub(in crate::ui::shell) import_passphrase_input: Entity<InputState>,
    pub(in crate::ui::shell) deploy_profile_select:
        Entity<SelectState<SearchableVec<ForwardProfileSelectItem>>>,
    pub(in crate::ui::shell) deploy_location_input: Entity<InputState>,
    pub(in crate::ui::shell) deploy_filename_input: Entity<InputState>,
    pub(in crate::ui::shell) deploy_command_input: Entity<InputState>,
}

pub(in crate::ui::shell) struct PortForwardingForms {
    pub(in crate::ui::shell) filter_input: Entity<InputState>,
    pub(in crate::ui::shell) profile_select:
        Entity<SelectState<SearchableVec<ForwardProfileSelectItem>>>,
    pub(in crate::ui::shell) label_input: Entity<InputState>,
    pub(in crate::ui::shell) listen_host_input: Entity<InputState>,
    pub(in crate::ui::shell) listen_port_input: Entity<InputState>,
    pub(in crate::ui::shell) target_host_input: Entity<InputState>,
    pub(in crate::ui::shell) target_port_input: Entity<InputState>,
}

pub(in crate::ui::shell) struct SnippetsForms {
    pub(in crate::ui::shell) filter_input: Entity<InputState>,
    pub(in crate::ui::shell) description_input: Entity<InputState>,
    pub(in crate::ui::shell) package_input: Entity<InputState>,
    pub(in crate::ui::shell) package_select: Entity<SelectState<SearchableVec<String>>>,
    pub(in crate::ui::shell) creating_new_package: bool,
    pub(in crate::ui::shell) script_input: Entity<InputState>,
}

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
    pub(in crate::ui::shell) recording_binding: Option<KeyBindingSlot>,
    pub(in crate::ui::shell) pending_preview: Option<String>,
    pub(in crate::ui::shell) pending_binding: Option<KeyBinding>,
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
    pub(in crate::ui::shell) web_search_api_key_input: Entity<InputState>,
    pub(in crate::ui::shell) web_search_endpoint_input: Entity<InputState>,
    pub(in crate::ui::shell) web_search_max_results_input: Entity<InputState>,
    pub(in crate::ui::shell) editing_ai_provider_id: Option<String>,
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
            KeyBindingSlot::NextTab => "settings.key_bindings.slots.next_tab.label",
            KeyBindingSlot::CloseTab => "settings.key_bindings.slots.close_tab.label",
            KeyBindingSlot::ReopenTab => "settings.key_bindings.slots.reopen_tab.label",
            KeyBindingSlot::OpenSettings => "settings.key_bindings.slots.open_settings.label",
            KeyBindingSlot::Copy => "settings.key_bindings.slots.copy.label",
            KeyBindingSlot::Paste => "settings.key_bindings.slots.paste.label",
            KeyBindingSlot::Search => "settings.key_bindings.slots.search.label",
            KeyBindingSlot::SplitRight => "settings.key_bindings.slots.split_right.label",
            KeyBindingSlot::SplitDown => "settings.key_bindings.slots.split_down.label",
            KeyBindingSlot::ClosePane => "settings.key_bindings.slots.close_pane.label",
        }
    }

    fn description_key(self) -> &'static str {
        match self {
            KeyBindingSlot::NextTab => "settings.key_bindings.slots.next_tab.description",
            KeyBindingSlot::CloseTab => "settings.key_bindings.slots.close_tab.description",
            KeyBindingSlot::ReopenTab => "settings.key_bindings.slots.reopen_tab.description",
            KeyBindingSlot::OpenSettings => "settings.key_bindings.slots.open_settings.description",
            KeyBindingSlot::Copy => "settings.key_bindings.slots.copy.description",
            KeyBindingSlot::Paste => "settings.key_bindings.slots.paste.description",
            KeyBindingSlot::Search => "settings.key_bindings.slots.search.description",
            KeyBindingSlot::SplitRight => "settings.key_bindings.slots.split_right.description",
            KeyBindingSlot::SplitDown => "settings.key_bindings.slots.split_down.description",
            KeyBindingSlot::ClosePane => "settings.key_bindings.slots.close_pane.description",
        }
    }

    pub(in crate::ui::shell) fn label(self) -> String {
        i18n::string(self.label_key())
    }

    pub(in crate::ui::shell) fn description(self) -> String {
        i18n::string(self.description_key())
    }
}

pub(in crate::ui::shell) struct ChatSearchForms {
    pub(in crate::ui::shell) session_filter_input: Entity<InputState>,
    pub(in crate::ui::shell) session_filter_open: bool,
    pub(in crate::ui::shell) conversation_search_input: Entity<InputState>,
    pub(in crate::ui::shell) conversation_search_open: bool,
    pub(in crate::ui::shell) conversation_search_visible: bool,
    pub(in crate::ui::shell) conversation_search_visibility: f32,
    pub(in crate::ui::shell) conversation_search_animation: Option<TerminalSearchAnimation>,
    pub(in crate::ui::shell) match_count: usize,
    pub(in crate::ui::shell) current_match: Option<usize>,
    pub(in crate::ui::shell) status: Option<String>,
}

pub(in crate::ui::shell) struct PanelForms {
    pub(in crate::ui::shell) hosts: HostsForms,
    pub(in crate::ui::shell) trusted: TrustedHostsForms,
    pub(in crate::ui::shell) keychain: KeychainForms,
    pub(in crate::ui::shell) forwarding: PortForwardingForms,
    pub(in crate::ui::shell) snippets: SnippetsForms,
    pub(in crate::ui::shell) settings: SettingsForms,
}
