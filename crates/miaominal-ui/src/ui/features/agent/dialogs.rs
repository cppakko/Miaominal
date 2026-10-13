use super::*;

impl AgentController {
    pub(super) fn render_chat_session_delete_prompt(
        &self,
        entity: OverlayTarget<Self>,
        prompt: &PendingChatSessionDeleteState,
        exit_progress: Option<f32>,
    ) -> gpui_kit::AnyElement {
        let title = if prompt.title.trim().is_empty() {
            i18n::string("workspace.panel.agent.history.untitled_chat")
        } else {
            prompt.title.clone()
        };
        let subtitle = i18n::string_args("dialogs.chat_delete.message", &[("title", &title)]);

        let controller_cancel = entity.clone();
        let controller_confirm = entity.clone();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "chat-session-delete-cancel",
                    i18n::string("dialogs.chat_delete.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_chat_session_delete(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "chat-session-delete-confirm",
                    i18n::string("dialogs.chat_delete.confirm"),
                    BasicDialogActionTone::Destructive,
                )
                .on_click(move |_, _, cx| {
                    controller_confirm.update(cx, |controller, cx| {
                        controller.confirm_chat_session_delete(cx);
                    });
                }),
            );

        render_basic_dialog(
            "chat-session-delete",
            i18n::string("dialogs.chat_delete.title"),
            Some(subtitle),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }

    pub(super) fn render_chat_session_rename_prompt(
        &self,
        entity: OverlayTarget<Self>,
        retained_input: Entity<InputState>,
        prompt: &PendingChatSessionRenameState,
        exit_progress: Option<f32>,
        _cx: &App,
    ) -> gpui_kit::AnyElement {
        let title_input = retained_input;
        let current_title = if prompt.current_title.trim().is_empty() {
            i18n::string("workspace.panel.agent.history.untitled_chat")
        } else {
            prompt.current_title.clone()
        };
        let subtitle =
            i18n::string_args("dialogs.chat_rename.message", &[("title", &current_title)]);

        let controller_cancel = entity.clone();
        let controller_confirm = entity.clone();

        let body = v_flex()
            .w_full()
            .child(surface_text_input_stack(
                i18n::string("dialogs.chat_rename.title_label"),
                title_input.clone(),
                TextInputSurface::Highest,
                false,
            ))
            .into_any_element();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "chat-session-rename-cancel",
                    i18n::string("dialogs.common.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_chat_session_rename(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "chat-session-rename-confirm",
                    i18n::string("dialogs.chat_rename.confirm"),
                    BasicDialogActionTone::Default,
                )
                .on_click({
                    let controller = controller_confirm.clone();
                    let title_input = title_input.clone();
                    move |_, _, cx| {
                        controller.update(cx, |controller, cx| {
                            let new_title = title_input.read(cx).value().to_string();
                            controller.confirm_chat_session_rename(new_title, cx);
                        });
                    }
                }),
            );

        render_basic_dialog(
            "chat-session-rename",
            i18n::string("dialogs.chat_rename.title"),
            Some(subtitle),
            Some(body),
            actions.into_any_element(),
            exit_progress,
        )
    }
}
