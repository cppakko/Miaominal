use super::*;

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

pub(in crate::ui::shell) struct InitialProfileSelection {
    pub(in crate::ui::shell) selected_profile_data: Option<SessionProfile>,
    pub(in crate::ui::shell) editing_auth_method: AuthMethod,
    pub(in crate::ui::shell) available_groups: Vec<String>,
    pub(in crate::ui::shell) selected_group: String,
    pub(in crate::ui::shell) selected_existing_group: Option<String>,
}
