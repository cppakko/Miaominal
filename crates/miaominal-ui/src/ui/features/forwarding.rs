use super::*;
use crate::ui::{
    contracts::{CatalogQueryPort, ForwardingRuntimePort, ProfileCommitPort},
    i18n,
};
use gpui_kit::component::{
    input::InputEvent,
    select::{SearchableVec, SelectEvent, SelectState},
};
use gpui_kit::{EventEmitter, Subscription};
use miaominal_secrets::SecretStore;
use miaominal_services::{PortForwardManager, PortForwardRuntimeState, ProfileService};
use miaominal_storage::config_store::store::SessionStore;
use std::cell::{Cell, RefCell};

#[path = "forwarding/dialogs.rs"]
mod dialogs;
#[path = "forwarding/view/mod.rs"]
mod view;

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct PendingPortForwardRuleDeleteState {
    pub(in crate::ui::shell) profile_id: String,
    pub(in crate::ui::shell) rule_id: String,
    pub(in crate::ui::shell) profile_label: String,
    pub(in crate::ui::shell) rule_label: String,
}

#[derive(Clone)]
struct PortForwardRuleInputValues {
    profile_id: String,
    kind: PortForwardKind,
    resolved_label: String,
    listen_host: String,
    listen_port: u16,
    target_host: String,
    target_port: u16,
}

#[derive(Clone)]
struct PortForwardingForms {
    filter_input: Entity<InputState>,
    profile_select: Entity<SelectState<SearchableVec<ForwardProfileSelectItem>>>,
    label_input: Entity<InputState>,
    listen_host_input: Entity<InputState>,
    listen_port_input: Entity<InputState>,
    target_host_input: Entity<InputState>,
    target_port_input: Entity<InputState>,
}

#[derive(Clone)]
struct ForwardEditorState {
    port_forward_editor_open: bool,
    port_forward_editor_profile_id: Option<String>,
    port_forward_editor_rule_id: Option<String>,
    port_forward_kind: PortForwardKind,
}

#[derive(Clone)]
struct ForwardSaveDraft {
    editor_revision: u64,
    values: PortForwardRuleInputValues,
    rule_id: Option<String>,
}

enum ForwardingChange {
    Save(ForwardSaveDraft),
    Duplicate { profile_id: String, rule_id: String },
    Delete { profile_id: String, rule_id: String },
    Disable { profile_id: String, rule_id: String },
}

struct ForwardingChangeOutcome {
    profile_id: String,
    profile_name: String,
    rule: PortForwardRule,
    rules: Vec<PortForwardRule>,
}

fn apply_forwarding_change(
    profiles: &mut [SessionProfile],
    change: ForwardingChange,
) -> anyhow::Result<ForwardingChangeOutcome> {
    let profile_id = match &change {
        ForwardingChange::Save(draft) => &draft.values.profile_id,
        ForwardingChange::Duplicate { profile_id, .. }
        | ForwardingChange::Delete { profile_id, .. }
        | ForwardingChange::Disable { profile_id, .. } => profile_id,
    };
    let profile = profiles
        .iter_mut()
        .find(|profile| &profile.id == profile_id && !profile.is_local())
        .ok_or_else(|| {
            anyhow::anyhow!(i18n::string(
                "errors.forwarding.validation.selected_host_profile_missing"
            ))
        })?;
    let locate = |id: &str, rules: &[PortForwardRule]| {
        rules.iter().position(|rule| rule.id == id).ok_or_else(|| {
            anyhow::anyhow!(i18n::string("forwarding.messages.rule_no_longer_exists"))
        })
    };
    let rule = match change {
        ForwardingChange::Save(draft) => {
            let existing = draft
                .rule_id
                .as_ref()
                .map(|id| locate(id, &profile.port_forwarding_rules))
                .transpose()?;
            let values = draft.values;
            let rule = PortForwardRule {
                id: draft
                    .rule_id
                    .unwrap_or_else(|| format!("pf-{}", uuid::Uuid::new_v4())),
                label: values.resolved_label,
                kind: values.kind,
                listen_host: values.listen_host,
                listen_port: values.listen_port,
                target_host: values.target_host,
                target_port: values.target_port,
                enabled: existing
                    .map(|i| profile.port_forwarding_rules[i].enabled)
                    .unwrap_or(true),
            };
            if let Some(index) = existing {
                profile.port_forwarding_rules[index] = rule.clone();
            } else {
                profile.port_forwarding_rules.push(rule.clone());
            }
            rule
        }
        ForwardingChange::Duplicate { rule_id, .. } => {
            let index = locate(&rule_id, &profile.port_forwarding_rules)?;
            let mut rule = profile.port_forwarding_rules[index].clone();
            rule.id = format!("pf-{}", uuid::Uuid::new_v4());
            rule.label = i18n::string_args(
                "forwarding.messages.duplicate_label",
                &[("label", &ForwardingController::rule_summary_label(&rule))],
            );
            profile
                .port_forwarding_rules
                .insert(index + 1, rule.clone());
            rule
        }
        ForwardingChange::Delete { rule_id, .. } => {
            let index = locate(&rule_id, &profile.port_forwarding_rules)?;
            profile.port_forwarding_rules.remove(index)
        }
        ForwardingChange::Disable { rule_id, .. } => {
            let index = locate(&rule_id, &profile.port_forwarding_rules)?;
            profile.port_forwarding_rules[index].enabled = false;
            profile.port_forwarding_rules[index].clone()
        }
    };
    Ok(ForwardingChangeOutcome {
        profile_id: profile.id.clone(),
        profile_name: profile.name.clone(),
        rule,
        rules: profile.port_forwarding_rules.clone(),
    })
}

pub(in crate::ui::shell) struct ForwardingController {
    catalog: CatalogQueryPort,
    profile_commit: ProfileCommitPort,
    store: Option<SessionStore>,
    secrets: SecretStore,
    vault_status: LocalVaultStatus,
    manager: Option<PortForwardManager>,
    runtime: ForwardingRuntimePort,
    forms: PortForwardingForms,
    editor_state: RefCell<ForwardEditorState>,
    view_mode: Cell<ProfileViewMode>,
    pending_delete: RefCell<Option<PendingPortForwardRuleDeleteState>>,
    pending_save: RefCell<Option<ForwardSaveDraft>>,
    editor_revision: Cell<u64>,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<AppCommand> for ForwardingController {}

pub(in crate::ui::shell) struct ForwardingControllerArgs {
    pub catalog: CatalogQueryPort,
    pub profile_commit: ProfileCommitPort,
    pub store: Option<SessionStore>,
    pub secrets: SecretStore,
    pub vault_status: LocalVaultStatus,
    pub manager: Option<PortForwardManager>,
    pub runtime: ForwardingRuntimePort,
}

impl ForwardingController {
    pub(in crate::ui::shell) fn new(
        args: ForwardingControllerArgs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let catalog = args.catalog;
        let forms = PortForwardingForms {
            filter_input: new_input_state(
                i18n::string("placeholders.forward.filter"),
                "",
                false,
                window,
                cx,
            ),
            profile_select: cx.new(|cx| {
                SelectState::new(
                    Self::forward_profile_options(&catalog.profiles()),
                    None,
                    window,
                    cx,
                )
                .searchable(true)
            }),
            label_input: new_input_state(
                i18n::string("placeholders.forward.rule_label"),
                "",
                false,
                window,
                cx,
            ),
            listen_host_input: new_input_state(
                i18n::string("placeholders.forward.listen_host"),
                "",
                false,
                window,
                cx,
            ),
            listen_port_input: new_input_state(
                i18n::string("placeholders.forward.listen_port"),
                "",
                false,
                window,
                cx,
            ),
            target_host_input: new_input_state(
                i18n::string("placeholders.forward.target_host"),
                "",
                false,
                window,
                cx,
            ),
            target_port_input: new_input_state(
                i18n::string("placeholders.forward.target_port"),
                "",
                false,
                window,
                cx,
            ),
        };
        let subscriptions = vec![
            cx.subscribe(&forms.filter_input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe(
                &forms.profile_select,
                |controller,
                 _,
                 event: &SelectEvent<SearchableVec<ForwardProfileSelectItem>>,
                 cx| {
                    let SelectEvent::Confirm(profile_id) = event;
                    if controller.editor_state().port_forward_editor_profile_id == *profile_id {
                        return;
                    }
                    controller.select_port_forward_editor_profile(profile_id.clone(), cx);
                },
            ),
        ];
        Self {
            profile_commit: args.profile_commit,
            catalog,
            store: args.store,
            secrets: args.secrets,
            vault_status: args.vault_status,
            manager: args.manager,
            runtime: args.runtime,
            forms,
            editor_state: RefCell::new(ForwardEditorState {
                port_forward_editor_open: false,
                port_forward_editor_profile_id: None,
                port_forward_editor_rule_id: None,
                port_forward_kind: PortForwardKind::Local,
            }),
            view_mode: Cell::new(ProfileViewMode::Grid),
            pending_delete: RefCell::new(None),
            pending_save: RefCell::new(None),
            editor_revision: Cell::new(0),
            _subscriptions: subscriptions,
        }
    }

    pub(in crate::ui::shell) fn refresh_localized_placeholders(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (input, key) in [
            (&self.forms.filter_input, "placeholders.forward.filter"),
            (&self.forms.label_input, "placeholders.forward.rule_label"),
            (
                &self.forms.listen_host_input,
                "placeholders.forward.listen_host",
            ),
            (
                &self.forms.listen_port_input,
                "placeholders.forward.listen_port",
            ),
            (
                &self.forms.target_host_input,
                "placeholders.forward.target_host",
            ),
            (
                &self.forms.target_port_input,
                "placeholders.forward.target_port",
            ),
        ] {
            set_input_placeholder(input, i18n::string(key), window, cx);
        }
    }

    pub(in crate::ui::shell) fn credentials_changed(
        &mut self,
        secrets: SecretStore,
        status: LocalVaultStatus,
        cx: &mut Context<Self>,
    ) {
        self.secrets = secrets;
        self.vault_status = status;
        cx.notify();
    }

    pub(in crate::ui::shell) fn adopt_catalog(&self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.editor_state().port_forward_editor_profile_id;
        self.sync_port_forward_profile_select(selected.as_deref(), window, cx);
        cx.notify();
    }
    fn editor_state(&self) -> ForwardEditorState {
        self.editor_state.borrow().clone()
    }
    pub(in crate::ui::shell) fn editor_open(&self) -> bool {
        self.editor_state.borrow().port_forward_editor_open
    }
    fn profiles(&self) -> Vec<SessionProfile> {
        self.catalog.profiles()
    }
    fn set_forward_view_mode(&self, mode: ProfileViewMode, cx: &mut Context<Self>) {
        self.view_mode.set(mode);
        cx.notify();
    }
    fn port_forward_rule_runtime_state(
        &self,
        profile_id: &str,
        rule_id: &str,
    ) -> Option<PortForwardRuntimeState> {
        self.manager
            .as_ref()?
            .snapshot()
            .session(profile_id, rule_id)
            .map(|runtime| runtime.state.clone())
    }
    pub(in crate::ui::shell) fn pending_port_forward_rule_delete(
        &self,
    ) -> Option<PendingPortForwardRuleDeleteState> {
        self.pending_delete.borrow().clone()
    }
    fn set_pending_port_forward_rule_delete(
        &self,
        pending: Option<PendingPortForwardRuleDeleteState>,
    ) {
        self.pending_delete.replace(pending);
    }
    fn take_pending_port_forward_rule_delete(&self) -> Option<PendingPortForwardRuleDeleteState> {
        self.pending_delete.take()
    }
    fn localized_port_forward_kind_label(kind: PortForwardKind) -> String {
        match kind {
            PortForwardKind::Local => i18n::string("forwarding.editor.local"),
            PortForwardKind::Remote => i18n::string("forwarding.editor.remote"),
        }
    }

    fn rule_summary_label(rule: &PortForwardRule) -> String {
        let label = rule.label.trim();
        if !label.is_empty() {
            return label.to_string();
        }

        format!(
            "{} {}:{} -> {}:{}",
            Self::localized_port_forward_kind_label(rule.kind),
            rule.listen_host,
            rule.listen_port,
            rule.target_host,
            rule.target_port
        )
    }

    fn synced_sessions_suffix(count: usize) -> String {
        if count == 0 {
            String::new()
        } else {
            let count = count.to_string();
            if count == "1" {
                i18n::string_args(
                    "forwarding.messages.synced_suffix_one",
                    &[("count", &count)],
                )
            } else {
                i18n::string_args(
                    "forwarding.messages.synced_suffix_other",
                    &[("count", &count)],
                )
            }
        }
    }

    fn port_forward_rule_indices(&self, profile_id: &str, rule_id: &str) -> Option<(usize, usize)> {
        self.catalog
            .profiles()
            .iter()
            .enumerate()
            .find_map(|(profile_index, profile)| {
                (profile.id == profile_id)
                    .then_some(profile)
                    .and_then(|profile| {
                        profile
                            .port_forwarding_rules
                            .iter()
                            .position(|rule| rule.id == rule_id)
                            .map(|rule_index| (profile_index, rule_index))
                    })
            })
    }

    fn forward_profile_options(
        profiles: &[SessionProfile],
    ) -> SearchableVec<ForwardProfileSelectItem> {
        SearchableVec::new(
            profiles
                .iter()
                .filter(|profile| !profile.is_local())
                .map(ForwardProfileSelectItem::new)
                .collect::<Vec<_>>(),
        )
    }

    fn sync_port_forward_profile_select(
        &self,
        selected_profile_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let options = Self::forward_profile_options(&self.catalog.profiles());
        let selected_profile_id = selected_profile_id.map(str::to_string);
        let profile_select = self.forms.clone().profile_select;

        profile_select.update(cx, |select, cx| {
            select.set_items(options, window, cx);
            if let Some(selected_profile_id) = selected_profile_id.as_ref() {
                select.set_selected_value(selected_profile_id, window, cx);
            } else {
                select.set_selected_index(None, window, cx);
            }
        });
    }

    fn request_port_forward_rule_removal(
        &self,
        profile_id: &str,
        rule_id: &str,
        cx: &mut Context<Self>,
    ) {
        let Some((profile_index, rule_index)) = self.port_forward_rule_indices(profile_id, rule_id)
        else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.rule_not_found",
            )));
            return;
        };

        let rule_details = {
            let profiles = self.catalog.profiles();
            let profile = &profiles[profile_index];
            profile
                .port_forwarding_rules
                .get(rule_index)
                .map(|rule| (profile.connection_label(), Self::rule_summary_label(rule)))
        };
        let Some((profile_label, rule_label)) = rule_details else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.rule_not_found",
            )));
            return;
        };

        self.set_pending_port_forward_rule_delete(Some(PendingPortForwardRuleDeleteState {
            profile_id: profile_id.to_string(),
            rule_id: rule_id.to_string(),
            profile_label,
            rule_label,
        }));
        cx.notify();
    }

    fn cancel_port_forward_rule_removal(&self, cx: &mut Context<Self>) {
        if let Some(pending) = self.take_pending_port_forward_rule_delete() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::PortForwardRuleDelete(pending),
            ));
        }
    }

    fn read_port_forward_rule_input_values(
        &self,
        cx: &App,
    ) -> anyhow::Result<PortForwardRuleInputValues> {
        let editor_state = self.editor_state();
        let kind = editor_state.port_forward_kind;
        let Some(profile_id) = editor_state.port_forward_editor_profile_id else {
            return Err(ValidationFailure::required(i18n::string(
                "errors.forwarding.validation.host_profile_required",
            ))
            .into());
        };
        let Some(_) = self.catalog.profile(&profile_id) else {
            return Err(ValidationFailure::invalid(i18n::string(
                "errors.forwarding.validation.selected_host_profile_missing",
            ))
            .into());
        };

        let forms = self.forms.clone();
        let label = forms.label_input.read(cx).value().to_string();
        let listen_host = forms.listen_host_input.read(cx).value().trim().to_string();
        let listen_port_text = forms.listen_port_input.read(cx).value().trim().to_string();
        let target_host = forms.target_host_input.read(cx).value().trim().to_string();
        let target_port_text = forms.target_port_input.read(cx).value().trim().to_string();

        if listen_host.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.forwarding.validation.listen_host_required",
            ))
            .into());
        }
        if listen_port_text.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.forwarding.validation.listen_port_required",
            ))
            .into());
        }
        if target_host.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.forwarding.validation.target_host_required",
            ))
            .into());
        }
        if target_port_text.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.forwarding.validation.target_port_required",
            ))
            .into());
        }

        let listen_port: u16 = listen_port_text.trim().parse().map_err(|_| {
            anyhow::anyhow!(ValidationFailure::invalid(i18n::string_args(
                "errors.forwarding.validation.invalid_listen_port",
                &[("port", &listen_port_text)],
            )))
        })?;
        let target_port: u16 = target_port_text.trim().parse().map_err(|_| {
            anyhow::anyhow!(ValidationFailure::invalid(i18n::string_args(
                "errors.forwarding.validation.invalid_target_port",
                &[("port", &target_port_text)],
            )))
        })?;

        let resolved_label = {
            let label = label.trim();
            if label.is_empty() {
                format!(
                    "{} {}:{} -> {}:{}",
                    Self::localized_port_forward_kind_label(kind),
                    listen_host,
                    listen_port,
                    target_host,
                    target_port
                )
            } else {
                label.to_string()
            }
        };

        Ok(PortForwardRuleInputValues {
            profile_id,
            kind,
            resolved_label,
            listen_host,
            listen_port,
            target_host,
            target_port,
        })
    }

    fn report_port_forward_rule_save_error(
        &self,
        error: anyhow::Error,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(validation) = error.downcast_ref::<ValidationFailure>() {
            let message = validation.message.clone();
            cx.emit(AppCommand::Feedback(message.clone()));
            crate::ui::shell::push_app_notification(
                window,
                validation_notification(validation.kind, message),
                cx,
            );
        } else {
            let message = error.to_string();
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "forwarding.messages.save_failed",
                &[("message", &message)],
            )));
        }
        cx.notify();
    }

    fn open_port_forward_panel(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_port_forward_editor_state(true, None, None, PortForwardKind::Local);
        let forms = self.forms.clone();
        set_input_value(&forms.label_input, "", window, cx);
        set_input_value(&forms.listen_host_input, "", window, cx);
        set_input_value(&forms.listen_port_input, "", window, cx);
        set_input_value(&forms.target_host_input, "", window, cx);
        set_input_value(&forms.target_port_input, "", window, cx);
        self.sync_port_forward_profile_select(None, window, cx);
        cx.emit(AppCommand::SidebarEditorStateChanged(Some(
            PageEditorSidebarKind::PortForwarding,
        )));
        let message = if self.catalog.profiles().is_empty() {
            i18n::string("forwarding.messages.create_host_profile_before_adding")
        } else {
            i18n::string("forwarding.messages.choose_host_profile_for_new_rule")
        };
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    fn edit_port_forward_rule(
        &self,
        profile_id: String,
        rule_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((profile_index, rule_index)) =
            self.port_forward_rule_indices(&profile_id, &rule_id)
        else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.rule_not_found",
            )));
            return;
        };

        let profile = self.catalog.profiles()[profile_index].clone();
        let Some(rule) = profile.port_forwarding_rules.get(rule_index).cloned() else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.rule_not_found",
            )));
            return;
        };

        self.set_port_forward_editor_state(
            true,
            Some(profile.id.clone()),
            Some(rule.id.clone()),
            rule.kind,
        );
        let forms = self.forms.clone();
        set_input_value(&forms.label_input, rule.label.clone(), window, cx);
        set_input_value(
            &forms.listen_host_input,
            rule.listen_host.clone(),
            window,
            cx,
        );
        set_input_value(
            &forms.listen_port_input,
            rule.listen_port.to_string(),
            window,
            cx,
        );
        set_input_value(
            &forms.target_host_input,
            rule.target_host.clone(),
            window,
            cx,
        );
        set_input_value(
            &forms.target_port_input,
            rule.target_port.to_string(),
            window,
            cx,
        );
        self.sync_port_forward_profile_select(Some(profile.id.as_str()), window, cx);
        cx.emit(AppCommand::SidebarEditorStateChanged(Some(
            PageEditorSidebarKind::PortForwarding,
        )));
        let rule_label = Self::rule_summary_label(&rule);
        cx.emit(AppCommand::Feedback(i18n::string_args(
            "forwarding.messages.editing_rule",
            &[("rule", &rule_label)],
        )));
        cx.notify();
    }

    fn close_port_forward_rule_editor(&self, cx: &mut Context<Self>) {
        let editor_state = self.editor_state();
        if !editor_state.port_forward_editor_open {
            return;
        }

        let message = if editor_state.port_forward_editor_rule_id.is_some() {
            i18n::string("forwarding.messages.canceled_editing")
        } else {
            i18n::string("forwarding.messages.canceled_new")
        };
        self.clear_port_forward_editor();
        cx.emit(AppCommand::SidebarEditorStateChanged(None));
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    fn select_port_forward_editor_profile(
        &self,
        profile_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self
            .editor_state
            .borrow()
            .port_forward_editor_rule_id
            .is_some()
        {
            return;
        }

        let Some(profile_id) = profile_id else {
            self.set_port_forward_editor_profile_id(None);
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.choose_profile_before_adding",
            )));
            cx.notify();
            return;
        };

        let profile = self
            .catalog
            .profiles()
            .iter()
            .find(|profile| profile.id == profile_id)
            .map(|profile| (profile.id.clone(), profile.name.clone()));
        let Some((profile_id, profile_name)) = profile else {
            self.set_port_forward_editor_profile_id(None);
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.profile_not_found",
            )));
            cx.notify();
            return;
        };

        self.set_port_forward_editor_profile_id(Some(profile_id));
        if self
            .editor_state
            .borrow()
            .port_forward_editor_rule_id
            .is_none()
        {
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "forwarding.messages.creating_rule_for_profile",
                &[("profile", &profile_name)],
            )));
        }
        cx.notify();
    }

    fn set_port_forward_editor_state(
        &self,
        open: bool,
        profile_id: Option<String>,
        rule_id: Option<String>,
        kind: PortForwardKind,
    ) {
        self.pending_save.take();
        self.editor_revision
            .set(self.editor_revision.get().wrapping_add(1).max(1));
        let mut state = self.editor_state.borrow_mut();
        state.port_forward_editor_open = open;
        state.port_forward_editor_profile_id = profile_id;
        state.port_forward_editor_rule_id = rule_id;
        state.port_forward_kind = kind;
    }

    pub(in crate::ui::shell) fn clear_port_forward_editor(&self) {
        self.pending_save.take();
        self.editor_revision
            .set(self.editor_revision.get().wrapping_add(1).max(1));
        let mut state = self.editor_state.borrow_mut();
        state.port_forward_editor_open = false;
        state.port_forward_editor_profile_id = None;
        state.port_forward_editor_rule_id = None;
    }

    fn set_port_forward_editor_profile_id(&self, profile_id: Option<String>) {
        self.pending_save.take();
        self.editor_revision
            .set(self.editor_revision.get().wrapping_add(1).max(1));
        self.editor_state
            .borrow_mut()
            .port_forward_editor_profile_id = profile_id;
    }

    fn set_port_forward_kind(&self, kind: PortForwardKind, cx: &mut Context<Self>) {
        self.pending_save.take();
        self.editor_revision
            .set(self.editor_revision.get().wrapping_add(1).max(1));
        self.editor_state.borrow_mut().port_forward_kind = kind;
        cx.notify();
    }
}

impl ForwardingController {
    fn commit_rule_change(
        &self,
        change: ForwardingChange,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<ForwardingChangeOutcome> {
        let service = ProfileService::new(self.store.clone(), self.secrets.clone());
        let result = std::rc::Rc::new(RefCell::new(None));
        let capture = result.clone();
        self.profile_commit.commit(cx, move |mut profiles| {
            let outcome = apply_forwarding_change(&mut profiles, change)?;
            service.persist_sessions(&profiles)?;
            capture.replace(Some(outcome));
            Ok(profiles)
        })?;
        result
            .take()
            .ok_or_else(|| anyhow::anyhow!("forwarding commit produced no outcome"))
    }

    fn operation_failed(&self, error: anyhow::Error, cx: &mut Context<Self>) {
        cx.emit(AppCommand::Feedback(i18n::string_args(
            "forwarding.messages.save_failed",
            &[("message", &error.to_string())],
        )));
        cx.notify();
    }

    fn commit_draft(&self, draft: ForwardSaveDraft, window: &mut Window, cx: &mut Context<Self>) {
        if draft.editor_revision != self.editor_revision.get() || !self.editor_open() {
            return;
        }
        let is_edit = draft.rule_id.is_some();
        let outcome = match self.commit_rule_change(ForwardingChange::Save(draft.clone()), cx) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.pending_save.replace(Some(draft));
                self.report_port_forward_rule_save_error(error, window, cx);
                return;
            }
        };
        let synced_suffix = Self::synced_sessions_suffix(self.runtime.sync_rules(
            &outcome.profile_id,
            &outcome.rules,
            cx,
        ));
        self.clear_port_forward_editor();
        cx.emit(AppCommand::SidebarEditorStateChanged(None));
        cx.emit(AppCommand::Feedback(i18n::string_args(
            if is_edit {
                "forwarding.messages.saved"
            } else {
                "forwarding.messages.added"
            },
            &[
                ("rule", &outcome.rule.label),
                ("profile", &outcome.profile_name),
                ("synced_suffix", &synced_suffix),
            ],
        )));
        cx.notify();
    }

    pub(in crate::ui::shell) fn save_port_forward_rule(
        &self,
        requires_unlock: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let values = match self.read_port_forward_rule_input_values(cx) {
            Ok(values) => values,
            Err(error) => {
                self.report_port_forward_rule_save_error(error, window, cx);
                return;
            }
        };
        let draft = ForwardSaveDraft {
            editor_revision: self.editor_revision.get(),
            values,
            rule_id: self.editor_state().port_forward_editor_rule_id,
        };
        if requires_unlock {
            self.pending_save.replace(Some(draft));
            cx.emit(AppCommand::vault_unlock(
                DeferredAppCommand::SavePortForwardRule,
            ));
        } else {
            self.commit_draft(draft, window, cx);
        }
    }

    pub(in crate::ui::shell) fn continue_save_port_forward_rule_after_unlock(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(draft) = self.pending_save.take() {
            self.commit_draft(draft, window, cx);
        }
    }

    fn duplicate_port_forward_rule(&self, profile_id: &str, rule_id: &str, cx: &mut Context<Self>) {
        let outcome = match self.commit_rule_change(
            ForwardingChange::Duplicate {
                profile_id: profile_id.into(),
                rule_id: rule_id.into(),
            },
            cx,
        ) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.operation_failed(error, cx);
                return;
            }
        };
        let synced_suffix =
            Self::synced_sessions_suffix(self.runtime.sync_rules(profile_id, &outcome.rules, cx));
        cx.emit(AppCommand::Feedback(i18n::string_args(
            "forwarding.messages.duplicated",
            &[
                ("rule", &outcome.rule.label),
                ("profile", &outcome.profile_name),
                ("synced_suffix", &synced_suffix),
            ],
        )));
        cx.notify();
    }

    fn stop_runtime(
        &self,
        profile_id: &str,
        rule_id: &str,
        cx: &mut Context<Self>,
    ) -> (bool, bool) {
        let stopped = self
            .manager
            .as_ref()
            .is_some_and(|manager| manager.stop(profile_id, rule_id));
        let closed = self.runtime.retire_rule_session(profile_id, rule_id, cx);
        (stopped, closed)
    }

    fn confirm_port_forward_rule_removal(&self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_port_forward_rule_delete() else {
            return;
        };
        let outcome = match self.commit_rule_change(
            ForwardingChange::Delete {
                profile_id: pending.profile_id.clone(),
                rule_id: pending.rule_id.clone(),
            },
            cx,
        ) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.operation_failed(error, cx);
                return;
            }
        };
        let (_, closed) = self.stop_runtime(&pending.profile_id, &pending.rule_id, cx);
        let disconnect_suffix = if closed {
            i18n::string("forwarding.messages.closed_connection_suffix")
        } else {
            String::new()
        };
        let synced_suffix = Self::synced_sessions_suffix(self.runtime.sync_rules(
            &pending.profile_id,
            &outcome.rules,
            cx,
        ));
        let editor = self.editor_state();
        if editor.port_forward_editor_profile_id.as_deref() == Some(&pending.profile_id)
            && editor.port_forward_editor_rule_id.as_deref() == Some(&pending.rule_id)
        {
            self.clear_port_forward_editor();
            cx.emit(AppCommand::SidebarEditorStateChanged(None));
        }
        self.cancel_port_forward_rule_removal(cx);
        cx.emit(AppCommand::Feedback(i18n::string_args(
            "forwarding.messages.removed",
            &[
                ("rule", &Self::rule_summary_label(&outcome.rule)),
                ("profile", &outcome.profile_name),
                ("disconnect_suffix", &disconnect_suffix),
                ("synced_suffix", &synced_suffix),
            ],
        )));
        cx.notify();
    }

    fn connect_port_forward_rule(&self, profile_id: &str, rule_id: &str, cx: &mut Context<Self>) {
        if self.runtime.has_rule_session(profile_id, rule_id, cx) {
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.already_connected",
            )));
            return;
        }
        let Some(profile) = self.catalog.profile(profile_id).filter(|profile| {
            profile
                .port_forwarding_rules
                .iter()
                .any(|rule| rule.id == rule_id)
        }) else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.rule_not_found",
            )));
            return;
        };
        let needs_unlock = self.vault_status == LocalVaultStatus::Locked
            && match profile.effective_auth_method() {
                AuthMethod::Password => profile.password.is_empty() && profile.has_stored_password,
                AuthMethod::KeyFile => {
                    profile.passphrase.is_empty() && profile.has_stored_passphrase
                }
                AuthMethod::ManagedKey => !profile.managed_key_id.trim().is_empty(),
                AuthMethod::Agent | AuthMethod::KeyboardInteractive => false,
            };
        if needs_unlock {
            cx.emit(AppCommand::vault_unlock_prompt());
            return;
        }
        cx.emit(AppCommand::OpenTab(
            super::controllers::TabOpenRequest::PortForwarding {
                profile_id: profile_id.into(),
                rule_id: rule_id.into(),
            },
        ));
    }

    fn set_port_forward_rule_enabled(
        &self,
        profile_id: &str,
        rule_id: &str,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if enabled {
            self.connect_port_forward_rule(profile_id, rule_id, cx);
        } else {
            self.disable_rule(profile_id, rule_id, false, cx);
        }
    }
    fn disconnect_port_forward_rule(
        &self,
        profile_id: &str,
        rule_id: &str,
        cx: &mut Context<Self>,
    ) {
        self.disable_rule(profile_id, rule_id, true, cx);
    }
    fn disable_rule(
        &self,
        profile_id: &str,
        rule_id: &str,
        disconnect: bool,
        cx: &mut Context<Self>,
    ) {
        let outcome = match self.commit_rule_change(
            ForwardingChange::Disable {
                profile_id: profile_id.into(),
                rule_id: rule_id.into(),
            },
            cx,
        ) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.operation_failed(error, cx);
                return;
            }
        };
        let (stopped, closed) = self.stop_runtime(profile_id, rule_id, cx);
        let synced_suffix =
            Self::synced_sessions_suffix(self.runtime.sync_rules(profile_id, &outcome.rules, cx));
        let stopped_suffix = if closed {
            i18n::string("forwarding.messages.stopped_tunnel_suffix")
        } else {
            String::new()
        };
        let rule_label = Self::rule_summary_label(&outcome.rule);
        let message = if disconnect && !stopped && !closed {
            i18n::string("forwarding.messages.not_connected")
        } else if disconnect {
            i18n::string_args(
                "forwarding.messages.disconnected",
                &[("title", &rule_label), ("synced_suffix", &synced_suffix)],
            )
        } else {
            i18n::string_args(
                if closed {
                    "forwarding.messages.disabled"
                } else {
                    "forwarding.messages.disabled_plain"
                },
                &[
                    ("rule", &rule_label),
                    ("profile", &outcome.profile_name),
                    ("stopped_suffix", &stopped_suffix),
                    ("synced_suffix", &synced_suffix),
                ],
            )
        };
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::contracts::{
        CatalogReader, ForwardingRuntime, ProfileCommitOperation, ProfileCommitter,
    };
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{Render, TestAppContext, WindowBounds, WindowOptions, size};
    use std::rc::Rc;

    #[derive(Clone)]
    struct Catalog(Rc<RefCell<Vec<SessionProfile>>>, Rc<Cell<usize>>);
    impl CatalogReader for Catalog {
        fn profiles(&self) -> Vec<SessionProfile> {
            self.0.borrow().clone()
        }
        fn profile(&self, id: &str) -> Option<SessionProfile> {
            self.0
                .borrow()
                .iter()
                .find(|profile| profile.id == id)
                .cloned()
        }
        fn proxies(&self) -> Vec<miaominal_core::proxy::ProxyProfile> {
            vec![]
        }
    }
    impl ProfileCommitter for Catalog {
        fn commit(
            &self,
            _: &mut App,
            operation: ProfileCommitOperation,
        ) -> anyhow::Result<Vec<SessionProfile>> {
            let profiles = operation(self.profiles())?;
            if profiles != *self.0.borrow() {
                self.0.replace(profiles.clone());
                self.1.set(self.1.get() + 1);
            }
            Ok(profiles)
        }
    }
    #[derive(Default)]
    struct Runtime {
        synced: RefCell<Vec<(String, Vec<PortForwardRule>)>>,
        retired: RefCell<Vec<(String, String)>>,
    }
    impl ForwardingRuntime for Runtime {
        fn has_rule_session(&self, _: &str, _: &str, _: &App) -> bool {
            false
        }
        fn online_session_count_for_profile(&self, _: &str, _: &App) -> usize {
            0
        }
        fn sync_rules(&self, id: &str, rules: &[PortForwardRule], _: &mut App) -> usize {
            self.synced.borrow_mut().push((id.into(), rules.to_vec()));
            0
        }
        fn retire_rule_session(&self, profile: &str, rule: &str, _: &mut App) -> bool {
            self.retired
                .borrow_mut()
                .push((profile.into(), rule.into()));
            false
        }
    }
    struct Harness {
        controller: Entity<ForwardingController>,
    }
    impl Render for Harness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let save_controller = self.controller.clone();
            div()
                .size_full()
                .child(self.controller.read(cx).render_port_forward_editor_sidebar(
                    self.controller.clone(),
                    move |window, cx| {
                        save_controller.update(cx, |controller, cx| {
                            controller.save_port_forward_rule(false, window, cx)
                        })
                    },
                    cx,
                ))
        }
    }
    fn profile(id: &str) -> SessionProfile {
        let mut profile = SessionProfile::blank(id, 1);
        profile.name = id.into();
        profile.host = "example.test".into();
        profile.port_forwarding_rules.push(PortForwardRule {
            id: "rule-a".into(),
            label: "original".into(),
            kind: PortForwardKind::Local,
            listen_host: "127.0.0.1".into(),
            listen_port: 8000,
            target_host: "127.0.0.1".into(),
            target_port: 80,
            enabled: true,
        });
        profile
    }

    #[gpui_kit::test]
    fn failed_writes_preserve_editor_and_running_rules(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let catalog = Catalog(
            Rc::new(RefCell::new(vec![profile("alpha"), profile("beta")])),
            Rc::new(Cell::new(0)),
        );
        let runtime = Rc::new(Runtime::default());
        let (view, cx) = cx.add_window_view(|window, cx| {
            let controller = cx.new(|cx| {
                ForwardingController::new(
                    ForwardingControllerArgs {
                        catalog: CatalogQueryPort::new(Rc::new(catalog.clone())),
                        profile_commit: ProfileCommitPort::new(Rc::new(catalog.clone())),
                        store: Some(SessionStore::with_path(std::env::temp_dir())),
                        secrets: SecretStore::new_locked_vault(),
                        vault_status: LocalVaultStatus::Disabled,
                        manager: None,
                        runtime: ForwardingRuntimePort::new(runtime.clone()),
                    },
                    window,
                    cx,
                )
            });
            Harness { controller }
        });
        let controller = view.read_with(cx, |view, _| view.controller.clone());
        let original = catalog.profiles();
        cx.update(|window, cx| {
            controller.update(cx, |controller, cx| {
                controller.edit_port_forward_rule("alpha".into(), "rule-a".into(), window, cx);
                set_input_value(&controller.forms.label_input, "submitted", window, cx);
                controller.save_port_forward_rule(false, window, cx);
                assert!(controller.editor_open());
                assert_eq!(controller.forms.label_input.read(cx).value(), "submitted");
                controller.duplicate_port_forward_rule("alpha", "rule-a", cx);
                controller.disconnect_port_forward_rule("alpha", "rule-a", cx);
                controller.set_port_forward_rule_enabled("alpha", "rule-a", false, cx);
                controller.request_port_forward_rule_removal("alpha", "rule-a", cx);
                controller.confirm_port_forward_rule_removal(cx);
                assert!(controller.pending_port_forward_rule_delete().is_some());
                assert_eq!(catalog.profiles(), original);
                assert!(runtime.synced.borrow().is_empty());
                assert!(runtime.retired.borrow().is_empty());
                controller.cancel_port_forward_rule_removal(cx);
                controller.save_port_forward_rule(true, window, cx);
                set_input_value(
                    &controller.forms.label_input,
                    "changed after submission",
                    window,
                    cx,
                );
            })
        });
        assert_eq!(catalog.1.get(), 0);
        // Another window can change/reorder hosts while the unlock dialog is open.
        let mut reordered = original.clone();
        reordered.reverse();
        reordered[0].name = "changed in other window".into();
        reordered[1].port_forwarding_rules[0].enabled = false;
        reordered[1].username = "changed by Hosts".into();
        reordered[1].is_favorite = true;
        reordered[1].remote_path_favorites = vec!["/changed by SFTP".into()];
        let mut other_rule = reordered[1].port_forwarding_rules[0].clone();
        other_rule.id = "new rule from other window".into();
        reordered[1].port_forwarding_rules.push(other_rule);
        catalog.0.replace(reordered);
        let path =
            std::env::temp_dir().join(format!("miaominal-forward-{}.json", uuid::Uuid::new_v4()));
        cx.update(|window, cx| {
            controller.update(cx, |controller, cx| {
                controller.store = Some(SessionStore::with_path(path.clone()));
                controller.adopt_catalog(window, cx);
                assert!(controller.pending_save.borrow().is_some());
                assert_eq!(
                    controller.forms.label_input.read(cx).value(),
                    "changed after submission"
                );
            })
        });
        assert_eq!(catalog.1.get(), 0);
        cx.update(|window, cx| {
            controller.update(cx, |controller, cx| {
                controller.continue_save_port_forward_rule_after_unlock(window, cx);
                assert!(!controller.editor_open());
            })
        });
        assert_eq!(catalog.1.get(), 1);
        let saved = catalog.profiles();
        assert_eq!(saved[0].name, "changed in other window");
        assert_eq!(saved[1].port_forwarding_rules[0].label, "submitted");
        assert!(!saved[1].port_forwarding_rules[0].enabled);
        assert_eq!(saved[1].username, "changed by Hosts");
        assert!(saved[1].is_favorite);
        assert_eq!(saved[1].remote_path_favorites, vec!["/changed by SFTP"]);
        assert_eq!(
            saved[1].port_forwarding_rules[1].id,
            "new rule from other window"
        );
        assert_eq!(runtime.synced.borrow().len(), 1);
        assert_eq!(runtime.synced.borrow()[0].0, "alpha");
        assert!(runtime.retired.borrow().is_empty());
        // A removed target must not be recreated, and changing editors cancels a deferred save.
        cx.update(|window, cx| {
            controller.update(cx, |controller, cx| {
                controller.edit_port_forward_rule("alpha".into(), "rule-a".into(), window, cx);
                controller.save_port_forward_rule(true, window, cx);
                catalog.0.borrow_mut()[1].port_forwarding_rules.clear();
                controller.adopt_catalog(window, cx);
                controller.continue_save_port_forward_rule_after_unlock(window, cx);
                assert!(controller.editor_open());
                assert!(catalog.profiles()[1].port_forwarding_rules.is_empty());
                assert_eq!(
                    controller
                        .editor_state()
                        .port_forward_editor_rule_id
                        .as_deref(),
                    Some("rule-a")
                );
                controller.edit_port_forward_rule("beta".into(), "rule-a".into(), window, cx);
                controller.save_port_forward_rule(true, window, cx);
                controller.open_port_forward_panel(window, cx);
                controller.continue_save_port_forward_rule_after_unlock(window, cx);
                assert_eq!(
                    catalog.profiles()[0].port_forwarding_rules[0].label,
                    "original"
                );
                controller.adopt_catalog(window, cx);
                cx.notify();
            })
        });
        assert_eq!(catalog.1.get(), 1);
        // Deletion applies only to the captured profile/rule IDs after reordering.
        cx.update(|window, cx| {
            controller.update(cx, |controller, cx| {
                controller.edit_port_forward_rule("beta".into(), "rule-a".into(), window, cx);
                controller.request_port_forward_rule_removal("beta", "rule-a", cx);
                catalog.0.borrow_mut().reverse();
                controller.confirm_port_forward_rule_removal(cx);
                assert!(!controller.editor_open());
                assert!(controller.pending_port_forward_rule_delete().is_none());
            })
        });
        assert_eq!(catalog.1.get(), 2);
        assert!(catalog.profiles()[1].port_forwarding_rules.is_empty());
        assert_eq!(
            *runtime.retired.borrow(),
            vec![("beta".into(), "rule-a".into())]
        );
        assert_eq!(runtime.synced.borrow().len(), 2);
        std::fs::remove_file(path).unwrap();
    }

    #[gpui_kit::test]
    fn forwarding_editor_validates_and_commits_from_real_keyboard_and_pointer_events(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let catalog = Catalog(
            Rc::new(RefCell::new(vec![profile("alpha"), profile("beta")])),
            Rc::new(Cell::new(0)),
        );
        let runtime = Rc::new(Runtime::default());
        let path = std::env::temp_dir().join(format!(
            "miaominal-forward-ui-{}.json",
            uuid::Uuid::new_v4()
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
                            ForwardingController::new(
                                ForwardingControllerArgs {
                                    catalog: CatalogQueryPort::new(Rc::new(catalog.clone())),
                                    profile_commit: ProfileCommitPort::new(Rc::new(
                                        catalog.clone(),
                                    )),
                                    store: Some(SessionStore::with_path(path.clone())),
                                    secrets: SecretStore::new_locked_vault(),
                                    vault_status: LocalVaultStatus::Disabled,
                                    manager: None,
                                    runtime: ForwardingRuntimePort::new(runtime.clone()),
                                },
                                window,
                                cx,
                            )
                        });
                        controller.update(cx, |controller, cx| {
                            controller.edit_port_forward_rule(
                                "alpha".into(),
                                "rule-a".into(),
                                window,
                                cx,
                            )
                        });
                        Harness { controller }
                    })
                },
            )
            .expect("open forwarding editor test window")
        });
        let controller = cx.update(|cx| harness.read(cx).controller.clone());
        let (label_id, target_port_id) = cx.update(|cx| {
            let forms = &controller.read(cx).forms;
            (
                ("input", forms.label_input.entity_id()),
                ("input", forms.target_port_input.entity_id()),
            )
        });
        let save_id = SharedString::from(format!(
            "editor-button-{}",
            i18n::string("forwarding.actions.save_rule")
        ));
        cx.update_window(window_handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(target_port_id, cx);
            window.press("ctrl-a", cx);
            window.input("bad-port", cx);
            window.click(save_id.clone(), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(catalog.1.get(), 0);
        assert!(runtime.synced.borrow().is_empty());
        assert!(cx.update(|cx| controller.read(cx).editor_open()));
        cx.update_window(window_handle, |_, window, cx| {
            window.click(target_port_id, cx);
            window.press("ctrl-a", cx);
            window.input("5432", cx);
            window.click(label_id, cx);
            window.press("ctrl-a", cx);
            window.input("数据库隧道x", cx);
            window.press("backspace", cx);
            assert_eq!(window.find(label_id).value(), Some("数据库隧道"));
            assert_eq!(window.find(label_id).focused(), Some(true));
            window.click(save_id.clone(), cx);
        })
        .unwrap();
        cx.run_until_parked();
        let profiles = catalog.profiles();
        assert_eq!(catalog.1.get(), 1);
        assert_eq!(profiles[0].port_forwarding_rules[0].label, "数据库隧道");
        assert_eq!(profiles[0].port_forwarding_rules[0].target_port, 5432);
        assert_eq!(profiles[1], profile("beta"));
        assert_eq!(runtime.synced.borrow().len(), 1);
        assert!(runtime.retired.borrow().is_empty());
        assert!(!cx.update(|cx| controller.read(cx).editor_open()));
        std::fs::remove_file(path).unwrap();
    }

    #[gpui_kit::test]
    fn forwarding_replaced_editor_rejects_stale_save_completion(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let catalog = Catalog(
            Rc::new(RefCell::new(vec![profile("alpha"), profile("beta")])),
            Rc::new(Cell::new(0)),
        );
        let runtime = Rc::new(Runtime::default());
        let (view, cx) = cx.add_window_view(|window, cx| Harness {
            controller: cx.new(|cx| {
                ForwardingController::new(
                    ForwardingControllerArgs {
                        catalog: CatalogQueryPort::new(Rc::new(catalog.clone())),
                        profile_commit: ProfileCommitPort::new(Rc::new(catalog.clone())),
                        store: Some(SessionStore::with_path(std::env::temp_dir())),
                        secrets: SecretStore::new_locked_vault(),
                        vault_status: LocalVaultStatus::Disabled,
                        manager: None,
                        runtime: ForwardingRuntimePort::new(runtime.clone()),
                    },
                    window,
                    cx,
                )
            }),
        });
        let controller = view.read_with(cx, |view, _| view.controller.clone());
        cx.update(|window, cx| {
            controller.update(cx, |controller, cx| {
                controller.edit_port_forward_rule("alpha".into(), "rule-a".into(), window, cx);
                let stale = ForwardSaveDraft {
                    editor_revision: controller.editor_revision.get(),
                    values: controller.read_port_forward_rule_input_values(cx).unwrap(),
                    rule_id: Some("rule-a".into()),
                };
                controller.save_port_forward_rule(true, window, cx);
                controller.edit_port_forward_rule("beta".into(), "rule-a".into(), window, cx);
                assert!(controller.pending_save.borrow().is_none());
                controller.commit_draft(stale, window, cx);
                assert!(controller.editor_open());
                assert_eq!(
                    controller
                        .editor_state()
                        .port_forward_editor_profile_id
                        .as_deref(),
                    Some("beta")
                );
                assert!(controller.pending_save.borrow().is_none());
                controller.continue_save_port_forward_rule_after_unlock(window, cx);
            })
        });
        assert_eq!(catalog.1.get(), 0);
        assert!(runtime.synced.borrow().is_empty());
    }
}
