use crate::ui::assets::AppIcon;
use crate::ui::components::forms::{
    localized_secret_placeholder, new_input_state, set_code_editor_input_placeholder,
    set_editor_value, set_input_masked, set_input_placeholder, set_input_value,
};
use crate::ui::components::notifications::{
    ValidationFailure, error_notification, validation_notification,
};
use crate::ui::components::selection::{ManagedKeySelectItem, ProfileSelectItem, SelectOption};
use crate::ui::contracts::commands::{AppCommand, DeferredAppCommand, TabOpenRequest};
use crate::ui::contracts::navigation::{PageEditorSidebarKind, ProfileViewMode};
use crate::ui::contracts::security::LocalVaultStatus;
use gpui_kit::prelude::*;
#[cfg(test)]
use gpui_kit::{Bounds, div};
#[cfg(test)]
use miaominal_core::profile::{PortForwardKind, PortForwardRule};
mod import_labels;
use crate::ui::contracts::overlay::{OverlayRevisionState, OverlaySnapshot};
use crate::ui::contracts::{CatalogQueryPort, ProfileCommitPort};
use crate::ui::i18n;
use crate::ui::utils::local_shell;
use gpui_kit::component::{
    input::{EditorState, InputEvent, InputState, TabSize},
    select::{SearchableVec, SelectEvent, SelectItem, SelectState},
};
use gpui_kit::{App, Context, Entity, EventEmitter, ScrollHandle, Subscription, Window};
use miaominal_core::{
    keychain::ManagedKeyRecord,
    profile::{
        AuthMethod, DEFAULT_SESSION_CHARSET, ImportIssue, ImportSourceKind, ImportedBatch,
        ProfileKind, SessionEnvironmentVariable, SessionProfile, ShellType,
    },
    proxy::ProxyProfile,
};
use miaominal_secrets::{SecretKind, SecretStore};
use miaominal_services::{ImportedProfilesResult, ProfileService};
use miaominal_storage::config_store::store::SessionStore;
use std::rc::Rc;
use std::{
    cell::{Cell, Ref, RefCell, RefMut},
    collections::HashSet,
    path::PathBuf,
};
#[path = "hosts/dialogs.rs"]
mod dialogs;
#[path = "hosts/profile_import.rs"]
mod profile_import;
#[path = "hosts/view.rs"]
mod view;
#[derive(Clone)]
pub(in crate::ui) struct HostEditorEnvironmentVariableRow {
    pub(in crate::ui) name_input: Entity<InputState>,
    pub(in crate::ui) value_input: Entity<InputState>,
}

#[derive(Clone)]
pub(in crate::ui) struct HostEditorForms {
    pub(in crate::ui) name_input: Entity<InputState>,
    pub(in crate::ui) group_input: Entity<InputState>,
    pub(in crate::ui) group_select: Entity<SelectState<SearchableVec<String>>>,
    pub(in crate::ui) managed_key_select: Entity<SelectState<SearchableVec<ManagedKeySelectItem>>>,
    pub(in crate::ui) proxy_jump_select: Entity<SelectState<SearchableVec<ProfileSelectItem>>>,
    pub(in crate::ui) entry_proxy_select:
        Entity<SelectState<SearchableVec<SelectOption<Option<String>>>>>,
    pub(in crate::ui) charset_select: Entity<SelectState<SearchableVec<String>>>,
    pub(in crate::ui) creating_new_group: bool,
    pub(in crate::ui) tags_input: Entity<InputState>,
    pub(in crate::ui) host_input: Entity<InputState>,
    pub(in crate::ui) port_input: Entity<InputState>,
    pub(in crate::ui) username_input: Entity<InputState>,
    pub(in crate::ui) password_input: Entity<InputState>,
    pub(in crate::ui) private_key_input: Entity<InputState>,
    pub(in crate::ui) agent_identity_input: Entity<InputState>,
    pub(in crate::ui) certificate_input: Entity<InputState>,
    pub(in crate::ui) passphrase_input: Entity<InputState>,
    pub(in crate::ui) startup_command_input: Entity<EditorState>,
    pub(in crate::ui) proxy_jump_profile_ids: Vec<String>,
    pub(in crate::ui) selected_proxy_jump_hop: Option<usize>,
    pub(in crate::ui) environment_variable_rows: Vec<HostEditorEnvironmentVariableRow>,
    pub(in crate::ui) shell_type: ShellType,
    pub(in crate::ui) editing_auth_method: AuthMethod,
    pub(in crate::ui) agent_forwarding_enabled: bool,
    pub(in crate::ui) profile_kind: ProfileKind,
    pub(in crate::ui) local_shell_input: Entity<InputState>,
    pub(in crate::ui) local_shell_args_input: Entity<InputState>,
    pub(in crate::ui) local_working_directory_input: Entity<InputState>,
}

#[derive(Clone)]
pub(in crate::ui) struct HostsForms {
    pub(in crate::ui) filter_input: Entity<InputState>,
}

#[derive(Debug, Clone)]
pub(in crate::ui) struct PendingProfileDeleteState {
    pub(in crate::ui) profile_id: String,
    pub(in crate::ui) profile_name: String,
    pub(in crate::ui) reload_inputs_after_delete: bool,
}

#[derive(Debug, Clone)]
pub(in crate::ui) struct PendingProfileImportResultState {
    pub(in crate::ui) source: ImportSourceKind,
    pub(in crate::ui) imported_count: usize,
    pub(in crate::ui) issues: Vec<ImportIssue>,
}
fn is_valid_environment_variable_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };

    if first != '_' && !first.is_ascii_alphabetic() {
        return false;
    }

    characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

#[derive(Clone)]
pub(in crate::ui) struct HostEditorState {
    pub(in crate::ui) host_editor_open: bool,
    pub(in crate::ui) host_editor_is_new: bool,
}
#[derive(Clone)]
pub(in crate::ui) struct HostsCatalogViewState {
    pub(in crate::ui) hosts_view_mode: ProfileViewMode,
    pub(in crate::ui) hosts_group_filter: Option<String>,
}
#[derive(Clone)]
pub(in crate::ui) struct HostsPanelForms {
    pub(in crate::ui) hosts: HostsForms,
}
#[derive(Default)]
struct HostsPendingDialogs {
    profile_delete: Option<PendingProfileDeleteState>,
    profile_import_result: Option<PendingProfileImportResultState>,
}
pub(in crate::ui) struct HostsControllerArgs {
    pub(in crate::ui) catalog_query: CatalogQueryPort,
    pub(in crate::ui) profile_commit: ProfileCommitPort,
    pub(in crate::ui) session_store: Option<SessionStore>,
    pub(in crate::ui) secrets: SecretStore,
    pub(in crate::ui) profiles: Vec<SessionProfile>,
    pub(in crate::ui) proxies: Vec<ProxyProfile>,
    pub(in crate::ui) selected_profile: Option<usize>,
    pub(in crate::ui) managed_keys: Vec<ManagedKeyRecord>,
    pub(in crate::ui) local_vault_status: LocalVaultStatus,
}
struct HostsServices {
    session_store: Option<SessionStore>,
    secrets: SecretStore,
    local_vault_status: LocalVaultStatus,
}
pub(in crate::ui) struct HostsController {
    services: HostsServices,
    catalog_query: CatalogQueryPort,
    profile_commit: ProfileCommitPort,
    window_handle: gpui_kit::AnyWindowHandle,
    managed_key_options: RefCell<Vec<ManagedKeySelectItem>>,
    pending_save: RefCell<Option<HostSaveSubmission>>,
    editor_revision: Cell<u64>,
    editor_profile_id: RefCell<Option<String>>,
    profiles: RefCell<Vec<SessionProfile>>,
    proxies: RefCell<Vec<ProxyProfile>>,
    selected_profile: Cell<Option<usize>>,
    host_password_visible: bool,
    host_editor_forms: RefCell<HostEditorForms>,
    host_editor_scroll_handle: ScrollHandle,
    panel_forms: HostsPanelForms,
    editor_state: RefCell<HostEditorState>,
    catalog_view: RefCell<HostsCatalogViewState>,
    pending_dialogs: RefCell<HostsPendingDialogs>,
    overlay_revisions: RefCell<OverlayRevisionState>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone)]
struct HostSaveSubmission {
    editor_revision: u64,
    profile: SessionProfile,
    is_new: bool,
}

fn rebase_host_submission(
    service: &ProfileService,
    profiles: &[SessionProfile],
    submission: HostSaveSubmission,
) -> anyhow::Result<(SessionProfile, Option<usize>)> {
    let mut profile = submission.profile;
    if submission.is_new {
        profile.id = service.next_profile_id(profiles);
        return Ok((profile, None));
    }
    let index = resolve_profile_index(profiles, &profile.id).ok_or_else(|| {
        anyhow::anyhow!(i18n::string_args(
            "profile.messages.already_removed",
            &[("name", &profile.name)]
        ))
    })?;
    let current = &profiles[index];
    profile.port_forwarding_rules = current.port_forwarding_rules.clone();
    profile.remote_path_favorites = current.remote_path_favorites.clone();
    profile.is_favorite = current.is_favorite;
    profile.last_connected_at = current.last_connected_at;
    if profile.password.is_empty() {
        profile.has_stored_password = current.has_stored_password;
    }
    if profile.passphrase.is_empty() {
        profile.has_stored_passphrase = current.has_stored_passphrase;
    }
    Ok((profile, Some(index)))
}

impl HostsController {
    fn build_forms(
        profiles: &[SessionProfile],
        proxies: &[ProxyProfile],
        selected_profile: Option<usize>,
        managed_keys: &[ManagedKeyRecord],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> HostEditorForms {
        let selection = initial_profile_selection(profiles, selected_profile);
        let selected_profile_data = selection.selected_profile_data;
        let selected_group = selection.selected_group;
        let selected_existing_group = selection.selected_existing_group;
        let name_input = new_input_state(
            i18n::string("placeholders.host_editor.profile_name"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.name.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let group_input = new_input_state(
            i18n::string("placeholders.host_editor.new_group_name"),
            if selected_existing_group.is_none() {
                selected_group.clone()
            } else {
                String::new()
            },
            false,
            window,
            cx,
        );
        let tags_input = new_input_state(
            i18n::string("placeholders.host_editor.tags"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.tags.join(", "))
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let host_input = new_input_state(
            i18n::string("placeholders.host_editor.host"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.host.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let port_input = new_input_state(
            "22",
            selected_profile_data
                .as_ref()
                .map(|profile| profile.port.to_string())
                .unwrap_or_else(|| "22".to_string()),
            false,
            window,
            cx,
        );
        let username_input = new_input_state(
            i18n::string("placeholders.host_editor.username"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.username.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let password_input = new_input_state(
            localized_secret_placeholder(
                selected_profile_data
                    .as_ref()
                    .is_some_and(|profile| profile.has_stored_password),
                "placeholders.host_editor.password",
            ),
            "",
            true,
            window,
            cx,
        );
        let private_key_input = new_input_state(
            i18n::string("placeholders.host_editor.private_key_path"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.private_key_path.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let agent_identity_input = new_input_state(
            i18n::string("placeholders.host_editor.agent_identity"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.agent_identity.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let certificate_input = new_input_state(
            i18n::string("placeholders.host_editor.certificate_path"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.certificate_path.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let passphrase_input = new_input_state(
            localized_secret_placeholder(
                selected_profile_data
                    .as_ref()
                    .is_some_and(|profile| profile.has_stored_passphrase),
                "placeholders.host_editor.key_passphrase",
            ),
            "",
            true,
            window,
            cx,
        );
        let startup_command_value = selected_profile_data
            .as_ref()
            .map(|profile| profile.startup_command.clone())
            .unwrap_or_default();
        let startup_command_input = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("bash")
                .indent_guides(false)
                .folding(false)
                .searchable(false)
                .tab_size(TabSize {
                    tab_size: 2,
                    ..Default::default()
                })
                .placeholder("")
                .default_value(startup_command_value)
        });
        set_code_editor_input_placeholder(
            &startup_command_input,
            i18n::string("placeholders.host_editor.startup_command"),
            false,
            window,
            cx,
        );
        let local_shell_input = new_input_state(
            local_shell::local_shell_placeholder(),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.local_shell.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let local_shell_args_input = new_input_state(
            i18n::string("placeholders.host_editor.local_shell_args"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.local_shell_args.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let local_working_directory_input = new_input_state(
            i18n::string("placeholders.host_editor.local_working_directory"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.local_working_directory.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let proxy_jump_profile_ids = selected_profile_data
            .as_ref()
            .map(|profile| profile.proxy_jump_profile_ids.clone())
            .unwrap_or_default();
        let selected_entry_proxy_id = selected_profile_data
            .as_ref()
            .and_then(|profile| profile.entry_proxy_id.clone());
        let environment_variable_rows = selected_profile_data
            .as_ref()
            .map(|profile| {
                Self::host_editor_environment_variable_rows(
                    &profile.environment_variables,
                    window,
                    cx,
                )
            })
            .unwrap_or_else(|| Self::host_editor_environment_variable_rows(&[], window, cx));
        let group_select = cx.new(|cx| {
            let mut state = SelectState::new(
                SearchableVec::new(selection.available_groups),
                None,
                window,
                cx,
            );
            if let Some(existing_group) = selected_existing_group.as_ref() {
                state.set_selected_value(existing_group, window, cx);
            }
            state
        });
        let managed_key_select = cx.new(|cx| {
            let mut state = SelectState::new(
                ManagedKeySelectItem::options(managed_keys),
                None,
                window,
                cx,
            )
            .searchable(true);
            if let Some(managed_key_id) = selected_profile_data
                .as_ref()
                .map(|profile| profile.managed_key_id.trim().to_string())
                .filter(|managed_key_id| !managed_key_id.is_empty())
            {
                state.set_selected_value(&managed_key_id, window, cx);
            }
            state
        });
        let selected_charset = selected_profile_data
            .as_ref()
            .map(|profile| profile.charset.trim().to_string())
            .filter(|charset| !charset.is_empty())
            .unwrap_or_else(|| DEFAULT_SESSION_CHARSET.to_string());
        let charset_select = cx.new(|cx| {
            let mut state = SelectState::new(
                SearchableVec::new(Self::available_session_charsets()),
                None,
                window,
                cx,
            )
            .searchable(true);
            state.set_selected_value(&selected_charset, window, cx);
            state
        });
        let proxy_jump_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Vec::<ProfileSelectItem>::new()),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        let entry_proxy_options =
            Self::entry_proxy_select_options(proxies, selected_entry_proxy_id.as_deref());
        let selected_entry_proxy = entry_proxy_options
            .iter()
            .position(|option| option.value() == &selected_entry_proxy_id)
            .map(|index| gpui_kit::component::IndexPath::default().row(index));
        let entry_proxy_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(entry_proxy_options),
                selected_entry_proxy,
                window,
                cx,
            )
            .searchable(true)
        });
        HostEditorForms {
            name_input,
            group_input,
            group_select,
            managed_key_select,
            proxy_jump_select,
            entry_proxy_select,
            charset_select,
            creating_new_group: !selected_group.is_empty() && selected_existing_group.is_none(),
            tags_input,
            host_input,
            port_input,
            username_input,
            password_input,
            private_key_input,
            agent_identity_input,
            certificate_input,
            passphrase_input,
            startup_command_input,
            proxy_jump_profile_ids,
            selected_proxy_jump_hop: None,
            environment_variable_rows,
            shell_type: selected_profile_data
                .as_ref()
                .map(|profile| profile.shell_type)
                .unwrap_or_default(),
            editing_auth_method: selection.editing_auth_method,
            agent_forwarding_enabled: selected_profile_data
                .as_ref()
                .is_some_and(|profile| profile.agent_forwarding),
            profile_kind: selected_profile_data
                .as_ref()
                .map(|profile| profile.kind)
                .unwrap_or_default(),
            local_shell_input,
            local_shell_args_input,
            local_working_directory_input,
        }
    }
    pub(in crate::ui) fn refresh_localized_placeholders(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host_editor = self.host_editor_forms();
        let selected_profile = if self.editor_state().host_editor_is_new {
            None
        } else {
            self.selected_profile()
                .and_then(|index| self.profiles().get(index).cloned())
        };
        let has_saved_password = selected_profile
            .as_ref()
            .is_some_and(|profile| profile.has_stored_password);
        let has_saved_passphrase = selected_profile
            .as_ref()
            .is_some_and(|profile| profile.has_stored_passphrase);

        for (input, key) in [
            (
                &host_editor.name_input,
                "placeholders.host_editor.profile_name",
            ),
            (
                &host_editor.group_input,
                "placeholders.host_editor.new_group_name",
            ),
            (&host_editor.tags_input, "placeholders.host_editor.tags"),
            (&host_editor.host_input, "placeholders.host_editor.host"),
            (
                &host_editor.username_input,
                "placeholders.host_editor.username",
            ),
            (
                &host_editor.private_key_input,
                "placeholders.host_editor.private_key_path",
            ),
            (
                &host_editor.agent_identity_input,
                "placeholders.host_editor.agent_identity",
            ),
            (
                &host_editor.certificate_input,
                "placeholders.host_editor.certificate_path",
            ),
        ] {
            set_input_placeholder(input, i18n::string(key), window, cx);
        }
        set_input_placeholder(&host_editor.port_input, "22", window, cx);
        set_input_placeholder(
            &host_editor.password_input,
            localized_secret_placeholder(has_saved_password, "placeholders.host_editor.password"),
            window,
            cx,
        );
        set_input_placeholder(
            &host_editor.passphrase_input,
            localized_secret_placeholder(
                has_saved_passphrase,
                "placeholders.host_editor.key_passphrase",
            ),
            window,
            cx,
        );
        set_code_editor_input_placeholder(
            &host_editor.startup_command_input,
            i18n::string("placeholders.host_editor.startup_command"),
            false,
            window,
            cx,
        );
        for row in &host_editor.environment_variable_rows {
            set_input_placeholder(
                &row.name_input,
                i18n::string("placeholders.host_editor.environment_variable_name"),
                window,
                cx,
            );
            set_input_placeholder(
                &row.value_input,
                i18n::string("placeholders.host_editor.environment_variable_value"),
                window,
                cx,
            );
        }

        set_input_placeholder(
            &self.panel_forms.hosts.filter_input,
            i18n::string("placeholders.hosts.filter"),
            window,
            cx,
        );
    }
    pub(in crate::ui) fn new(
        args: HostsControllerArgs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let host_editor = Self::build_forms(
            &args.profiles,
            &args.proxies,
            args.selected_profile,
            &args.managed_keys,
            window,
            cx,
        );
        let panel_forms = HostsPanelForms {
            hosts: HostsForms {
                filter_input: new_input_state(
                    i18n::string("placeholders.hosts.filter"),
                    "",
                    false,
                    window,
                    cx,
                ),
            },
        };
        macro_rules! subscribe_to_change {
            ($input:expr) => {
                cx.subscribe(&$input, |_, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                })
            };
        }
        let group_input = host_editor.group_input.clone();
        let group_select = host_editor.group_select.clone();
        let managed_key_select = host_editor.managed_key_select.clone();
        let proxy_jump_select = host_editor.proxy_jump_select.clone();
        let entry_proxy_select = host_editor.entry_proxy_select.clone();
        let hosts_filter_input = panel_forms.hosts.filter_input.clone();
        let subscriptions = vec![
            subscribe_to_change!(group_input),
            cx.subscribe(
                &group_select,
                |controller, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                    let SelectEvent::Confirm(selected_group) = event;
                    if selected_group.is_some() {
                        controller.host_editor_forms_mut().creating_new_group = false;
                    }
                    cx.notify();
                },
            ),
            cx.subscribe(
                &managed_key_select,
                |controller, _, event: &SelectEvent<SearchableVec<ManagedKeySelectItem>>, cx| {
                    let SelectEvent::Confirm(selected_managed_key_id) = event;
                    if selected_managed_key_id.is_some()
                        && controller.host_editor_forms().editing_auth_method
                            != AuthMethod::ManagedKey
                    {
                        controller.host_editor_forms_mut().editing_auth_method =
                            AuthMethod::ManagedKey;
                        cx.notify();
                    }
                },
            ),
            cx.subscribe(
                &proxy_jump_select,
                |controller, _, event: &SelectEvent<SearchableVec<ProfileSelectItem>>, cx| {
                    let SelectEvent::Confirm(selected_profile_id) = event;
                    if let Some(selected_profile_id) = selected_profile_id.as_deref() {
                        controller.add_proxy_jump_profile(selected_profile_id, cx);
                    }
                },
            ),
            cx.subscribe(
                &entry_proxy_select,
                |_controller,
                 _,
                 event: &SelectEvent<SearchableVec<SelectOption<Option<String>>>>,
                 cx| {
                    let SelectEvent::Confirm(_) = event;
                    cx.notify();
                },
            ),
            subscribe_to_change!(hosts_filter_input),
        ];
        Self {
            catalog_query: args.catalog_query,
            profile_commit: args.profile_commit,
            window_handle: window.window_handle(),
            managed_key_options: RefCell::new(ManagedKeySelectItem::sorted_items(
                &args.managed_keys,
            )),
            pending_save: RefCell::new(None),
            editor_revision: Cell::new(0),
            editor_profile_id: RefCell::new(None),
            services: HostsServices {
                session_store: args.session_store,
                secrets: args.secrets,
                local_vault_status: args.local_vault_status,
            },
            profiles: RefCell::new(args.profiles),
            proxies: RefCell::new(args.proxies),
            selected_profile: Cell::new(args.selected_profile),
            host_password_visible: false,
            host_editor_forms: RefCell::new(host_editor),
            host_editor_scroll_handle: ScrollHandle::new(),
            panel_forms,
            editor_state: RefCell::new(HostEditorState {
                host_editor_open: false,
                host_editor_is_new: false,
            }),
            catalog_view: RefCell::new(HostsCatalogViewState {
                hosts_view_mode: ProfileViewMode::Grid,
                hosts_group_filter: None,
            }),
            overlay_revisions: RefCell::default(),
            pending_dialogs: RefCell::new(HostsPendingDialogs::default()),
            _subscriptions: subscriptions,
        }
    }
    pub(in crate::ui) fn current_host_editor_profile_id(&self) -> Option<String> {
        let index = self.selected_profile.get()?;
        self.profiles
            .borrow()
            .get(index)
            .map(|profile| profile.id.clone())
    }

    pub(in crate::ui) fn available_proxy_jump_candidates(
        &self,
        target_profile_id: &str,
    ) -> Vec<SessionProfile> {
        let forms = self.host_editor_forms();
        let chained_ids: HashSet<&str> = forms
            .proxy_jump_profile_ids
            .iter()
            .map(String::as_str)
            .collect();
        let mut candidates = self
            .profiles
            .borrow()
            .iter()
            .filter(|profile| {
                !profile.is_local()
                    && profile.id != target_profile_id
                    && !chained_ids.contains(profile.id.as_str())
            })
            .cloned()
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        candidates
    }

    pub(in crate::ui) fn collect_available_groups(profiles: &[SessionProfile]) -> Vec<String> {
        let mut groups = profiles
            .iter()
            .filter_map(|profile| {
                let group = profile.group.trim();
                (!group.is_empty()).then(|| group.to_string())
            })
            .collect::<Vec<_>>();
        groups.sort_by_key(|group| group.to_ascii_lowercase());
        groups.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        groups
    }

    pub(in crate::ui) fn available_groups(&self) -> Vec<String> {
        Self::collect_available_groups(&self.profiles.borrow())
    }

    pub(in crate::ui) fn proxy_jump_chain_profiles(&self) -> Vec<SessionProfile> {
        let profile_ids = self.host_editor_forms().proxy_jump_profile_ids;
        let profiles = self.profiles.borrow();
        profile_ids
            .iter()
            .filter_map(|profile_id| {
                profiles
                    .iter()
                    .find(|profile| profile.id == *profile_id)
                    .cloned()
            })
            .collect()
    }

    pub(in crate::ui) fn set_auth_method(&self, auth_method: AuthMethod, cx: &mut Context<Self>) {
        self.host_editor_forms_mut().editing_auth_method = match auth_method {
            AuthMethod::KeyFile => AuthMethod::ManagedKey,
            _ => auth_method,
        };
        cx.notify();
    }

    pub(in crate::ui) fn set_agent_forwarding_enabled(
        &self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        self.host_editor_forms_mut().agent_forwarding_enabled = enabled;
        cx.notify();
    }

    pub(in crate::ui) fn set_host_editor_profile_kind(
        &self,
        kind: ProfileKind,
        cx: &mut Context<Self>,
    ) {
        if self.host_editor_forms().profile_kind == kind {
            return;
        }
        self.host_editor_forms_mut().profile_kind = kind;
        cx.notify();
    }

    pub(in crate::ui) fn set_local_shell_path(
        &self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.host_editor_forms().local_shell_input.clone();
        set_input_value(&input, path.display().to_string(), window, cx);
        cx.notify();
    }

    pub(in crate::ui) fn set_local_working_directory_path(
        &self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self
            .host_editor_forms()
            .local_working_directory_input
            .clone();
        set_input_value(&input, path.display().to_string(), window, cx);
        cx.notify();
    }

    fn host_editor_profile_kind(kind: ProfileKind) -> ProfileKind {
        match kind {
            ProfileKind::Local => ProfileKind::Local,
            ProfileKind::Ssh | ProfileKind::Telnet | ProfileKind::Rdp => ProfileKind::Ssh,
        }
    }

    pub(in crate::ui) fn set_shell_type(&self, shell_type: ShellType, cx: &mut Context<Self>) {
        self.host_editor_forms_mut().shell_type = shell_type;
        cx.notify();
    }

    pub(in crate::ui) fn begin_new_group(&self, window: &mut Window, cx: &mut Context<Self>) {
        let forms = self.host_editor_forms();
        self.host_editor_forms_mut().creating_new_group = true;
        forms.group_select.update(cx, |select, cx| {
            select.set_selected_index(None, window, cx);
        });
        set_input_value(&forms.group_input, "", window, cx);
        cx.notify();
    }

    fn new_host_editor_environment_variable_row<T: 'static>(
        name: impl Into<String>,
        value: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> HostEditorEnvironmentVariableRow {
        HostEditorEnvironmentVariableRow {
            name_input: new_input_state(
                i18n::string("placeholders.host_editor.environment_variable_name"),
                name.into(),
                false,
                window,
                cx,
            ),
            value_input: new_input_state(
                i18n::string("placeholders.host_editor.environment_variable_value"),
                value.into(),
                false,
                window,
                cx,
            ),
        }
    }

    pub(in crate::ui) fn host_editor_environment_variable_rows<T: 'static>(
        variables: &[SessionEnvironmentVariable],
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Vec<HostEditorEnvironmentVariableRow> {
        if variables.is_empty() {
            return vec![Self::new_host_editor_environment_variable_row(
                "", "", window, cx,
            )];
        }

        variables
            .iter()
            .map(|variable| {
                Self::new_host_editor_environment_variable_row(
                    variable.name.clone(),
                    variable.value.clone(),
                    window,
                    cx,
                )
            })
            .collect()
    }

    pub(in crate::ui) fn add_environment_variable_row(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row = Self::new_host_editor_environment_variable_row("", "", window, cx);
        self.host_editor_forms_mut()
            .environment_variable_rows
            .push(row);
        cx.notify();
    }

    pub(in crate::ui) fn remove_environment_variable_row(
        &self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let needs_replacement = {
            let mut forms = self.host_editor_forms_mut();
            if index < forms.environment_variable_rows.len() {
                forms.environment_variable_rows.remove(index);
            }
            forms.environment_variable_rows.is_empty()
        };
        if needs_replacement {
            let replacement = Self::new_host_editor_environment_variable_row("", "", window, cx);
            self.host_editor_forms_mut()
                .environment_variable_rows
                .push(replacement);
        }
        cx.notify();
    }

    pub(in crate::ui) fn select_proxy_jump_step(&self, step_index: usize, cx: &mut Context<Self>) {
        let mut forms = self.host_editor_forms_mut();
        forms.selected_proxy_jump_hop = if step_index < forms.proxy_jump_profile_ids.len() {
            Some(step_index)
        } else {
            None
        };
        drop(forms);
        cx.notify();
    }

    pub(in crate::ui) fn move_selected_proxy_jump_hop_up(&self, cx: &mut Context<Self>) {
        let mut forms = self.host_editor_forms_mut();
        let Some(selected_index) = forms.selected_proxy_jump_hop else {
            return;
        };
        if selected_index == 0 {
            return;
        }
        forms
            .proxy_jump_profile_ids
            .swap(selected_index - 1, selected_index);
        forms.selected_proxy_jump_hop = Some(selected_index - 1);
        drop(forms);
        cx.notify();
    }

    pub(in crate::ui) fn move_selected_proxy_jump_hop_down(&self, cx: &mut Context<Self>) {
        let mut forms = self.host_editor_forms_mut();
        let Some(selected_index) = forms.selected_proxy_jump_hop else {
            return;
        };
        if selected_index + 1 >= forms.proxy_jump_profile_ids.len() {
            return;
        }
        forms
            .proxy_jump_profile_ids
            .swap(selected_index, selected_index + 1);
        forms.selected_proxy_jump_hop = Some(selected_index + 1);
        drop(forms);
        cx.notify();
    }

    pub(in crate::ui) fn remove_selected_proxy_jump_hop(&self, cx: &mut Context<Self>) {
        let mut forms = self.host_editor_forms_mut();
        let Some(selected_index) = forms.selected_proxy_jump_hop else {
            return;
        };
        if selected_index >= forms.proxy_jump_profile_ids.len() {
            forms.selected_proxy_jump_hop = None;
            return;
        }
        forms.proxy_jump_profile_ids.remove(selected_index);
        forms.selected_proxy_jump_hop = if forms.proxy_jump_profile_ids.is_empty() {
            None
        } else {
            Some(selected_index.min(forms.proxy_jump_profile_ids.len() - 1))
        };
        drop(forms);
        self.sync_proxy_jump_candidate_select_in_active_window(cx);
        cx.notify();
    }

    pub(in crate::ui) fn close_host_editor(&self, cx: &mut Context<Self>) {
        if !self.editor_state().host_editor_open {
            return;
        }
        self.set_host_editor_state(false, false);
        cx.emit(AppCommand::SidebarEditorStateChanged(None));
        cx.notify();
    }

    pub(in crate::ui) fn request_selected_profile_delete(&self, cx: &mut Context<Self>) {
        let Some(index) = self.selected_profile() else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "profile.messages.select_profile_to_delete",
            )));
            return;
        };
        let Some(profile) = self.profiles.borrow().get(index).cloned() else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "profile.messages.select_profile_to_delete",
            )));
            return;
        };
        self.set_pending_profile_delete(Some(PendingProfileDeleteState {
            profile_id: profile.id,
            profile_name: profile.name,
            reload_inputs_after_delete: true,
        }));
        cx.notify();
    }

    pub(in crate::ui) fn request_new_profile_editor(&self, cx: &mut Context<Self>) {
        cx.emit(AppCommand::OpenTab(TabOpenRequest::NewProfileEditor));
    }

    pub(in crate::ui) fn request_profile_editor_at_index(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(profile_id) = self
            .profiles
            .borrow()
            .get(index)
            .map(|profile| profile.id.clone())
        else {
            return;
        };
        self.request_profile_editor(profile_id, false, cx);
    }

    pub(in crate::ui) fn request_profile_editor(
        &self,
        profile_id: String,
        open_hosts_tab: bool,
        cx: &mut Context<Self>,
    ) {
        if !self
            .profiles
            .borrow()
            .iter()
            .any(|profile| profile.id == profile_id)
        {
            cx.emit(AppCommand::Feedback(i18n::string(
                "trusted.messages.profile_not_found",
            )));
            return;
        }
        cx.emit(AppCommand::OpenTab(TabOpenRequest::ProfileEditor {
            profile_id,
            open_hosts_tab,
        }));
    }

    pub(in crate::ui) fn request_profile_connection_test(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.read_profile_from_inputs(
            self.current_profile_id(),
            ProfileInputPurpose::ConnectionTest,
            cx,
        ) {
            Ok(profile) => {
                cx.emit(AppCommand::OpenTab(TabOpenRequest::ProfileConnectionTest {
                    profile: Box::new(profile),
                }));
            }
            Err(error) => {
                if let Some(validation) = error.downcast_ref::<ValidationFailure>() {
                    let message = validation.message.clone();
                    cx.emit(AppCommand::Feedback(message.clone()));
                    crate::ui::components::notifications::push_app_notification(
                        window,
                        validation_notification(validation.kind, message),
                        cx,
                    );
                } else {
                    let message = error.to_string();
                    cx.emit(AppCommand::Feedback(i18n::string_args(
                        "profile.messages.test_connection_failed",
                        &[("message", &message)],
                    )));
                    crate::ui::components::notifications::push_app_notification(
                        window,
                        error_notification(
                            i18n::string("profile.messages.test_connection_failed_title"),
                            message,
                        ),
                        cx,
                    );
                }
                cx.notify();
            }
        }
    }

    pub(in crate::ui) fn request_profile_save(
        &mut self,
        sync_requires_unlock: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.read_profile_save_draft(cx) {
            Ok(profile) => {
                if sync_requires_unlock
                    || (self.services.local_vault_status == LocalVaultStatus::Locked
                        && (!profile.password.is_empty() || !profile.passphrase.is_empty()))
                {
                    self.pending_save.replace(Some(HostSaveSubmission {
                        editor_revision: self.editor_revision.get(),
                        profile,
                        is_new: self.editor_state().host_editor_is_new,
                    }));
                    cx.emit(AppCommand::vault_unlock(
                        DeferredAppCommand::SaveHostProfile,
                    ));
                } else {
                    let options = self.managed_key_options.borrow().clone();
                    self.commit_profile_save_request(profile, options, window, cx);
                }
            }
            Err(error) => self.report_profile_save_error(error, window, cx),
        }
    }

    pub(in crate::ui) fn commit_profile_save_request(
        &mut self,
        profile: SessionProfile,
        managed_key_options: Vec<ManagedKeySelectItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let submission = HostSaveSubmission {
            editor_revision: self.editor_revision.get(),
            profile,
            is_new: self.editor_state().host_editor_is_new,
        };
        self.commit_profile_submission(submission, managed_key_options, window, cx);
    }

    fn commit_profile_submission(
        &mut self,
        submission: HostSaveSubmission,
        managed_key_options: Vec<ManagedKeySelectItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editor_state().host_editor_open
            || submission.editor_revision != self.editor_revision.get()
        {
            return;
        }
        match self.save_prepared_profile(submission, cx) {
            Ok(profile) => {
                self.load_selected_profile_into_inputs(managed_key_options, window, cx);
                self.set_host_editor_state(false, false);
                cx.emit(AppCommand::SidebarEditorStateChanged(None));
                let message = if self.session_store_available() {
                    i18n::string_args("profile.messages.saved", &[("name", &profile.name)])
                } else {
                    i18n::string_args(
                        "profile.messages.saved_memory_only",
                        &[("name", &profile.name)],
                    )
                };
                cx.emit(AppCommand::Feedback(message));
                cx.notify();
            }
            Err(error) => self.report_profile_save_error(error, window, cx),
        }
    }

    pub(in crate::ui) fn request_sftp_at_profile_index(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(profile_id) = self
            .profiles
            .borrow()
            .get(index)
            .map(|profile| profile.id.clone())
        else {
            return;
        };
        cx.emit(AppCommand::OpenTab(TabOpenRequest::Sftp {
            profile_id,
            owner: None,
        }));
    }

    pub(in crate::ui) fn request_session_at_profile_index(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(profile_id) = self
            .profiles
            .borrow()
            .get(index)
            .map(|profile| profile.id.clone())
        else {
            return;
        };
        cx.emit(AppCommand::OpenTab(TabOpenRequest::Session { profile_id }));
    }

    pub(in crate::ui) fn host_password_reveal_icon(&self) -> AppIcon {
        if self.host_password_visible() {
            AppIcon::EyeOff
        } else {
            AppIcon::Eye
        }
    }

    pub(in crate::ui) fn host_editor_auth_method(auth_method: AuthMethod) -> AuthMethod {
        match auth_method {
            AuthMethod::KeyFile => AuthMethod::ManagedKey,
            _ => auth_method,
        }
    }

    fn saved_secret_placeholder(has_saved: bool, fallback_key: &'static str) -> String {
        if has_saved {
            i18n::string("placeholders.saved.keep_existing")
        } else {
            i18n::string(fallback_key)
        }
    }

    fn sync_group_controls(&self, group: &str, window: &mut Window, cx: &mut Context<Self>) {
        let group = group.trim();
        let available_groups = self.available_groups();
        let selected_existing_group = available_groups
            .iter()
            .find(|candidate| candidate.eq_ignore_ascii_case(group))
            .cloned();
        let forms = self.host_editor_forms();

        forms.group_select.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(available_groups), window, cx);
            if let Some(existing_group) = selected_existing_group.as_ref() {
                select.set_selected_value(existing_group, window, cx);
            } else {
                select.set_selected_index(None, window, cx);
            }
        });

        let creating_new_group = !group.is_empty() && selected_existing_group.is_none();
        self.host_editor_forms_mut().creating_new_group = creating_new_group;
        set_input_value(
            &forms.group_input,
            if creating_new_group {
                group.to_string()
            } else {
                String::new()
            },
            window,
            cx,
        );
    }

    pub(in crate::ui) fn sync_managed_key_select(
        &self,
        options: Vec<ManagedKeySelectItem>,
        selected_key_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.managed_key_options.replace(options.clone());
        let forms = self.host_editor_forms();
        let selected_key_id = selected_key_id
            .map(str::to_string)
            .or_else(|| forms.managed_key_select.read(cx).selected_value().cloned());
        let has_selected_key = selected_key_id.as_ref().is_some_and(|selected_key_id| {
            options.iter().any(|item| item.value() == selected_key_id)
        });

        forms.managed_key_select.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(options), window, cx);
            if has_selected_key {
                if let Some(selected_key_id) = selected_key_id.as_ref() {
                    select.set_selected_value(selected_key_id, window, cx);
                }
            } else {
                select.set_selected_index(None, window, cx);
            }
        });
    }

    fn sync_entry_proxy_select(
        &self,
        selected_proxy_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let options =
            Self::entry_proxy_select_options(&self.proxies.borrow(), selected_proxy_id.as_deref());
        let select = self.host_editor_forms().entry_proxy_select;
        select.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(options), window, cx);
            select.set_selected_value(&selected_proxy_id, window, cx);
        });
    }

    pub(in crate::ui) fn refresh_entry_proxy_select(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected_proxy_id = self
            .host_editor_forms()
            .entry_proxy_select
            .read(cx)
            .selected_value()
            .cloned()
            .flatten();
        self.sync_entry_proxy_select(selected_proxy_id, window, cx);
    }

    pub(in crate::ui) fn sync_managed_key_select_in_active_window(
        &self,
        options: Vec<ManagedKeySelectItem>,
        selected_key_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.managed_key_options.replace(options.clone());
        let forms = self.host_editor_forms();
        let selected_key_id =
            selected_key_id.or_else(|| forms.managed_key_select.read(cx).selected_value().cloned());
        let has_selected_key = selected_key_id.as_ref().is_some_and(|selected_key_id| {
            options.iter().any(|item| item.value() == selected_key_id)
        });
        let managed_key_select = forms.managed_key_select;

        if let Some(window_handle) = Some(self.window_handle)
            && let Err(error) = window_handle.update(cx, move |_, window, cx| {
                managed_key_select.update(cx, |select, cx| {
                    select.set_items(SearchableVec::new(options), window, cx);
                    if has_selected_key {
                        if let Some(selected_key_id) = selected_key_id.as_ref() {
                            select.set_selected_value(selected_key_id, window, cx);
                        }
                    } else {
                        select.set_selected_index(None, window, cx);
                    }
                });
            })
        {
            log::debug!("failed to refresh managed key options: {error:?}");
        }
    }

    fn sync_proxy_jump_candidate_select(
        &self,
        selected_profile_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target_profile_id = self.current_host_editor_profile_id().unwrap_or_default();
        let options = SearchableVec::new(
            self.available_proxy_jump_candidates(&target_profile_id)
                .iter()
                .map(ProfileSelectItem::new)
                .collect::<Vec<_>>(),
        );
        let selected_profile_id = selected_profile_id.map(str::to_string);
        let proxy_jump_select = self.host_editor_forms().proxy_jump_select;

        proxy_jump_select.update(cx, |select, cx| {
            select.set_items(options, window, cx);
            if let Some(selected_profile_id) = selected_profile_id.as_ref() {
                select.set_selected_value(selected_profile_id, window, cx);
            } else {
                select.set_selected_index(None, window, cx);
            }
        });
    }

    pub(in crate::ui) fn set_host_password_visibility(
        &mut self,
        visible: bool,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.host_password_visible = visible;
        let password_input = self.host_editor_forms().password_input;
        set_input_masked(&password_input, !visible, focus, window, cx);
    }

    pub(in crate::ui) fn load_selected_profile_into_inputs(
        &mut self,
        managed_key_options: Vec<ManagedKeySelectItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile = self
            .selected_profile()
            .and_then(|index| self.profiles.borrow().get(index).cloned());
        if let Some(profile) = profile {
            self.populate_profile_inputs(&profile, managed_key_options, window, cx);
        } else {
            self.clear_profile_inputs(managed_key_options, window, cx);
        }
    }

    fn populate_profile_inputs(
        &mut self,
        profile: &SessionProfile,
        managed_key_options: Vec<ManagedKeySelectItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_host_password_visibility(false, false, window, cx);
        let forms = self.host_editor_forms();
        set_input_value(&forms.name_input, profile.name.clone(), window, cx);
        self.sync_group_controls(&profile.group, window, cx);
        set_input_value(&forms.tags_input, profile.tags.join(", "), window, cx);
        set_input_value(&forms.host_input, profile.host.clone(), window, cx);
        set_input_value(&forms.port_input, profile.port.to_string(), window, cx);
        set_input_value(&forms.username_input, profile.username.clone(), window, cx);
        set_input_value(&forms.password_input, "", window, cx);
        set_input_placeholder(
            &forms.password_input,
            Self::saved_secret_placeholder(
                profile.has_stored_password,
                "placeholders.host_editor.password",
            ),
            window,
            cx,
        );
        set_input_value(
            &forms.private_key_input,
            profile.private_key_path.clone(),
            window,
            cx,
        );
        self.sync_managed_key_select(
            managed_key_options,
            Some(&profile.managed_key_id),
            window,
            cx,
        );
        set_input_value(
            &forms.agent_identity_input,
            profile.agent_identity.clone(),
            window,
            cx,
        );
        set_input_value(
            &forms.certificate_input,
            profile.certificate_path.clone(),
            window,
            cx,
        );
        set_input_value(&forms.passphrase_input, "", window, cx);
        set_input_placeholder(
            &forms.passphrase_input,
            Self::saved_secret_placeholder(
                profile.has_stored_passphrase,
                "placeholders.host_editor.key_passphrase",
            ),
            window,
            cx,
        );
        set_editor_value(
            &forms.startup_command_input,
            profile.startup_command.clone(),
            window,
            cx,
        );
        set_input_value(
            &forms.local_shell_input,
            profile.local_shell.clone(),
            window,
            cx,
        );
        set_input_value(
            &forms.local_shell_args_input,
            profile.local_shell_args.clone(),
            window,
            cx,
        );
        set_input_value(
            &forms.local_working_directory_input,
            profile.local_working_directory.clone(),
            window,
            cx,
        );
        let selected_charset = if profile.charset.trim().is_empty() {
            DEFAULT_SESSION_CHARSET.to_string()
        } else {
            profile.charset.clone()
        };
        forms.charset_select.update(cx, |select, cx| {
            select.set_selected_value(&selected_charset, window, cx);
        });
        self.sync_entry_proxy_select(profile.entry_proxy_id.clone(), window, cx);
        {
            let mut forms = self.host_editor_forms_mut();
            forms.proxy_jump_profile_ids = profile.proxy_jump_profile_ids.clone();
            forms.selected_proxy_jump_hop = None;
        }
        self.sync_proxy_jump_candidate_select(None, window, cx);
        let environment_variable_rows =
            Self::host_editor_environment_variable_rows(&profile.environment_variables, window, cx);
        let mut forms = self.host_editor_forms_mut();
        forms.environment_variable_rows = environment_variable_rows;
        forms.shell_type = profile.shell_type;
        forms.editing_auth_method = Self::host_editor_auth_method(profile.effective_auth_method());
        forms.agent_forwarding_enabled = profile.agent_forwarding;
        forms.profile_kind = profile.kind;
    }

    fn clear_profile_inputs(
        &mut self,
        managed_key_options: Vec<ManagedKeySelectItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_host_password_visibility(false, false, window, cx);
        let forms = self.host_editor_forms();
        set_input_value(&forms.name_input, "", window, cx);
        self.sync_group_controls("", window, cx);
        set_input_value(&forms.tags_input, "", window, cx);
        set_input_value(&forms.host_input, "", window, cx);
        set_input_value(&forms.port_input, "22", window, cx);
        set_input_value(&forms.username_input, "", window, cx);
        set_input_value(&forms.password_input, "", window, cx);
        set_input_placeholder(
            &forms.password_input,
            i18n::string("placeholders.host_editor.password"),
            window,
            cx,
        );
        set_input_value(&forms.private_key_input, "", window, cx);
        self.sync_managed_key_select(managed_key_options, Some(""), window, cx);
        set_input_value(&forms.agent_identity_input, "", window, cx);
        set_input_value(&forms.certificate_input, "", window, cx);
        set_input_value(&forms.passphrase_input, "", window, cx);
        set_input_placeholder(
            &forms.passphrase_input,
            i18n::string("placeholders.host_editor.key_passphrase"),
            window,
            cx,
        );
        set_editor_value(&forms.startup_command_input, "", window, cx);
        set_input_value(&forms.local_shell_input, "", window, cx);
        set_input_placeholder(
            &forms.local_shell_input,
            local_shell::local_shell_placeholder(),
            window,
            cx,
        );
        set_input_value(&forms.local_shell_args_input, "", window, cx);
        set_input_value(&forms.local_working_directory_input, "", window, cx);
        forms.charset_select.update(cx, |select, cx| {
            select.set_selected_value(&DEFAULT_SESSION_CHARSET.to_string(), window, cx);
        });
        self.sync_entry_proxy_select(None, window, cx);
        {
            let mut forms = self.host_editor_forms_mut();
            forms.proxy_jump_profile_ids.clear();
            forms.selected_proxy_jump_hop = None;
        }
        self.sync_proxy_jump_candidate_select(None, window, cx);
        let environment_variable_rows =
            Self::host_editor_environment_variable_rows(&[], window, cx);
        let mut forms = self.host_editor_forms_mut();
        forms.environment_variable_rows = environment_variable_rows;
        forms.shell_type = ShellType::Posix;
        forms.editing_auth_method = AuthMethod::Password;
        forms.agent_forwarding_enabled = false;
        forms.profile_kind = ProfileKind::Ssh;
    }

    pub(in crate::ui) fn add_profile(
        &mut self,
        managed_key_options: Vec<ManagedKeySelectItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile =
            SessionProfile::blank(self.next_profile_id(), self.profiles.borrow().len() + 1);
        self.set_selected_profile(None);
        self.set_host_editor_state(true, true);
        self.populate_profile_inputs(&profile, managed_key_options, window, cx);
        self.reset_host_editor_scroll();
        cx.emit(AppCommand::SidebarEditorStateChanged(Some(
            PageEditorSidebarKind::Hosts,
        )));
        cx.emit(AppCommand::Feedback(i18n::string(
            "profile.messages.new_profile_created",
        )));
        cx.notify();
    }

    pub(in crate::ui) fn open_profile_editor(
        &mut self,
        index: usize,
        managed_key_options: Vec<ManagedKeySelectItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(profile) = self.profiles.borrow().get(index).cloned() else {
            return;
        };

        self.set_selected_profile(Some(index));
        self.populate_profile_inputs(&profile, managed_key_options, window, cx);
        self.set_host_editor_state(true, false);
        self.reset_host_editor_scroll();
        cx.emit(AppCommand::SidebarEditorStateChanged(Some(
            PageEditorSidebarKind::Hosts,
        )));
        cx.emit(AppCommand::Feedback(i18n::string_args(
            "navigation.messages.editing_profile",
            &[("name", &profile.name)],
        )));
        cx.notify();
    }

    fn delete_profile_by_id(
        &mut self,
        profile_id: &str,
        profile_name: &str,
        reload_inputs_after_delete: bool,
        managed_key_options: Vec<ManagedKeySelectItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let id = profile_id.to_string();
        let service = self.profile_service();
        let deleted = Rc::new(RefCell::new(None));
        let outcome = deleted.clone();
        let result = self.profile_commit.commit(cx, move |mut profiles| {
            if let Some(index) = resolve_profile_index(&profiles, &id) {
                let mut selection = None;
                let removed =
                    service.delete_and_persist_profile(&mut profiles, &mut selection, index)?;
                outcome.replace(removed.map(|removed| removed.removed));
            }
            Ok(profiles)
        });
        let profiles = match result {
            Ok(profiles) => profiles,
            Err(error) => {
                cx.emit(AppCommand::Feedback(i18n::string_args(
                    "profile.messages.deleted_local_save_failed",
                    &[("name", profile_name), ("error", &error.to_string())],
                )));
                cx.notify();
                return false;
            }
        };
        self.replace_profiles_from_application(profiles);
        let Some(removed) = deleted.take() else {
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "profile.messages.already_removed",
                &[("name", profile_name)],
            )));
            cx.notify();
            return true;
        };
        if self.editor_profile_id.borrow().as_deref() == Some(profile_id) {
            self.set_host_editor_state(false, false);
            cx.emit(AppCommand::SidebarEditorStateChanged(None));
        }
        if reload_inputs_after_delete {
            self.load_selected_profile_into_inputs(managed_key_options, window, cx);
        }
        cx.emit(AppCommand::Feedback(i18n::string_args(
            "profile.messages.deleted",
            &[("name", &removed.name)],
        )));
        cx.notify();
        true
    }

    pub(in crate::ui) fn load_selected_profile_password_input(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let Some(index) = self.selected_profile() else {
            return Ok(());
        };
        let Some(profile) = self.profiles.borrow().get(index).cloned() else {
            return Ok(());
        };

        if !profile.has_stored_password {
            return Ok(());
        }

        let password = self
            .services
            .secrets
            .get(&profile.id, SecretKind::Password)?
            .unwrap_or_default();
        let password_input = self.host_editor_forms().password_input;
        set_input_value(&password_input, password, window, cx);
        Ok(())
    }

    pub(in crate::ui) fn prepare_host_password_for_lock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.selected_profile() else {
            return;
        };
        let Some(profile) = self.profiles.borrow().get(index).cloned() else {
            return;
        };
        if !profile.has_stored_password {
            return;
        }

        let password_input = self.host_editor_forms().password_input;
        let current_password = password_input.read(cx).value().to_string();
        if current_password.is_empty() {
            self.set_host_password_visibility(false, false, window, cx);
            return;
        }

        match self.services.secrets.get(&profile.id, SecretKind::Password) {
            Ok(Some(stored_password)) if stored_password == current_password => {
                set_input_value(&password_input, "", window, cx);
            }
            Ok(_) => {}
            Err(error) => {
                log::warn!(
                    "failed to compare stored host password before locking local vault: {error:?}"
                );
            }
        }

        self.set_host_password_visibility(false, false, window, cx);
    }

    pub(in crate::ui) fn reveal_host_password_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.load_selected_profile_password_input(window, cx) {
            let message = error.to_string();
            cx.emit(AppCommand::Feedback(message.clone()));
            crate::ui::components::notifications::push_app_notification(
                window,
                error_notification(
                    i18n::string("settings.sync.vault.notifications.failed_title"),
                    message,
                ),
                cx,
            );
            cx.notify();
            return;
        }

        self.set_host_password_visibility(true, true, window, cx);
        cx.notify();
    }

    pub(in crate::ui) fn toggle_host_password_visibility(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.host_password_visible() {
            self.set_host_password_visibility(false, true, window, cx);
            cx.notify();
            return;
        }

        let forms = self.host_editor_forms();
        let has_text = !forms.password_input.read(cx).value().is_empty();
        let has_stored_password = self
            .selected_profile()
            .and_then(|index| self.profiles.borrow().get(index).cloned())
            .is_some_and(|profile| profile.has_stored_password);

        if has_text || !has_stored_password {
            self.set_host_password_visibility(true, true, window, cx);
            cx.notify();
            return;
        }

        if self.services.local_vault_status == LocalVaultStatus::Locked {
            cx.emit(AppCommand::vault_unlock(
                DeferredAppCommand::RevealHostPassword(self.editor_revision.get()),
            ));
            return;
        }

        self.reveal_host_password_input(window, cx);
    }

    pub(in crate::ui) fn continue_reveal_password_after_unlock(
        &mut self,
        revision: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor_revision.get() == revision && self.editor_state().host_editor_open {
            self.reveal_host_password_input(window, cx);
        }
    }

    fn import_profiles(
        &self,
        batch: ImportedBatch,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<ImportedProfilesResult> {
        let service = self.profile_service();
        let imported = Rc::new(RefCell::new(None));
        let outcome = imported.clone();
        let profiles = self.profile_commit.commit(cx, move |mut profiles| {
            outcome.replace(Some(service.import_profiles(&mut profiles, batch)?));
            Ok(profiles)
        })?;
        self.replace_profiles_from_application(profiles);
        Ok(imported
            .take()
            .expect("successful import returns its result"))
    }

    fn group_value(&self, cx: &App) -> String {
        let forms = self.host_editor_forms();
        if forms.creating_new_group {
            forms.group_input.read(cx).value().trim().to_string()
        } else {
            forms
                .group_select
                .read(cx)
                .selected_value()
                .cloned()
                .unwrap_or_default()
                .trim()
                .to_string()
        }
    }

    fn session_charset_value(&self, cx: &App) -> String {
        self.host_editor_forms()
            .charset_select
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_else(|| DEFAULT_SESSION_CHARSET.to_string())
            .trim()
            .to_string()
    }

    fn read_environment_variables(
        &self,
        cx: &App,
    ) -> anyhow::Result<Vec<SessionEnvironmentVariable>> {
        let mut variables = Vec::new();
        let forms = self.host_editor_forms();

        for (index, row) in forms.environment_variable_rows.iter().enumerate() {
            let raw_name = row.name_input.read(cx).value().to_string();
            let raw_value = row.value_input.read(cx).value().to_string();
            let name = raw_name.trim().to_string();

            if name.is_empty() {
                if raw_value.trim().is_empty() {
                    continue;
                }

                let index = (index + 1).to_string();
                anyhow::bail!(i18n::string_args(
                    "errors.host_editor.environment_variables.missing_name",
                    &[("index", &index)],
                ));
            }

            if !is_valid_environment_variable_name(&name) {
                anyhow::bail!(i18n::string_args(
                    "errors.host_editor.environment_variables.invalid_name",
                    &[("name", &name)],
                ));
            }

            variables.push(SessionEnvironmentVariable {
                name,
                value: raw_value,
            });
        }

        Ok(variables)
    }

    fn read_proxy_jump_profile_ids(&self, target_profile_id: &str) -> anyhow::Result<Vec<String>> {
        let mut seen = HashSet::new();
        let mut resolved = Vec::new();
        let forms = self.host_editor_forms();
        let profiles = self.profiles.borrow();

        for profile_id in &forms.proxy_jump_profile_ids {
            let profile_id = profile_id.trim();
            if profile_id.is_empty() {
                continue;
            }
            if profile_id == target_profile_id {
                anyhow::bail!("host chaining cannot reference the host being edited");
            }
            if !seen.insert(profile_id.to_string()) {
                anyhow::bail!("host chaining cannot include the same saved host more than once");
            }

            let profile = profiles
                .iter()
                .find(|profile| profile.id == profile_id)
                .ok_or_else(|| anyhow::anyhow!("a selected jump host is no longer available"))?;
            resolved.push(profile.id.clone());
        }

        Ok(resolved)
    }

    fn current_profile_id(&self) -> String {
        self.editor_profile_id
            .borrow()
            .clone()
            .unwrap_or_else(|| self.next_profile_id())
    }

    pub(in crate::ui) fn next_profile_id(&self) -> String {
        self.profile_service()
            .next_profile_id(&self.catalog_query.profiles())
    }

    fn read_profile_from_inputs(
        &self,
        profile_id: String,
        purpose: ProfileInputPurpose,
        cx: &App,
    ) -> anyhow::Result<SessionProfile> {
        let forms = self.host_editor_forms();
        let name = forms.name_input.read(cx).value().to_string();
        let group = self.group_value(cx);
        let tags_text = forms.tags_input.read(cx).value().to_string();
        let host = forms.host_input.read(cx).value().to_string();
        let port_text = forms.port_input.read(cx).value().to_string();
        let username = forms.username_input.read(cx).value().to_string();
        let password = forms.password_input.read(cx).value().to_string();
        let private_key_path = forms.private_key_input.read(cx).value().to_string();
        let managed_key_id = forms
            .managed_key_select
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_default();
        let agent_identity = forms.agent_identity_input.read(cx).value().to_string();
        let certificate_path = forms.certificate_input.read(cx).value().to_string();
        let passphrase = forms.passphrase_input.read(cx).value().to_string();
        let startup_command = forms.startup_command_input.read(cx).value().to_string();
        let charset = self.session_charset_value(cx);

        let name = name.trim().to_string();
        let group = group.trim().to_string();
        let tags = ProfileService::parse_tags(&tags_text);
        let host = host.trim().to_string();
        let username = username.trim().to_string();
        let private_key_path = private_key_path.trim().to_string();
        let managed_key_id = managed_key_id.trim().to_string();
        let agent_identity = agent_identity.trim().to_string();
        let certificate_path = certificate_path.trim().to_string();
        let startup_command = startup_command.trim().to_string();
        let charset = if charset.is_empty() {
            DEFAULT_SESSION_CHARSET.to_string()
        } else {
            charset
        };
        let environment_variables = self.read_environment_variables(cx)?;
        let is_local_kind = forms.profile_kind == ProfileKind::Local;
        let proxy_jump_profile_ids = if is_local_kind {
            Vec::new()
        } else {
            self.read_proxy_jump_profile_ids(&profile_id)?
        };
        let entry_proxy_id = if is_local_kind {
            None
        } else {
            forms
                .entry_proxy_select
                .read(cx)
                .selected_value()
                .cloned()
                .flatten()
        };
        if let Some(entry_proxy_id) = entry_proxy_id.as_deref()
            && !self
                .proxies
                .borrow()
                .iter()
                .any(|proxy| proxy.id == entry_proxy_id)
        {
            return Err(ValidationFailure::invalid(i18n::string_args(
                "errors.profile.validation.entry_proxy_missing",
                &[("id", entry_proxy_id)],
            ))
            .into());
        }

        if is_local_kind {
            let local_shell = forms.local_shell_input.read(cx).value().trim().to_string();
            let local_shell_args = forms
                .local_shell_args_input
                .read(cx)
                .value()
                .trim()
                .to_string();
            let local_working_directory = forms
                .local_working_directory_input
                .read(cx)
                .value()
                .trim()
                .to_string();
            if local_shell.is_empty() {
                return Err(ValidationFailure::required(i18n::string(
                    "errors.profile.validation.local_shell_required",
                ))
                .into());
            }
            if !local_shell::local_shell_is_available(&local_shell) {
                return Err(ValidationFailure::invalid(i18n::string_args(
                    "errors.profile.validation.local_shell_not_found",
                    &[("path", &local_shell)],
                ))
                .into());
            }
            if !local_shell::local_working_directory_is_available(&local_working_directory) {
                return Err(ValidationFailure::invalid(i18n::string_args(
                    "errors.profile.validation.local_working_directory_not_found",
                    &[("path", &local_working_directory)],
                ))
                .into());
            }
            let profiles = self.profiles.borrow();
            let existing = profiles.iter().find(|profile| profile.id == profile_id);
            let name = if name.is_empty() {
                local_shell::local_terminal_label(&local_shell)
            } else {
                name
            };
            return Ok(SessionProfile {
                id: profile_id,
                name,
                group: String::new(),
                tags,
                kind: ProfileKind::Local,
                host: String::new(),
                port: 22,
                username: String::new(),
                password: String::new(),
                auth_method: None,
                private_key_path: String::new(),
                managed_key_id: String::new(),
                agent_identity: String::new(),
                agent_identity_label: String::new(),
                certificate_path: String::new(),
                passphrase: String::new(),
                agent_forwarding: false,
                startup_command,
                charset,
                environment_variables,
                shell_type: ShellType::Posix,
                proxy_jump_profile_ids: Vec::new(),
                entry_proxy_id: None,
                has_stored_password: false,
                has_stored_passphrase: false,
                port_forwarding_rules: existing
                    .map(|profile| profile.port_forwarding_rules.clone())
                    .unwrap_or_default(),
                is_favorite: existing.is_some_and(|profile| profile.is_favorite),
                remote_path_favorites: Vec::new(),
                local_shell,
                local_shell_args,
                local_working_directory,
                last_connected_at: existing.and_then(|profile| profile.last_connected_at),
            });
        }

        if purpose.requires_name() && name.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.profile.validation.profile_name_required",
            ))
            .into());
        }
        if host.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.profile.validation.host_required",
            ))
            .into());
        }
        if username.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.profile.validation.username_required",
            ))
            .into());
        }

        let name = if name.is_empty() {
            format!("{}@{}", username, host)
        } else {
            name
        };
        let port: u16 = port_text.trim().parse().map_err(|_| {
            anyhow::anyhow!(ValidationFailure::invalid(i18n::string_args(
                "errors.profile.validation.invalid_port",
                &[("port", &port_text)],
            )))
        })?;

        let profiles = self.profiles.borrow();
        let existing = profiles.iter().find(|profile| profile.id == profile_id);
        let prior_password = existing.is_some_and(|profile| profile.has_stored_password);
        let prior_passphrase = existing.is_some_and(|profile| profile.has_stored_passphrase);
        let has_password = !password.trim().is_empty();
        let has_passphrase = !passphrase.trim().is_empty();
        let auth_method = Self::host_editor_auth_method(forms.editing_auth_method);

        match auth_method {
            AuthMethod::Password => {
                if !has_password && !prior_password {
                    return Err(ValidationFailure::required(i18n::string(
                        "errors.profile.validation.password_requires_password",
                    ))
                    .into());
                }
            }
            AuthMethod::KeyFile | AuthMethod::ManagedKey => {
                if managed_key_id.is_empty() {
                    return Err(ValidationFailure::required(i18n::string(
                        "errors.profile.validation.managed_key_requires_id",
                    ))
                    .into());
                }
            }
            AuthMethod::Agent => {
                if agent_identity.is_empty() {
                    return Err(ValidationFailure::required(i18n::string(
                        "errors.profile.validation.ssh_agent_requires_identity",
                    ))
                    .into());
                }
            }
            AuthMethod::KeyboardInteractive => {}
        }

        let private_key_path = if matches!(auth_method, AuthMethod::KeyFile) {
            private_key_path
        } else {
            String::new()
        };
        let passphrase = if matches!(auth_method, AuthMethod::KeyFile) {
            passphrase
        } else {
            String::new()
        };
        let has_stored_passphrase = if matches!(auth_method, AuthMethod::KeyFile) {
            has_passphrase || prior_passphrase
        } else {
            false
        };

        if !certificate_path.is_empty() && matches!(auth_method, AuthMethod::Password) {
            return Err(ValidationFailure::invalid(i18n::string(
                "errors.profile.validation.certificate_requires_key_based_identity",
            ))
            .into());
        }

        let form_local_shell = forms.local_shell_input.read(cx).value().trim().to_string();
        let form_local_shell_args = forms
            .local_shell_args_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let form_local_working_directory = forms
            .local_working_directory_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let local_shell = if form_local_shell.is_empty() {
            existing
                .map(|profile| profile.local_shell.clone())
                .unwrap_or_default()
        } else {
            form_local_shell
        };
        let local_shell_args = if form_local_shell_args.is_empty() {
            existing
                .map(|profile| profile.local_shell_args.clone())
                .unwrap_or_default()
        } else {
            form_local_shell_args
        };
        let local_working_directory = if form_local_working_directory.is_empty() {
            existing
                .map(|profile| profile.local_working_directory.clone())
                .unwrap_or_default()
        } else {
            form_local_working_directory
        };

        Ok(SessionProfile {
            id: profile_id,
            name,
            group,
            tags,
            kind: Self::host_editor_profile_kind(forms.profile_kind),
            host,
            port,
            username,
            password,
            auth_method: Some(auth_method),
            private_key_path,
            passphrase,
            managed_key_id,
            agent_identity: agent_identity.clone(),
            agent_identity_label: existing
                .map(|profile| profile.agent_identity_label.clone())
                .filter(|label| !label.trim().is_empty())
                .unwrap_or(agent_identity),
            certificate_path,
            agent_forwarding: forms.agent_forwarding_enabled,
            startup_command,
            charset,
            environment_variables,
            shell_type: forms.shell_type,
            proxy_jump_profile_ids,
            entry_proxy_id,
            has_stored_password: has_password || prior_password,
            has_stored_passphrase,
            port_forwarding_rules: existing
                .map(|profile| profile.port_forwarding_rules.clone())
                .unwrap_or_default(),
            is_favorite: existing.map(|profile| profile.is_favorite).unwrap_or(false),
            remote_path_favorites: existing
                .map(|profile| profile.remote_path_favorites.clone())
                .unwrap_or_default(),
            local_shell,
            local_shell_args,
            local_working_directory,
            last_connected_at: existing.and_then(|profile| profile.last_connected_at),
        })
    }

    pub(in crate::ui) fn read_profile_save_draft(
        &self,
        cx: &App,
    ) -> anyhow::Result<SessionProfile> {
        let id = self.current_profile_id();
        if !self.editor_state().host_editor_is_new
            && self.editor_profile_id.borrow().is_some()
            && self.profile_index(&id).is_none()
        {
            return Err(ValidationFailure::invalid(i18n::string_args(
                "profile.messages.already_removed",
                &[(
                    "name",
                    &self.host_editor_forms().name_input.read(cx).value(),
                )],
            ))
            .into());
        }
        self.read_profile_from_inputs(id, ProfileInputPurpose::Save, cx)
    }

    fn save_prepared_profile(
        &self,
        submission: HostSaveSubmission,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<SessionProfile> {
        let service = self.profile_service();
        let saved = Rc::new(RefCell::new(None));
        let outcome = saved.clone();
        let profiles = self.profile_commit.commit(cx, move |mut profiles| {
            let (profile, mut selected) = rebase_host_submission(&service, &profiles, submission)?;
            service.save_profile(&mut profiles, &mut selected, profile.clone())?;
            outcome.replace(Some(profile));
            Ok(profiles)
        })?;
        let profile = saved
            .take()
            .expect("successful profile save returns its profile");
        self.replace_profiles_from_application(profiles);
        self.selected_profile.set(self.profile_index(&profile.id));
        Ok(profile)
    }

    fn report_profile_save_error(
        &self,
        error: anyhow::Error,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(validation) = error.downcast_ref::<ValidationFailure>() {
            let message = validation.message.clone();
            cx.emit(AppCommand::Feedback(message.clone()));
            crate::ui::components::notifications::push_app_notification(
                window,
                validation_notification(validation.kind, message),
                cx,
            );
        } else {
            let message = error.to_string();
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "profile.messages.save_failed",
                &[("message", &message)],
            )));
        }
        cx.notify();
    }

    pub(in crate::ui) fn continue_save_profile_after_unlock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(submission) = self.pending_save.take() else {
            return;
        };
        let options = self.managed_key_options.borrow().clone();
        self.commit_profile_submission(submission, options, window, cx);
    }

    fn sync_proxy_jump_candidate_select_in_active_window(&self, cx: &mut Context<Self>) {
        let target_profile_id = self.current_host_editor_profile_id().unwrap_or_default();
        let options = SearchableVec::new(
            self.available_proxy_jump_candidates(&target_profile_id)
                .iter()
                .map(ProfileSelectItem::new)
                .collect::<Vec<_>>(),
        );
        let select = self.host_editor_forms().proxy_jump_select;
        if let Some(window_handle) = Some(self.window_handle)
            && let Err(error) = window_handle.update(cx, move |_, window, cx| {
                select.update(cx, |select, cx| {
                    select.set_items(options, window, cx);
                    select.set_selected_index(None, window, cx);
                });
            })
        {
            log::debug!("failed to refresh proxy jump candidates: {error:?}");
        }
    }

    fn add_proxy_jump_profile(&self, profile_id: &str, cx: &mut Context<Self>) {
        let forms = self.host_editor_forms();
        if forms
            .proxy_jump_profile_ids
            .iter()
            .any(|existing| existing == profile_id)
            || self
                .current_host_editor_profile_id()
                .is_some_and(|current_id| current_id == profile_id)
        {
            return;
        }

        {
            let mut forms = self.host_editor_forms_mut();
            forms.proxy_jump_profile_ids.push(profile_id.to_string());
            forms.selected_proxy_jump_hop = Some(forms.proxy_jump_profile_ids.len() - 1);
        }
        self.sync_proxy_jump_candidate_select_in_active_window(cx);
        cx.notify();
    }

    fn available_session_charsets() -> Vec<String> {
        [
            "UTF-8",
            "GB18030",
            "GBK",
            "GB2312",
            "Big5",
            "Shift_JIS",
            "EUC-JP",
            "EUC-KR",
            "ISO-8859-1",
            "ISO-8859-15",
            "Windows-1252",
            "KOI8-R",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    fn entry_proxy_select_options(
        proxies: &[ProxyProfile],
        selected_proxy_id: Option<&str>,
    ) -> Vec<SelectOption<Option<String>>> {
        let mut options = vec![SelectOption::new(
            None,
            i18n::string("hosts.editor.entry_proxy.direct"),
        )];
        options.extend(
            proxies
                .iter()
                .map(|proxy| SelectOption::new(Some(proxy.id.clone()), proxy.connection_label())),
        );
        if let Some(selected_proxy_id) = selected_proxy_id
            && !proxies.iter().any(|proxy| proxy.id == selected_proxy_id)
        {
            options.push(SelectOption::new(
                Some(selected_proxy_id.to_string()),
                i18n::string_args(
                    "hosts.editor.entry_proxy.missing_option",
                    &[("id", selected_proxy_id)],
                ),
            ));
        }
        options
    }

    pub(in crate::ui) fn request_profile_delete_at_index(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(profile) = self.profiles.borrow().get(index).cloned() else {
            return;
        };

        self.set_pending_profile_delete(Some(PendingProfileDeleteState {
            profile_id: profile.id,
            profile_name: profile.name,
            reload_inputs_after_delete: false,
        }));
        cx.notify();
    }

    pub(in crate::ui) fn duplicate_profile_at_index(&self, index: usize, cx: &mut Context<Self>) {
        let Some(original) = self.profiles.borrow().get(index).cloned() else {
            return;
        };
        let service = self.profile_service();
        let id = original.id.clone();
        let duplicated = Rc::new(RefCell::new(None));
        let outcome = duplicated.clone();
        let result = self.profile_commit.commit(cx, move |mut profiles| {
            let index = resolve_profile_index(&profiles, &id)
                .ok_or_else(|| anyhow::anyhow!("profile no longer exists"))?;
            let name = i18n::string_args(
                "profile.messages.duplicate_name",
                &[("name", &profiles[index].name)],
            );
            let profile = service
                .duplicate_profile(&mut profiles, index, name)
                .expect("resolved duplicate profile");
            service.persist_sessions(&profiles)?;
            outcome.replace(Some(profile));
            Ok(profiles)
        });
        let message = match result {
            Ok(profiles) => {
                self.replace_profiles_from_application(profiles);
                let profile = duplicated
                    .take()
                    .expect("successful duplicate returns profile");
                i18n::string_args("profile.messages.duplicated_as", &[("name", &profile.name)])
            }
            Err(error) => i18n::string_args(
                "profile.messages.duplicated_local_save_failed",
                &[("name", &original.name), ("error", &error.to_string())],
            ),
        };
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    pub(in crate::ui) fn toggle_profile_favorite(&self, index: usize, cx: &mut Context<Self>) {
        let Some(original) = self.profiles.borrow().get(index).cloned() else {
            return;
        };
        let service = self.profile_service();
        let id = original.id.clone();
        let changed = Rc::new(RefCell::new(None));
        let outcome = changed.clone();
        let result = self.profile_commit.commit(cx, move |mut profiles| {
            let index = resolve_profile_index(&profiles, &id)
                .ok_or_else(|| anyhow::anyhow!("profile no longer exists"))?;
            let profile = &mut profiles[index];
            profile.is_favorite = !profile.is_favorite;
            outcome.replace(Some(profile.clone()));
            service.persist_sessions(&profiles)?;
            Ok(profiles)
        });
        let message = match result {
            Ok(profiles) => {
                self.replace_profiles_from_application(profiles);
                let profile = changed
                    .take()
                    .expect("successful favorite command returns profile");
                i18n::string_args(
                    if profile.is_favorite {
                        "profile.messages.added_to_favorites"
                    } else {
                        "profile.messages.removed_from_favorites"
                    },
                    &[("name", &profile.name)],
                )
            }
            Err(error) => i18n::string_args(
                if original.is_favorite {
                    "profile.messages.unstarred_local_save_failed"
                } else {
                    "profile.messages.starred_local_save_failed"
                },
                &[("name", &original.name), ("error", &error.to_string())],
            ),
        };
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    fn host_editor_forms_state(&self) -> &RefCell<HostEditorForms> {
        &self.host_editor_forms
    }

    fn host_editor_forms(&self) -> HostEditorForms {
        self.host_editor_forms_state().borrow().clone()
    }

    fn host_editor_forms_mut(&self) -> RefMut<'_, HostEditorForms> {
        self.host_editor_forms_state().borrow_mut()
    }

    pub(in crate::ui) fn host_editor_scroll_handle(&self) -> ScrollHandle {
        self.host_editor_scroll_handle.clone()
    }

    fn reset_host_editor_scroll(&self) {
        self.host_editor_scroll_handle
            .set_offset(Default::default());
    }

    pub(in crate::ui) fn pending_profile_delete(&self) -> Option<PendingProfileDeleteState> {
        self.pending_dialogs.borrow().profile_delete.clone()
    }

    pub(in crate::ui) fn set_pending_profile_delete(
        &self,
        pending: Option<PendingProfileDeleteState>,
    ) {
        self.overlay_revisions.borrow_mut().begin("profile-delete");
        self.pending_dialogs.borrow_mut().profile_delete = pending;
    }

    pub(in crate::ui) fn take_pending_profile_delete(&self) -> Option<PendingProfileDeleteState> {
        self.pending_dialogs.borrow_mut().profile_delete.take()
    }

    pub(in crate::ui) fn cancel_profile_delete(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.profile_delete_overlay(cx.entity());
        if self.take_pending_profile_delete().is_some()
            && let Some(snapshot) = snapshot
        {
            cx.emit(AppCommand::OverlayDismissed(snapshot));
        }
    }

    pub(in crate::ui) fn pending_profile_import_result(
        &self,
    ) -> Option<PendingProfileImportResultState> {
        self.pending_dialogs.borrow().profile_import_result.clone()
    }

    pub(in crate::ui) fn set_pending_profile_import_result(
        &self,
        pending: Option<PendingProfileImportResultState>,
    ) {
        self.overlay_revisions
            .borrow_mut()
            .begin("profile-import-result");
        self.pending_dialogs.borrow_mut().profile_import_result = pending;
    }

    pub(in crate::ui) fn dismiss_profile_import_result(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.profile_import_result_overlay(cx.entity());
        if self
            .pending_dialogs
            .borrow_mut()
            .profile_import_result
            .take()
            .is_some()
            && let Some(snapshot) = snapshot
        {
            cx.emit(AppCommand::OverlayDismissed(snapshot));
        }
    }

    pub(in crate::ui) fn host_password_visible(&self) -> bool {
        self.host_password_visible
    }

    pub(in crate::ui) fn set_host_editor_state(&self, open: bool, is_new: bool) {
        self.pending_save.take();
        self.editor_revision
            .set(self.editor_revision.get().wrapping_add(1).max(1));
        self.editor_profile_id.replace(if open && !is_new {
            self.selected_profile()
                .and_then(|index| self.profiles.borrow().get(index).map(|p| p.id.clone()))
        } else {
            None
        });
        let mut state = self.editor_state.borrow_mut();
        state.host_editor_open = open;
        state.host_editor_is_new = is_new;
    }

    pub(in crate::ui) fn set_hosts_view_mode(
        &mut self,
        mode: ProfileViewMode,
        cx: &mut Context<Self>,
    ) {
        self.catalog_view.get_mut().hosts_view_mode = mode;
        cx.notify();
    }

    pub(in crate::ui) fn toggle_hosts_group_filter(
        &mut self,
        group: String,
        cx: &mut Context<Self>,
    ) {
        let status =
            if self.catalog_view.get_mut().hosts_group_filter.as_deref() == Some(group.as_str()) {
                self.catalog_view.get_mut().hosts_group_filter = None;
                i18n::string("hosts.messages.cleared_group_filter")
            } else {
                self.catalog_view.get_mut().hosts_group_filter = Some(group.clone());
                i18n::string_args("hosts.messages.filtering_by_group", &[("group", &group)])
            };
        cx.emit(AppCommand::Feedback(status));
        cx.notify();
    }

    pub(in crate::ui) fn remove_managed_key_from_editor(&self, key_id: &str, cx: &App) {
        let forms = self.host_editor_forms();
        if forms
            .managed_key_select
            .read(cx)
            .selected_value()
            .is_some_and(|id| id == key_id)
            && forms.editing_auth_method == AuthMethod::ManagedKey
        {
            self.host_editor_forms_mut().editing_auth_method = AuthMethod::Password;
        }
    }

    pub(in crate::ui) fn profile_index(&self, id: &str) -> Option<usize> {
        resolve_profile_index(&self.profiles.borrow(), id)
    }

    pub(in crate::ui) fn profiles(&self) -> Ref<'_, Vec<SessionProfile>> {
        self.profiles.borrow()
    }

    pub(in crate::ui) fn selected_profile(&self) -> Option<usize> {
        self.selected_profile.get()
    }

    pub(in crate::ui) fn set_selected_profile(&self, selected: Option<usize>) {
        self.selected_profile.set(selected);
    }

    fn profile_service(&self) -> ProfileService {
        ProfileService::new(
            self.services.session_store.clone(),
            self.services.secrets.clone(),
        )
    }

    pub(in crate::ui) fn session_store_available(&self) -> bool {
        self.services.session_store.is_some()
    }

    pub(in crate::ui) fn owner_window(&self) -> gpui_kit::AnyWindowHandle {
        self.window_handle
    }

    pub(in crate::ui) fn panel_forms(&self) -> HostsPanelForms {
        self.panel_forms.clone()
    }
    pub(in crate::ui) fn editor_state(&self) -> HostEditorState {
        self.editor_state.borrow().clone()
    }
    pub(in crate::ui) fn catalog_view(&self) -> HostsCatalogViewState {
        self.catalog_view.borrow().clone()
    }
    pub(in crate::ui) fn replace_profiles_from_application(&self, profiles: Vec<SessionProfile>) {
        let selected_id = self.selected_profile().and_then(|ix| {
            self.profiles
                .borrow()
                .get(ix)
                .map(|profile| profile.id.clone())
        });
        self.selected_profile
            .set(selected_id.and_then(|id| profiles.iter().position(|profile| profile.id == id)));
        self.profiles.replace(profiles);
    }
    pub(in crate::ui) fn replace_proxies_from_application(&self, proxies: Vec<ProxyProfile>) {
        self.proxies.replace(proxies);
    }
    pub(in crate::ui) fn credentials_changed(
        &mut self,
        secrets: SecretStore,
        status: LocalVaultStatus,
        cx: &mut Context<Self>,
    ) {
        self.services.secrets = secrets;
        self.services.local_vault_status = status;
        cx.notify();
    }
}
impl EventEmitter<AppCommand> for HostsController {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProfileInputPurpose {
    Save,
    ConnectionTest,
}

impl ProfileInputPurpose {
    fn requires_name(self) -> bool {
        matches!(self, Self::Save)
    }
}
pub(in crate::ui) struct HostConnectionTest {
    pub(in crate::ui) in_progress: bool,
    cancel: std::rc::Rc<dyn Fn(&mut App)>,
}
impl HostConnectionTest {
    pub(in crate::ui) fn new(in_progress: bool, cancel: impl Fn(&mut App) + 'static) -> Self {
        Self {
            in_progress,
            cancel: std::rc::Rc::new(cancel),
        }
    }
    pub(in crate::ui) fn cancel(&self, cx: &mut App) {
        (self.cancel)(cx);
    }
}

fn resolve_profile_index(profiles: &[SessionProfile], id: &str) -> Option<usize> {
    profiles.iter().position(|profile| profile.id == id)
}

impl HostsController {
    pub(in crate::ui) fn current_overlay(
        &self,
        controller: Entity<Self>,
    ) -> Option<OverlaySnapshot> {
        self.profile_delete_overlay(controller.clone())
            .or_else(|| self.profile_import_result_overlay(controller))
    }

    fn profile_delete_overlay(&self, controller: Entity<Self>) -> Option<OverlaySnapshot> {
        let prompt = self.pending_profile_delete()?;
        let revision = self.overlay_revisions.borrow().current("profile-delete");
        let render_controller = controller.clone();
        let dismiss_controller = controller.clone();
        Some(
            OverlaySnapshot::new(
                format!("hosts:profile-delete:{}", prompt.profile_id),
                revision,
                move |progress, _, _| {
                    Self::render_profile_delete_prompt(
                        render_controller.clone(),
                        &prompt,
                        revision,
                        progress,
                    )
                },
                move |_, cx| {
                    dismiss_controller.update(cx, |this, cx| {
                        if this.overlay_revisions.borrow().current("profile-delete") == revision {
                            this.cancel_profile_delete(cx);
                        }
                    });
                },
            )
            .with_submit(move |window, cx| {
                controller.update(cx, |this, cx| {
                    if this.overlay_revisions.borrow().current("profile-delete") == revision {
                        this.confirm_profile_delete(window, cx);
                    }
                });
            }),
        )
    }

    fn profile_import_result_overlay(&self, controller: Entity<Self>) -> Option<OverlaySnapshot> {
        let prompt = self.pending_profile_import_result()?;
        let revision = self
            .overlay_revisions
            .borrow()
            .current("profile-import-result");
        let render_controller = controller.clone();
        Some(OverlaySnapshot::new(
            "hosts:profile-import-result",
            revision,
            move |progress, _, _| {
                Self::render_profile_import_result(
                    render_controller.clone(),
                    &prompt,
                    revision,
                    progress,
                )
            },
            move |_, cx| {
                controller.update(cx, |this, cx| {
                    if this
                        .overlay_revisions
                        .borrow()
                        .current("profile-import-result")
                        == revision
                    {
                        this.dismiss_profile_import_result(cx);
                    }
                });
            },
        ))
    }
}

pub(in crate::ui) fn initial_profile_selection(
    profiles: &[SessionProfile],
    selected_profile: Option<usize>,
) -> InitialProfileSelection {
    let selected_profile_data = selected_profile.and_then(|index| profiles.get(index).cloned());
    let editing_auth_method = selected_profile_data
        .as_ref()
        .map(SessionProfile::effective_auth_method)
        .map(HostsController::host_editor_auth_method)
        .unwrap_or_default();
    let available_groups = HostsController::collect_available_groups(profiles);
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

pub(in crate::ui) struct InitialProfileSelection {
    pub(in crate::ui) selected_profile_data: Option<SessionProfile>,
    pub(in crate::ui) editing_auth_method: AuthMethod,
    pub(in crate::ui) available_groups: Vec<String>,
    pub(in crate::ui) selected_group: String,
    pub(in crate::ui) selected_existing_group: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::contracts::{CatalogReader, ProfileCommitOperation, ProfileCommitter};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{Render, TestAppContext, WindowBounds, WindowOptions, point, px, size};
    use miaominal_secrets::CredentialStore;
    use miaominal_secrets::credential_backend::CredentialBackend;

    struct EmptyCredentials;
    impl CredentialBackend for EmptyCredentials {
        fn name(&self) -> &'static str {
            "host-tests"
        }
        fn get(&self, _: &str, _: &str) -> anyhow::Result<Option<String>> {
            Ok(None)
        }
        fn set(&self, _: &str, _: &str, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn delete(&self, _: &str, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct CatalogFixture {
        profiles: RefCell<Vec<SessionProfile>>,
        commits: Cell<usize>,
    }

    impl CatalogReader for CatalogFixture {
        fn profiles(&self) -> Vec<SessionProfile> {
            self.profiles.borrow().clone()
        }
        fn profile(&self, id: &str) -> Option<SessionProfile> {
            self.profiles
                .borrow()
                .iter()
                .find(|profile| profile.id == id)
                .cloned()
        }
        fn proxies(&self) -> Vec<ProxyProfile> {
            Vec::new()
        }
    }

    impl ProfileCommitter for CatalogFixture {
        fn commit(
            &self,
            _: &mut App,
            operation: ProfileCommitOperation,
        ) -> anyhow::Result<Vec<SessionProfile>> {
            let profiles = operation(self.profiles())?;
            if profiles != *self.profiles.borrow() {
                self.profiles.replace(profiles.clone());
                self.commits.set(self.commits.get() + 1);
            }
            Ok(profiles)
        }
    }

    struct Harness {
        controller: Entity<HostsController>,
    }
    impl Render for Harness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let save_controller = self.controller.clone();
            div()
                .size_full()
                .child(self.controller.read(cx).render_hosts_editor_sidebar(
                    self.controller.clone(),
                    HostConnectionTest::new(false, |_| {}),
                    |_, _| {},
                    move |window, cx| {
                        save_controller.update(cx, |controller, cx| {
                            controller.request_profile_save(false, window, cx)
                        })
                    },
                    |_, _| {},
                    cx,
                ))
        }
    }

    fn test_args(
        catalog: &Rc<CatalogFixture>,
        profiles: Vec<SessionProfile>,
    ) -> HostsControllerArgs {
        HostsControllerArgs {
            catalog_query: CatalogQueryPort::new(catalog.clone()),
            profile_commit: ProfileCommitPort::new(catalog.clone()),
            session_store: None,
            secrets: SecretStore::with_credentials(CredentialStore::with_backend(
                "host-tests",
                EmptyCredentials,
            )),
            selected_profile: (!profiles.is_empty()).then_some(0),
            profiles,
            proxies: vec![],
            managed_keys: vec![],
            local_vault_status: LocalVaultStatus::Disabled,
        }
    }

    #[gpui_kit::test]
    fn failed_saves_keep_drafts_and_refreshes_do_not_publish(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let catalog = Rc::new(CatalogFixture::default());
        let source = catalog.clone();
        let (view, cx) = cx.add_window_view(move |window, cx| {
            let mut profile = SessionProfile::blank("alpha", 1);
            profile.host = "example.test".into();
            profile.username = "user".into();
            profile.auth_method = Some(AuthMethod::KeyboardInteractive);
            let controller = cx.new(|cx| {
                HostsController::new(
                    {
                        source.profiles.replace(vec![profile.clone()]);
                        let mut args = test_args(&source, vec![profile]);
                        args.session_store = Some(SessionStore::with_path(std::env::temp_dir()));
                        args
                    },
                    window,
                    cx,
                )
            });
            Harness { controller }
        });
        let controller = view.read_with(cx, |view, _| view.controller.clone());
        cx.update(|window, cx| {
            controller.update(cx, |controller, cx| {
                controller.open_profile_editor(0, vec![], window, cx);
                let forms = controller.host_editor_forms();
                set_input_value(&forms.name_input, "Unsaved draft", window, cx);
                let before = controller.profiles.borrow().clone();
                controller
                    .read_profile_save_draft(cx)
                    .expect("valid editor draft");
                controller.request_profile_save(false, window, cx);
                assert!(controller.editor_state().host_editor_open);
                assert_eq!(forms.name_input.read(cx).value(), "Unsaved draft");
                assert_eq!(*controller.profiles.borrow(), before);
                controller.toggle_profile_favorite(0, cx);
                controller.duplicate_profile_at_index(0, cx);
                assert_eq!(*controller.profiles.borrow(), before);
                controller.replace_profiles_from_application(before.clone());
                cx.notify();
                assert_eq!(forms.name_input.read(cx).value(), "Unsaved draft");
                controller
                    .host_editor_scroll_handle()
                    .set_offset(point(px(0.0), px(-120.0)));
                controller.open_profile_editor(0, vec![], window, cx);
                assert_eq!(
                    controller.host_editor_scroll_handle().offset(),
                    point(px(0.0), px(0.0))
                );
                controller.replace_profiles_from_application(vec![]);
                assert!(controller.read_profile_save_draft(cx).is_err());
                assert_eq!(controller.current_profile_id(), "alpha");
                controller.replace_profiles_from_application(before);
            })
        });
        assert_eq!(catalog.commits.get(), 0);
        let store_path = std::env::temp_dir().join(format!(
            "miaominal-host-commit-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        cx.update(|window, cx| {
            controller.update(cx, |controller, cx| {
                controller.services.session_store =
                    Some(SessionStore::with_path(store_path.clone()));
                controller.open_profile_editor(0, vec![], window, cx);
                let forms = controller.host_editor_forms();
                set_input_value(&forms.name_input, "Committed", window, cx);
                controller.request_profile_save(false, window, cx);
                assert!(!controller.editor_state().host_editor_open);
                assert_eq!(controller.profiles.borrow()[0].name, "Committed");
            })
        });
        assert_eq!(catalog.commits.get(), 1);
        cx.update(|_, cx| {
            controller.update(cx, |controller, cx| {
                let snapshot = controller.profiles.borrow().clone();
                controller.replace_profiles_from_application(snapshot);
                cx.notify();
            })
        });
        assert_eq!(catalog.commits.get(), 1);
        std::fs::remove_file(store_path).unwrap();
    }

    #[gpui_kit::test]
    fn local_profile_editor_accepts_keyboard_input_and_saves_through_pointer(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let catalog = Rc::new(CatalogFixture::default());
        let source = catalog.clone();
        let store_path = std::env::temp_dir().join(format!(
            "miaominal-host-local-ui-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (window_handle, harness) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        Default::default(),
                        size(px(480.), px(1200.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        let controller = cx.new(|cx| {
                            let mut args = test_args(&source, vec![]);
                            args.session_store = Some(SessionStore::with_path(store_path.clone()));
                            HostsController::new(args, window, cx)
                        });
                        controller.update(cx, |controller, cx| {
                            controller.add_profile(vec![], window, cx)
                        });
                        Harness { controller }
                    })
                },
            )
            .expect("open host editor test window")
        });
        let controller = cx.update(|cx| harness.read(cx).controller.clone());
        let forms = cx.update(|cx| controller.read(cx).host_editor_forms());
        let name_id = ("input", forms.name_input.entity_id());
        let shell_id = ("input", forms.local_shell_input.entity_id());
        let args_id = ("input", forms.local_shell_args_input.entity_id());
        cx.update_window(window_handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find(shell_id).is_none());
            let kind = window.find("host-editor-profile-kind-section").bounds();
            window.click_at(
                "host-editor-profile-kind-section",
                point(kind.size.width / 2. + px(80.), px(17.)),
                cx,
            );
            assert_eq!(
                controller.read(cx).host_editor_forms().profile_kind,
                ProfileKind::Local
            );
            assert!(window.find(shell_id).visible());
            window.click("host-editor-save", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(catalog.commits.get(), 0);
        assert!(cx.update(|cx| controller.read(cx).editor_state().host_editor_open));
        let executable = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        cx.update_window(window_handle, |_, window, cx| {
            window.click(name_id, cx);
            window.press("ctrl-a", cx);
            window.input("本地草稿x", cx);
            window.press("backspace", cx);
            assert_eq!(window.find(name_id).value(), Some("本地草稿"));
            assert_eq!(window.find(name_id).focused(), Some(true));
            window.click(shell_id, cx);
            window.input(&executable, cx);
            assert_eq!(window.find(shell_id).value(), Some(executable.as_str()));
            window.click(args_id, cx);
            window.input("--login", cx);
            assert_eq!(window.find(args_id).value(), Some("--login"));
            controller
                .read(cx)
                .read_profile_save_draft(cx)
                .expect("UI produced a valid local profile draft");
            window.click("host-editor-save", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(catalog.commits.get(), 1);
        let profile = &catalog.profiles.borrow()[0];
        assert_eq!(profile.kind, ProfileKind::Local);
        assert_eq!(profile.name, "本地草稿");
        assert_eq!(profile.local_shell, executable);
        assert_eq!(profile.local_shell_args, "--login");
        assert!(profile.host.is_empty());
        assert!(profile.proxy_jump_profile_ids.is_empty());
        assert!(!cx.update(|cx| controller.read(cx).editor_state().host_editor_open));
        std::fs::remove_file(store_path).unwrap();
    }

    #[test]
    fn editor_submission_rebases_stable_identity_and_preserves_other_profile_owners() {
        let service = ProfileService::new(
            None,
            SecretStore::with_credentials(CredentialStore::with_backend(
                "host-tests",
                EmptyCredentials,
            )),
        );
        let original = SessionProfile::blank("alpha", 1);
        let mut submitted = original.clone();
        submitted.name = "Edited".into();
        let mut latest = original;
        latest.is_favorite = true;
        latest.last_connected_at = Some(99);
        latest.remote_path_favorites = vec!["/newer".into()];
        latest.has_stored_password = true;
        latest.has_stored_passphrase = true;
        latest.port_forwarding_rules = vec![PortForwardRule {
            id: "newer".into(),
            label: "Newer rule".into(),
            kind: PortForwardKind::Local,
            listen_host: "127.0.0.1".into(),
            listen_port: 8000,
            target_host: "db".into(),
            target_port: 5432,
            enabled: true,
        }];
        let profiles = vec![SessionProfile::blank("beta", 2), latest.clone()];
        let (rebased, index) = rebase_host_submission(
            &service,
            &profiles,
            HostSaveSubmission {
                editor_revision: 0,
                profile: submitted.clone(),
                is_new: false,
            },
        )
        .unwrap();
        assert_eq!(index, Some(1));
        assert_eq!(rebased.name, "Edited");
        assert_eq!(rebased.port_forwarding_rules, latest.port_forwarding_rules);
        assert_eq!(rebased.remote_path_favorites, latest.remote_path_favorites);
        assert_eq!(rebased.last_connected_at, Some(99));
        assert!(
            rebased.is_favorite && rebased.has_stored_password && rebased.has_stored_passphrase
        );
        assert!(
            rebase_host_submission(
                &service,
                &profiles[..1],
                HostSaveSubmission {
                    editor_revision: 0,
                    profile: submitted,
                    is_new: false
                }
            )
            .is_err()
        );
    }

    #[gpui_kit::test]
    fn unlock_continuation_commits_captured_draft_and_rejects_deleted_identity(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let mut profile = SessionProfile::blank("alpha", 1);
        profile.host = "example.test".into();
        profile.username = "user".into();
        profile.auth_method = Some(AuthMethod::KeyboardInteractive);
        let catalog = Rc::new(CatalogFixture {
            profiles: RefCell::new(vec![profile.clone()]),
            ..Default::default()
        });
        let source = catalog.clone();
        let store_path = std::env::temp_dir().join(format!(
            "miaominal-host-unlock-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (view, cx) = cx.add_window_view(|window, cx| {
            let controller = cx.new(|cx| {
                let mut args = test_args(&source, vec![profile]);
                args.session_store = Some(SessionStore::with_path(store_path.clone()));
                HostsController::new(args, window, cx)
            });
            Harness { controller }
        });
        let controller = view.read_with(cx, |view, _| view.controller.clone());
        cx.update(|window, cx| {
            controller.update(cx, |controller, cx| {
                controller.open_profile_editor(0, vec![], window, cx);
                let forms = controller.host_editor_forms();
                set_input_value(&forms.name_input, "Submitted before unlock", window, cx);
                controller.request_profile_save(true, window, cx);
                set_input_value(&forms.name_input, "Later form change", window, cx);
                let mut latest = source.profiles()[0].clone();
                latest.is_favorite = true;
                latest.remote_path_favorites = vec!["/updated-during-unlock".into()];
                let reordered = vec![SessionProfile::blank("beta", 2), latest];
                source.profiles.replace(reordered.clone());
                controller.replace_profiles_from_application(reordered);
                controller.continue_save_profile_after_unlock(window, cx);
                assert!(!controller.editor_state().host_editor_open);
                assert_eq!(controller.selected_profile(), Some(1));
                assert_eq!(source.profiles()[1].name, "Submitted before unlock");
                assert!(source.profiles()[1].is_favorite);
                assert_eq!(
                    source.profiles()[1].remote_path_favorites,
                    vec!["/updated-during-unlock"]
                );
                controller.open_profile_editor(1, vec![], window, cx);
                set_input_value(&forms.name_input, "Deleted while unlocking", window, cx);
                controller.request_profile_save(true, window, cx);
                source.profiles.replace(vec![source.profiles()[0].clone()]);
                controller.continue_save_profile_after_unlock(window, cx);
                assert!(controller.editor_state().host_editor_open);
                assert_eq!(forms.name_input.read(cx).value(), "Deleted while unlocking");
                assert!(source.profile("alpha").is_none());
            });
        });
        assert_eq!(catalog.commits.get(), 1);
        std::fs::remove_file(store_path).unwrap();
    }

    #[gpui_kit::test]
    fn replacing_or_closing_host_editor_cancels_deferred_and_stale_saves(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut alpha = SessionProfile::blank("alpha", 1);
        alpha.host = "alpha.example".into();
        alpha.username = "alice".into();
        alpha.auth_method = Some(AuthMethod::KeyboardInteractive);
        let mut beta = SessionProfile::blank("beta", 2);
        beta.host = "beta.example".into();
        beta.username = "bob".into();
        beta.auth_method = Some(AuthMethod::KeyboardInteractive);
        let original = vec![alpha.clone(), beta];
        let catalog = Rc::new(CatalogFixture::default());
        catalog.profiles.replace(original.clone());
        let path = std::env::temp_dir().join(format!(
            "miaominal-host-editor-replacement-{}.json",
            uuid::Uuid::new_v4()
        ));
        let (window_handle, view) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let controller = cx.new(|cx| {
                        let mut args = test_args(&catalog, original.clone());
                        args.session_store = Some(SessionStore::with_path(path.clone()));
                        HostsController::new(args, window, cx)
                    });
                    Harness { controller }
                })
            })
            .expect("open host replacement test window")
        });
        let controller = cx.update(|cx| view.read(cx).controller.clone());
        cx.update_window(window_handle, |_, window, cx| {
            controller.update(cx, |controller, cx| {
                controller.open_profile_editor(0, vec![], window, cx);
                controller.request_profile_save(true, window, cx);
                let stale = controller
                    .pending_save
                    .borrow()
                    .clone()
                    .expect("deferred alpha save");
                controller.open_profile_editor(1, vec![], window, cx);
                assert!(controller.pending_save.borrow().is_none());
                controller.commit_profile_submission(stale, vec![], window, cx);
                controller.continue_save_profile_after_unlock(window, cx);
                assert!(controller.editor_state().host_editor_open);
                assert_eq!(
                    controller.current_host_editor_profile_id().as_deref(),
                    Some("beta")
                );
                controller.request_profile_save(true, window, cx);
                let stale = controller
                    .pending_save
                    .borrow()
                    .clone()
                    .expect("deferred beta save");
                controller.close_host_editor(cx);
                assert!(controller.pending_save.borrow().is_none());
                controller.commit_profile_submission(stale, vec![], window, cx);
                controller.continue_save_profile_after_unlock(window, cx);
                assert!(!controller.editor_state().host_editor_open);
            });
        })
        .unwrap();
        assert_eq!(catalog.commits.get(), 0);
        assert_eq!(catalog.profiles(), original);
        assert!(!path.exists());
    }

    #[test]
    fn new_submission_allocates_an_id_against_the_latest_catalog() {
        let service = ProfileService::new(
            None,
            SecretStore::with_credentials(CredentialStore::with_backend(
                "host-tests",
                EmptyCredentials,
            )),
        );
        let draft = SessionProfile::blank("profile-1", 1);
        let profiles = vec![draft.clone(), SessionProfile::blank("profile-2", 2)];
        let (submitted, index) = rebase_host_submission(
            &service,
            &profiles,
            HostSaveSubmission {
                editor_revision: 0,
                profile: draft,
                is_new: true,
            },
        )
        .unwrap();
        assert!(index.is_none());
        assert!(profiles.iter().all(|existing| existing.id != submitted.id));
    }

    #[test]
    fn profile_actions_resolve_stable_id_after_display_reordering() {
        let profiles = vec![
            SessionProfile::blank("alpha", 1),
            SessionProfile::blank("beta", 2),
        ];
        assert_eq!(resolve_profile_index(&profiles, "beta"), Some(1));
        let reordered = vec![profiles[1].clone(), profiles[0].clone()];
        assert_eq!(resolve_profile_index(&reordered, "beta"), Some(0));
        assert_eq!(resolve_profile_index(&reordered, "missing"), None);
    }
}
