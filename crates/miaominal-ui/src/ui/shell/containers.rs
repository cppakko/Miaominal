use super::{PortForwardKind, ProfileViewMode, SidebarSection, TrustedHostFilter};
use gpui::Subscription;
use miaominal_core::keychain::ManagedKeyRecord;
use miaominal_core::known_host::KnownHostEntry;
use miaominal_core::profile::SessionProfile;
use miaominal_core::snippet::SnippetRecord;
use miaominal_ssh::AgentIdentitySummary;

pub(in crate::ui::shell) struct AppDataState {
    pub known_hosts_entries: Vec<KnownHostEntry>,
    pub managed_keys: Vec<ManagedKeyRecord>,
    pub agent_identities: Vec<AgentIdentitySummary>,
    pub sessions: Vec<SessionProfile>,
    pub snippets: Vec<SnippetRecord>,
    pub selected_profile: Option<usize>,
    pub selected_snippet: Option<usize>,
}

pub(in crate::ui::shell) struct EditorOverlayState {
    pub host_editor_open: bool,
    pub host_editor_is_new: bool,
    pub snippets_editor_open: bool,
    pub keychain_editor_open: bool,
    pub port_forward_editor_open: bool,
    pub port_forward_editor_profile_id: Option<String>,
    pub port_forward_editor_rule_id: Option<String>,
    pub port_forward_kind: PortForwardKind,
}

impl EditorOverlayState {
    pub fn new() -> Self {
        Self {
            host_editor_open: false,
            host_editor_is_new: false,
            snippets_editor_open: false,
            keychain_editor_open: false,
            port_forward_editor_open: false,
            port_forward_editor_profile_id: None,
            port_forward_editor_rule_id: None,
            port_forward_kind: PortForwardKind::Local,
        }
    }
}

pub(in crate::ui::shell) struct PanelViewState {
    pub sidebar_section: SidebarSection,
    pub hosts_view_mode: ProfileViewMode,
    pub forward_view_mode: ProfileViewMode,
    pub snippets_view_mode: ProfileViewMode,
    pub hosts_group_filter: Option<String>,
    pub trusted_host_filter: TrustedHostFilter,
    pub snippets_package_filter: Option<String>,
}

impl PanelViewState {
    pub fn new() -> Self {
        Self {
            sidebar_section: SidebarSection::Hosts,
            hosts_view_mode: ProfileViewMode::Grid,
            forward_view_mode: ProfileViewMode::Grid,
            snippets_view_mode: ProfileViewMode::Grid,
            hosts_group_filter: None,
            trusted_host_filter: TrustedHostFilter::All,
            snippets_package_filter: None,
        }
    }
}

pub(in crate::ui::shell) struct AppViewSubscriptions {
    pub _font_family_subscription: Subscription,
    pub _font_fallbacks_subscription: Subscription,
    pub _language_select_subscription: Subscription,
    pub _last_tab_close_behavior_select_subscription: Subscription,
    pub _local_vault_auto_lock_duration_select_subscription: Subscription,
    pub _monitor_history_select_subscription: Subscription,
    pub _terminal_right_click_behavior_select_subscription: Subscription,
    pub _sync_provider_select_subscription: Subscription,
    pub _ai_provider_select_subscription: Subscription,
    pub _ai_provider_kind_select_subscription: Subscription,
    pub _web_search_kind_select_subscription: Subscription,
    pub _seed_color_subscription: Subscription,
    pub _group_input_subscription: Subscription,
    pub _group_select_subscription: Subscription,
    pub _managed_key_select_subscription: Subscription,
    pub _proxy_jump_select_subscription: Subscription,
    pub _snippet_package_select_subscription: Subscription,
    pub _keychain_filter_input_subscription: Subscription,
    pub _filter_input_subscription: Subscription,
    pub _trusted_filter_input_subscription: Subscription,
    pub _forward_filter_input_subscription: Subscription,
    pub _snippet_filter_input_subscription: Subscription,
    pub _forward_profile_select_subscription: Subscription,
    pub _local_sftp_path_input_subscription: Subscription,
    pub _remote_sftp_path_input_subscription: Subscription,
    pub _local_sftp_table_subscription: Subscription,
    pub _remote_sftp_table_subscription: Subscription,
    pub _rename_input_subscription: Subscription,
    pub _sftp_prompt_input_subscription: Subscription,
    pub _sftp_inline_rename_input_subscription: Subscription,
    pub _search_input_subscription: Subscription,
    pub _session_snippets_filter_input_subscription: Subscription,
    pub _terminal_focus_in_subscription: Subscription,
    pub _terminal_focus_out_subscription: Subscription,
    pub _window_activation_subscription: Subscription,
    pub _terminal_keystroke_interceptor: Subscription,
}
