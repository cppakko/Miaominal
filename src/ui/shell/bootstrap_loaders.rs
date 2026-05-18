use super::*;
use crate::services::LoadedAppData as LoadedServiceData;

impl AppView {
    pub(in crate::ui::shell) fn load_app_data(
        runtime: TokioHandle,
        local_vault_enabled: bool,
    ) -> LoadedAppData {
        let LoadedServiceData {
            services,
            known_hosts_entries,
            managed_keys,
            sessions,
            snippets,
            selected_profile,
            status_message,
        } = AppServices::load(runtime, local_vault_enabled);

        LoadedAppData {
            data: AppDataState {
                known_hosts_entries,
                managed_keys,
                agent_identities: Vec::new(),
                sessions,
                snippets,
                selected_profile,
                selected_snippet: None,
            },
            services,
            status_message,
        }
    }

    pub(in crate::ui::shell) fn initial_profile_selection(
        data: &AppDataState,
    ) -> InitialProfileSelection {
        let selected_profile_data = data
            .selected_profile
            .and_then(|index| data.sessions.get(index).cloned());
        let editing_auth_method = selected_profile_data
            .as_ref()
            .map(SessionProfile::effective_auth_method)
            .map(Self::host_editor_auth_method)
            .unwrap_or_default();
        let available_groups = Self::collect_available_groups(&data.sessions);
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
}

pub(in crate::ui::shell) struct LoadedAppData {
    pub(in crate::ui::shell) services: AppServices,
    pub(in crate::ui::shell) data: AppDataState,
    pub(in crate::ui::shell) status_message: String,
}

pub(in crate::ui::shell) struct InitialProfileSelection {
    pub(in crate::ui::shell) selected_profile_data: Option<SessionProfile>,
    pub(in crate::ui::shell) editing_auth_method: AuthMethod,
    pub(in crate::ui::shell) available_groups: Vec<String>,
    pub(in crate::ui::shell) selected_group: String,
    pub(in crate::ui::shell) selected_existing_group: Option<String>,
}
