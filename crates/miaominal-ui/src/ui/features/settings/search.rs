//! Public labels only: searching settings never constructs editors or reads credentials.
use super::page::{SettingsGroupId, SettingsPageId};
use crate::ui::{
    contracts::search::{SearchEntry, SearchKind, SearchTarget},
    i18n,
};

struct SearchGroup {
    page: SettingsPageId,
    group: SettingsGroupId,
    title: &'static str,
    items: &'static [&'static str],
}

// Group order mirrors each settings page; destinations resolve through named IDs.
const GROUPS: &[SearchGroup] = &[
    SearchGroup {
        page: SettingsPageId::Appearance,
        group: SettingsGroupId::Language,
        title: "settings.appearance.language_group.title",
        items: &["settings.appearance.language.label"],
    },
    SearchGroup {
        page: SettingsPageId::Appearance,
        group: SettingsGroupId::Typography,
        title: "settings.appearance.typography_group.title",
        items: &[
            "settings.appearance.font_family.label",
            "settings.appearance.terminal_font_family.label",
            "settings.appearance.font_fallbacks.label",
            "settings.appearance.interface_font_size.label",
            "settings.appearance.terminal_font_size.label",
            "settings.appearance.line_height.label",
        ],
    },
    SearchGroup {
        page: SettingsPageId::Appearance,
        group: SettingsGroupId::Theme,
        title: "settings.appearance.theme_group.title",
        items: &[
            "settings.appearance.dark_mode.label",
            "settings.appearance.seed_color.label",
        ],
    },
    SearchGroup {
        page: SettingsPageId::Connections,
        group: SettingsGroupId::RecentConnections,
        title: "settings.connections.recent_group.title",
        items: &["settings.connections.recent_count.label"],
    },
    SearchGroup {
        page: SettingsPageId::Connections,
        group: SettingsGroupId::Monitoring,
        title: "settings.connections.monitor_group.title",
        items: &[
            "settings.connections.auto_collect_monitoring.label",
            "settings.connections.monitor_history.label",
        ],
    },
    SearchGroup {
        page: SettingsPageId::Connections,
        group: SettingsGroupId::Tabs,
        title: "settings.connections.tabs_group.title",
        items: &[
            "settings.connections.last_tab_close_behavior.label",
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            "settings.connections.window_close_behavior.label",
        ],
    },
    SearchGroup {
        page: SettingsPageId::Connections,
        group: SettingsGroupId::Rdp,
        title: "rdp.settings_title",
        items: &["rdp.client"],
    },
    SearchGroup {
        page: SettingsPageId::Connections,
        group: SettingsGroupId::Proxies,
        title: "settings.proxies.group.title",
        items: &["settings.proxies.saved.label"],
    },
    SearchGroup {
        page: SettingsPageId::Connections,
        group: SettingsGroupId::Import,
        title: "settings.connections.import_group.title",
        items: &[
            "settings.connections.import_source.label",
            "settings.connections.import_action.label",
        ],
    },
    SearchGroup {
        page: SettingsPageId::OpenSshIntegration,
        group: SettingsGroupId::SshBridgeIntegration,
        title: "settings.openssh_integration.group.title",
        items: &[
            "settings.openssh_integration.mode.label",
            "settings.openssh_integration.status.label",
            "settings.openssh_integration.endpoint.label",
            "settings.openssh_integration.config_path.label",
            "settings.openssh_integration.profiles.label",
            "settings.openssh_integration.active_clients.label",
            "settings.openssh_integration.credentials.label",
            "settings.openssh_integration.validation.label",
            "settings.openssh_integration.last_error.label",
        ],
    },
    SearchGroup {
        page: SettingsPageId::OpenSshIntegration,
        group: SettingsGroupId::SshBridgeSecurityPolicy,
        title: "settings.openssh_integration.security_policy_group.title",
        items: &[
            "settings.openssh_integration.security.health_label",
            "settings.openssh_integration.security.policies_label",
        ],
    },
    SearchGroup {
        page: SettingsPageId::OpenSshIntegration,
        group: SettingsGroupId::SshBridgeSecurityPending,
        title: "settings.openssh_integration.security_pending_group.title",
        items: &["settings.openssh_integration.security.pending_label"],
    },
    SearchGroup {
        page: SettingsPageId::KeyBindings,
        group: SettingsGroupId::WorkspaceKeys,
        title: "settings.key_bindings.workspace_group.title",
        items: &[
            "settings.key_bindings.slots.next_tab.label",
            "settings.key_bindings.slots.close_tab.label",
            "settings.key_bindings.slots.reopen_tab.label",
            "settings.key_bindings.slots.open_settings.label",
        ],
    },
    SearchGroup {
        page: SettingsPageId::KeyBindings,
        group: SettingsGroupId::TerminalKeys,
        title: "settings.key_bindings.terminal_group.title",
        items: &[
            "settings.key_bindings.slots.copy.label",
            "settings.key_bindings.slots.paste.label",
            "settings.key_bindings.slots.search.label",
            "settings.key_bindings.slots.split_right.label",
            "settings.key_bindings.slots.split_down.label",
            "settings.key_bindings.slots.close_pane.label",
        ],
    },
    SearchGroup {
        page: SettingsPageId::KeyBindings,
        group: SettingsGroupId::Mouse,
        title: "settings.key_bindings.mouse_group.title",
        items: &[
            "settings.key_bindings.free_type_mode.label",
            "settings.key_bindings.right_click.label",
            "settings.key_bindings.shift_right_click.label",
        ],
    },
    SearchGroup {
        page: SettingsPageId::AiProviders,
        group: SettingsGroupId::AiProviders,
        title: "settings.ai_providers.providers_group.title",
        items: &["settings.ai_providers.saved.label"],
    },
    SearchGroup {
        page: SettingsPageId::AiProviders,
        group: SettingsGroupId::WebSearch,
        title: "settings.web_search.group.title",
        items: &[
            "settings.web_search.enabled.label",
            "settings.web_search.kind.label",
        ],
    },
    SearchGroup {
        page: SettingsPageId::Sync,
        group: SettingsGroupId::SyncStatus,
        title: "settings.sync.status_group.title",
        items: &[
            "settings.sync.status.last_synced.label",
            "settings.sync.status.state.label",
            "settings.sync.status.action.label",
        ],
    },
    SearchGroup {
        page: SettingsPageId::Sync,
        group: SettingsGroupId::AutoSync,
        title: "settings.sync.auto_sync_group.title",
        items: &[
            "settings.sync.auto_sync.label",
            "settings.sync.auto_sync.status.label",
        ],
    },
    SearchGroup {
        page: SettingsPageId::Sync,
        group: SettingsGroupId::SyncEncryption,
        title: "settings.sync.encryption_group.title",
        items: &["settings.sync.encryption.passphrase.label"],
    },
    SearchGroup {
        page: SettingsPageId::Sync,
        group: SettingsGroupId::SyncProvider,
        title: "settings.sync.provider_group.title",
        items: &["settings.sync.provider.backend.label"],
    },
    SearchGroup {
        page: SettingsPageId::Sync,
        group: SettingsGroupId::LocalOnly,
        title: "settings.sync.local_only_group_title",
        items: &[],
    },
    SearchGroup {
        page: SettingsPageId::Vault,
        group: SettingsGroupId::Vault,
        title: "settings.sync.vault_group.title",
        items: &[
            "settings.sync.vault.status.label",
            "settings.sync.vault.passphrase.label",
            "settings.sync.vault.auto_lock_duration.label",
            "settings.sync.vault.disable.label",
        ],
    },
    SearchGroup {
        page: SettingsPageId::About,
        group: SettingsGroupId::Overview,
        title: "settings.about.overview.title",
        items: &[],
    },
    SearchGroup {
        page: SettingsPageId::About,
        group: SettingsGroupId::Licenses,
        title: "settings.about.licenses.title",
        items: &[],
    },
    SearchGroup {
        page: SettingsPageId::About,
        group: SettingsGroupId::Onboarding,
        title: "settings.about.onboarding.title",
        items: &["settings.about.onboarding.label"],
    },
    SearchGroup {
        page: SettingsPageId::About,
        group: SettingsGroupId::DataDirectory,
        title: "settings.about.data_directory.title",
        items: &["settings.about.data_directory.label"],
    },
    SearchGroup {
        page: SettingsPageId::About,
        group: SettingsGroupId::Reset,
        title: "settings.about.reset_local.title",
        items: &["settings.about.reset_local.label"],
    },
];

fn page_title(page: SettingsPageId) -> &'static str {
    match page {
        SettingsPageId::Appearance => "settings.pages.appearance.title",
        SettingsPageId::Connections => "settings.pages.connections.title",
        SettingsPageId::OpenSshIntegration => "settings.pages.openssh_integration.title",
        SettingsPageId::KeyBindings => "settings.pages.key_bindings.title",
        SettingsPageId::AiProviders => "settings.pages.ai_providers.title",
        SettingsPageId::Sync => "settings.pages.sync.title",
        SettingsPageId::Vault => "settings.pages.vault.title",
        SettingsPageId::About => "settings.pages.about.title",
    }
}

pub(super) fn groups_for_page(page: SettingsPageId) -> Vec<SettingsGroupId> {
    GROUPS
        .iter()
        .filter(|group| group.page == page)
        .map(|group| group.group)
        .collect()
}

pub(super) fn search_destination(id: &str) -> Option<(SettingsPageId, SettingsGroupId)> {
    GROUPS
        .iter()
        .find(|group| group.title == id || group.items.contains(&id))
        .map(|group| (group.page, group.group))
}

pub(in crate::ui) fn search_entries() -> Vec<SearchEntry> {
    GROUPS
        .iter()
        .flat_map(|group| {
            std::iter::once(group.title)
                .chain(group.items.iter().copied())
                .map(move |key| {
                    SearchEntry::new(
                        format!("setting:{key}"),
                        SearchKind::Setting,
                        i18n::string(key),
                        format!(
                            "{} · {}",
                            i18n::string(page_title(group.page)),
                            i18n::string(group.title)
                        ),
                        format!(
                            "{key} {} {} {}",
                            group.title,
                            page_title(group.page),
                            match key {
                                "settings.appearance.interface_font_size.label" =>
                                    "界面字号 UI font size",
                                "settings.appearance.terminal_font_size.label" =>
                                    "终端字号 terminal font size",
                                _ => "",
                            }
                        )
                        .replace(['.', '_'], " "),
                        SearchTarget::Setting(key),
                    )
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_catalog_has_unique_resolvable_ids_and_localized_labels() {
        let entries = search_entries();
        let mut ids = std::collections::HashSet::new();
        assert!(entries.len() > 70);
        for entry in entries {
            assert!(
                ids.insert(entry.id().to_owned()),
                "duplicate {}",
                entry.id()
            );
            let SearchTarget::Setting(key) = entry.target() else {
                panic!("setting target")
            };
            assert!(search_destination(key).is_some());
            assert!(
                !entry.title().starts_with("settings."),
                "missing label {key}"
            );
        }
    }
}
