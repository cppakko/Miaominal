use super::{AppCommand, KeychainDeferredCommand};
use crate::ui::i18n;
use crate::ui::shell::*;
use gpui::EventEmitter;
use miaominal_core::keychain::{ManagedKeyRecord, ManagedKeySource};
use miaominal_secrets::SecretStore;
use miaominal_ssh::AgentIdentitySummary;
use miaominal_storage::keychain_store::ManagedKeyStore;
use miaominal_storage::known_hosts_store::KnownHostsStore;

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

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct PendingManagedKeyDeleteState {
    pub(in crate::ui::shell) key_id: String,
    pub(in crate::ui::shell) key_name: String,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct PendingManagedKeyRenameState {
    pub(in crate::ui::shell) key_id: String,
    pub(in crate::ui::shell) current_name: String,
}

#[derive(Clone)]
pub(in crate::ui::shell) struct KeychainForms {
    pub(in crate::ui::shell) filter_input: Entity<InputState>,
    pub(in crate::ui::shell) name_input: Entity<InputState>,
    pub(in crate::ui::shell) rename_name_input: Entity<InputState>,
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

pub(in crate::ui::shell) struct KeychainControllerArgs {
    pub managed_keys: Vec<ManagedKeyRecord>,
    pub runtime: TokioHandle,
    pub keychain_store: Option<ManagedKeyStore>,
    pub secrets: SecretStore,
    pub known_hosts: KnownHostsStore,
    pub local_vault_status: LocalVaultStatus,
}

pub(in crate::ui::shell) struct KeychainController {
    pub(in crate::ui::shell) forms: KeychainForms,
    pub(in crate::ui::shell) managed_keys: Vec<ManagedKeyRecord>,
    pub(in crate::ui::shell) agent_identities: Vec<AgentIdentitySummary>,
    pub(in crate::ui::shell) session_query: SessionQueryPort,
    pub(in crate::ui::shell) runtime: TokioHandle,
    pub(in crate::ui::shell) keychain_store: Option<ManagedKeyStore>,
    pub(in crate::ui::shell) secrets: SecretStore,
    pub(in crate::ui::shell) known_hosts: KnownHostsStore,
    pub(in crate::ui::shell) local_vault_status: LocalVaultStatus,
    pub(in crate::ui::shell) page_view: KeychainPageView,
    pub(in crate::ui::shell) editor_mode: KeychainEditorMode,
    pub(in crate::ui::shell) editor_open: bool,
    pub(in crate::ui::shell) deploy_in_progress: bool,
    pub(in crate::ui::shell) editor_draft_source: Option<ManagedKeySource>,
    pub(in crate::ui::shell) deploy_key_id: Option<String>,
    pub(in crate::ui::shell) pending_managed_key_delete: Option<PendingManagedKeyDeleteState>,
    pub(in crate::ui::shell) pending_managed_key_rename: Option<PendingManagedKeyRenameState>,
    pub(in crate::ui::shell) status_message: String,
    pub(in crate::ui::shell) import_task: Option<gpui::Task<()>>,
    pub(in crate::ui::shell) deploy_task: Option<gpui::Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl KeychainController {
    fn build_forms(
        session_query: &SessionQueryPort,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> KeychainForms {
        let filter_input = new_input_state(
            i18n::string("placeholders.keychain.filter"),
            "",
            false,
            window,
            cx,
        );
        let name_input = new_input_state(
            i18n::string("placeholders.keychain.managed_key_name"),
            "",
            false,
            window,
            cx,
        );
        let rename_name_input = new_input_state(
            i18n::string("dialogs.managed_key_rename.name_label"),
            "",
            false,
            window,
            cx,
        );
        let import_path_input = new_input_state(
            i18n::string("placeholders.keychain.import_private_key_path"),
            "",
            false,
            window,
            cx,
        );
        let import_private_key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("bash")
                .indent_guides(false)
                .folding(false)
                .searchable(false)
                .rows(6)
                .tab_size(TabSize {
                    tab_size: 2,
                    ..Default::default()
                })
                .placeholder("")
        });
        set_code_editor_input_placeholder(
            &import_private_key_input,
            i18n::string("placeholders.keychain.import_private_key_body"),
            false,
            window,
            cx,
        );
        let import_public_key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("bash")
                .indent_guides(false)
                .folding(false)
                .searchable(false)
                .rows(4)
                .tab_size(TabSize {
                    tab_size: 2,
                    ..Default::default()
                })
                .placeholder("")
        });
        set_code_editor_input_placeholder(
            &import_public_key_input,
            i18n::string("placeholders.keychain.import_public_key_body"),
            false,
            window,
            cx,
        );
        let import_passphrase_input = new_input_state(
            i18n::string("placeholders.keychain.import_passphrase_optional"),
            "",
            true,
            window,
            cx,
        );
        let deploy_profile_select = cx.new(|cx| {
            SelectState::new(
                Self::keychain_deploy_profile_options(&session_query.profiles()),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        let deploy_location_input = new_input_state(
            i18n::string("placeholders.keychain.deploy_location"),
            KEYCHAIN_DEPLOY_DEFAULT_LOCATION,
            false,
            window,
            cx,
        );
        let deploy_filename_input = new_input_state(
            i18n::string("placeholders.keychain.deploy_filename"),
            KEYCHAIN_DEPLOY_DEFAULT_FILENAME,
            false,
            window,
            cx,
        );
        let deploy_command_input = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("bash")
                .indent_guides(false)
                .folding(false)
                .searchable(false)
                .rows(8)
                .tab_size(TabSize {
                    tab_size: 2,
                    ..Default::default()
                })
                .placeholder("")
                .default_value(KEYCHAIN_DEPLOY_DEFAULT_COMMAND)
        });
        set_code_editor_input_placeholder(
            &deploy_command_input,
            i18n::string("placeholders.keychain.deploy_command"),
            false,
            window,
            cx,
        );

        KeychainForms {
            filter_input,
            name_input,
            rename_name_input,
            import_path_input,
            import_private_key_input,
            import_public_key_input,
            import_passphrase_input,
            deploy_profile_select,
            deploy_location_input,
            deploy_filename_input,
            deploy_command_input,
        }
    }

    pub(in crate::ui::shell) fn refresh_localized_placeholders(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (input, key) in [
            (
                &self.forms.name_input,
                "placeholders.keychain.managed_key_name",
            ),
            (
                &self.forms.rename_name_input,
                "dialogs.managed_key_rename.name_label",
            ),
            (
                &self.forms.import_path_input,
                "placeholders.keychain.import_private_key_path",
            ),
            (
                &self.forms.import_passphrase_input,
                "placeholders.keychain.import_passphrase_optional",
            ),
            (&self.forms.filter_input, "placeholders.keychain.filter"),
            (
                &self.forms.deploy_location_input,
                "placeholders.keychain.deploy_location",
            ),
            (
                &self.forms.deploy_filename_input,
                "placeholders.keychain.deploy_filename",
            ),
        ] {
            set_input_placeholder(input, i18n::string(key), window, cx);
        }
        set_code_editor_input_placeholder(
            &self.forms.import_private_key_input,
            i18n::string("placeholders.keychain.import_private_key_body"),
            false,
            window,
            cx,
        );
        set_code_editor_input_placeholder(
            &self.forms.import_public_key_input,
            i18n::string("placeholders.keychain.import_public_key_body"),
            false,
            window,
            cx,
        );
        set_code_editor_input_placeholder(
            &self.forms.deploy_command_input,
            i18n::string("placeholders.keychain.deploy_command"),
            false,
            window,
            cx,
        );
    }

    pub(in crate::ui::shell) fn new(
        args: KeychainControllerArgs,
        session_query: SessionQueryPort,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let forms = Self::build_forms(&session_query, window, cx);
        let filter_input = forms.filter_input.clone();
        let filter_subscription =
            cx.subscribe(&filter_input, |_: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });

        Self {
            forms,
            managed_keys: args.managed_keys,
            agent_identities: Vec::new(),
            session_query,
            runtime: args.runtime,
            keychain_store: args.keychain_store,
            secrets: args.secrets,
            known_hosts: args.known_hosts,
            local_vault_status: args.local_vault_status,
            page_view: KeychainPageView::ManagedKeys,
            editor_mode: KeychainEditorMode::Import,
            editor_open: false,
            deploy_in_progress: false,
            editor_draft_source: None,
            deploy_key_id: None,
            pending_managed_key_delete: None,
            pending_managed_key_rename: None,
            status_message: String::new(),
            import_task: None,
            deploy_task: None,
            _subscriptions: vec![filter_subscription],
        }
    }

    pub(in crate::ui::shell) fn status_message(&self) -> &str {
        &self.status_message
    }

    pub(in crate::ui::shell) fn managed_keys(&self) -> &[ManagedKeyRecord] {
        &self.managed_keys
    }

    pub(in crate::ui::shell) fn managed_key_ids(&self) -> Vec<String> {
        self.managed_keys.iter().map(|key| key.id.clone()).collect()
    }

    pub(in crate::ui::shell) fn editor_open(&self) -> bool {
        self.editor_open
    }

    pub(in crate::ui::shell) fn dismiss_editor(&mut self, cx: &mut Context<Self>) {
        if self.editor_open {
            self.editor_open = false;
            self.editor_mode = KeychainEditorMode::Import;
            self.deploy_key_id = None;
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn pending_managed_key_delete(
        &self,
    ) -> Option<PendingManagedKeyDeleteState> {
        self.pending_managed_key_delete.clone()
    }

    pub(in crate::ui::shell) fn pending_managed_key_rename(
        &self,
    ) -> Option<PendingManagedKeyRenameState> {
        self.pending_managed_key_rename.clone()
    }

    pub(in crate::ui::shell) fn managed_key_rename_input(&self) -> Entity<InputState> {
        self.forms.rename_name_input.clone()
    }

    pub(in crate::ui::shell) fn take_pending_managed_key_delete(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<PendingManagedKeyDeleteState> {
        let pending = self.pending_managed_key_delete.take();
        if pending.is_some() {
            cx.notify();
        }
        pending
    }

    pub(in crate::ui::shell) fn confirm_managed_key_delete(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.take_pending_managed_key_delete(cx) else {
            return;
        };
        cx.emit(AppCommand::OverlayDismissed(
            DialogOverlaySnapshot::ManagedKeyDelete(pending.clone()),
        ));
        self.delete_managed_key(&pending.key_id, window, cx);
    }

    pub(in crate::ui::shell) fn cancel_managed_key_delete(&mut self, cx: &mut Context<Self>) {
        if let Some(pending) = self.take_pending_managed_key_delete(cx) {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::ManagedKeyDelete(pending),
            ));
        }
    }

    pub(in crate::ui::shell) fn replace_managed_keys(
        &mut self,
        managed_keys: Vec<ManagedKeyRecord>,
        cx: &mut Context<Self>,
    ) {
        self.managed_keys = managed_keys;
        cx.notify();
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

    pub(in crate::ui::shell) fn resume_deferred(
        &mut self,
        command: KeychainDeferredCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            KeychainDeferredCommand::ImportManagedKey => {
                self.continue_import_managed_key_after_unlock(window, cx)
            }
            KeychainDeferredCommand::DeployManagedKey => self.deploy_managed_key(window, cx),
        }
    }
}

impl EventEmitter<AppCommand> for KeychainController {}
