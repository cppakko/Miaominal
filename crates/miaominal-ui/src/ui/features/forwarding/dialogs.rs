use crate::ui::components::animation::render_basic_dialog;
use crate::ui::components::{BasicDialogActionTone, basic_dialog_action_button};
use crate::ui::features::forwarding::{ForwardingController, PendingPortForwardRuleDeleteState};
use crate::ui::i18n;
use gpui_kit::Entity;
use gpui_kit::component::h_flex;
use gpui_kit::prelude::*;

impl ForwardingController {
    pub(in crate::ui::features::forwarding) fn render_port_forward_rule_delete_prompt(
        controller: Entity<Self>,
        prompt: &PendingPortForwardRuleDeleteState,
        revision: u64,
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
                        if controller
                            .overlay_revisions
                            .borrow()
                            .current("forwarding-delete")
                            == revision
                        {
                            controller.cancel_port_forward_rule_removal(cx);
                        }
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
                        if controller
                            .overlay_revisions
                            .borrow()
                            .current("forwarding-delete")
                            == revision
                        {
                            controller.confirm_port_forward_rule_removal(cx);
                        }
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
