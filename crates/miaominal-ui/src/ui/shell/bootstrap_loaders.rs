use super::*;
use miaominal_services::{ChatService, LoadedAppData as LoadedServiceData};
use miaominal_storage::chat_store::ChatSessionRecord;

pub(in crate::ui::shell) fn load_app_data(
    runtime: TokioHandle,
    local_vault_enabled: bool,
) -> LoadedAppData {
    let LoadedServiceData {
        services,
        known_hosts_entries,
        managed_keys,
        chat_service,
        chat_sessions,
        sessions,
        snippets,
        selected_profile,
        status_message,
    } = AppServices::load(runtime, local_vault_enabled);

    LoadedAppData {
        profiles: sessions,
        selected_profile,
        known_hosts_entries,
        snippets,
        selected_snippet: None,
        managed_keys,
        chat_service,
        chat_sessions,
        services,
        status_message,
    }
}

pub(in crate::ui::shell) fn initial_profile_selection(
    profiles: &[SessionProfile],
    selected_profile: Option<usize>,
) -> InitialProfileSelection {
    let selected_profile_data = selected_profile.and_then(|index| profiles.get(index).cloned());
    let editing_auth_method = selected_profile_data
        .as_ref()
        .map(SessionProfile::effective_auth_method)
        .map(SessionController::host_editor_auth_method)
        .unwrap_or_default();
    let available_groups = SessionController::collect_available_groups(profiles);
    let selected_group = selected_profile_data
        .as_ref()
        .map(|profile| profile.group.trim().to_string())
        .unwrap_or_default();
    let selected_existing_group = available_groups
        .iter()
        .find(|group| group.eq_ignore_ascii_case(selected_group.as_str()))
        .cloned();

    InitialProfileSelection {
        selected_profile_data,
        editing_auth_method,
        available_groups,
        selected_group,
        selected_existing_group,
    }
}

pub(in crate::ui::shell) struct LoadedAppData {
    pub(in crate::ui::shell) services: AppServices,
    pub(in crate::ui::shell) profiles: Vec<SessionProfile>,
    pub(in crate::ui::shell) selected_profile: Option<usize>,
    pub(in crate::ui::shell) known_hosts_entries: Vec<miaominal_core::known_host::KnownHostEntry>,
    pub(in crate::ui::shell) snippets: Vec<SnippetRecord>,
    pub(in crate::ui::shell) selected_snippet: Option<usize>,
    pub(in crate::ui::shell) managed_keys: Vec<ManagedKeyRecord>,
    pub(in crate::ui::shell) chat_service: Option<ChatService>,
    pub(in crate::ui::shell) chat_sessions: Vec<ChatSessionRecord>,
    pub(in crate::ui::shell) status_message: String,
}

pub(in crate::ui::shell) struct InitialProfileSelection {
    pub(in crate::ui::shell) selected_profile_data: Option<SessionProfile>,
    pub(in crate::ui::shell) editing_auth_method: AuthMethod,
    pub(in crate::ui::shell) available_groups: Vec<String>,
    pub(in crate::ui::shell) selected_group: String,
    pub(in crate::ui::shell) selected_existing_group: Option<String>,
}
