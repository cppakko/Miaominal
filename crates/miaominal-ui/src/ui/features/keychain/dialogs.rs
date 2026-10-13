use super::*;

impl KeychainController {
    pub(super) fn render_managed_key_delete_prompt(
        &self,
        entity: OverlayTarget<Self>,
        prompt: &PendingManagedKeyDeleteState,
        exit_progress: Option<f32>,
    ) -> gpui_kit::AnyElement {
        let key_name = if prompt.key_name.trim().is_empty() {
            i18n::string("dialogs.managed_key_delete.untitled_key")
        } else {
            prompt.key_name.clone()
        };
        let subtitle = i18n::string_args(
            "dialogs.managed_key_delete.message",
            &[("key_name", key_name.as_str())],
        );

        let controller_cancel = entity.clone();
        let controller_confirm = entity.clone();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "managed-key-delete-cancel",
                    i18n::string("dialogs.managed_key_delete.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_managed_key_delete(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "managed-key-delete-confirm",
                    i18n::string("dialogs.managed_key_delete.confirm"),
                    BasicDialogActionTone::Destructive,
                )
                .on_click(move |_, window, cx| {
                    controller_confirm.update(cx, |controller, cx| {
                        controller.confirm_managed_key_delete(window, cx);
                    });
                }),
            );

        render_basic_dialog(
            "managed-key-delete",
            i18n::string("dialogs.managed_key_delete.title"),
            Some(subtitle),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }

    pub(super) fn render_managed_key_rename_prompt(
        &self,
        entity: OverlayTarget<Self>,
        retained_input: Entity<InputState>,
        prompt: &PendingManagedKeyRenameState,
        exit_progress: Option<f32>,
        _cx: &App,
    ) -> gpui_kit::AnyElement {
        let rename_input = retained_input;
        let subtitle = i18n::string_args(
            "dialogs.managed_key_rename.message",
            &[("key_name", &prompt.current_name)],
        );

        let controller_cancel = entity.clone();
        let controller_confirm = entity.clone();
        let body = v_flex()
            .w_full()
            .child(surface_text_input_stack(
                i18n::string("dialogs.managed_key_rename.name_label"),
                rename_input,
                TextInputSurface::Highest,
                true,
            ))
            .into_any_element();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "managed-key-rename-cancel",
                    i18n::string("dialogs.common.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_managed_key_rename(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "managed-key-rename-confirm",
                    i18n::string("dialogs.managed_key_rename.confirm"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, window, cx| {
                    controller_confirm.update(cx, |controller, cx| {
                        controller.confirm_managed_key_rename(window, cx);
                    });
                }),
            );

        render_basic_dialog(
            "managed-key-rename",
            i18n::string("dialogs.managed_key_rename.title"),
            Some(subtitle),
            Some(body),
            actions.into_any_element(),
            exit_progress,
        )
    }
}
