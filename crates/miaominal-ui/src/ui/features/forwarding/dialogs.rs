use super::*;

impl ForwardingController {
    pub(in crate::ui::shell) fn render_port_forward_rule_delete_prompt(
        &self,
        controller: Entity<Self>,
        prompt: &PendingPortForwardRuleDeleteState,
        exit_progress: Option<f32>,
    ) -> gpui_kit::AnyElement {
        let subtitle = i18n::string_args(
            "dialogs.forwarding_rule_delete.message",
            &[
                ("rule", prompt.rule_label.as_str()),
                ("profile", prompt.profile_label.as_str()),
            ],
        );

        let controller_cancel = controller.clone();
        let controller_confirm = controller.clone();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "port-forward-rule-delete-cancel",
                    i18n::string("dialogs.forwarding_rule_delete.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_port_forward_rule_removal(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "port-forward-rule-delete-confirm",
                    i18n::string("dialogs.forwarding_rule_delete.confirm"),
                    BasicDialogActionTone::Destructive,
                )
                .on_click(move |_, _, cx| {
                    controller_confirm.update(cx, |controller, cx| {
                        controller.confirm_port_forward_rule_removal(cx);
                    });
                }),
            );

        render_basic_dialog(
            "port-forward-rule-delete",
            i18n::string("dialogs.forwarding_rule_delete.title"),
            Some(subtitle),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }
}
