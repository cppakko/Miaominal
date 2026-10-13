use crate::ui::components::animation::render_basic_dialog;
use crate::ui::components::{BasicDialogActionTone, basic_dialog_action_button};
use crate::ui::features::snippets::{PendingSnippetDeleteState, SnippetsController};
use crate::ui::i18n;
use gpui_kit::Entity;
use gpui_kit::component::h_flex;
use gpui_kit::prelude::*;

impl SnippetsController {
    pub(in crate::ui::features::snippets) fn render_snippet_delete_prompt(
        controller: Entity<Self>,
        prompt: &PendingSnippetDeleteState,
        revision: u64,
        exit_progress: Option<f32>,
    ) -> gpui_kit::AnyElement {
        let description = if prompt.snippet_description.trim().is_empty() {
            i18n::string("dialogs.snippet_delete.untitled_snippet")
        } else {
            prompt.snippet_description.clone()
        };
        let subtitle = i18n::string_args(
            "dialogs.snippet_delete.message",
            &[("description", description.as_str())],
        );

        let controller_cancel = controller.clone();
        let controller_confirm = controller.clone();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "snippet-delete-cancel",
                    i18n::string("dialogs.snippet_delete.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    controller_cancel.update(cx, |controller, cx| {
                        if controller
                            .overlay_revisions
                            .borrow()
                            .current("snippet-delete")
                            == revision
                        {
                            controller.cancel_snippet_delete(cx);
                        }
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "snippet-delete-confirm",
                    i18n::string("dialogs.snippet_delete.confirm"),
                    BasicDialogActionTone::Destructive,
                )
                .on_click(move |_, _, cx| {
                    controller_confirm.update(cx, |controller, cx| {
                        if controller
                            .overlay_revisions
                            .borrow()
                            .current("snippet-delete")
                            == revision
                        {
                            controller.confirm_snippet_delete(cx);
                        }
                    });
                }),
            );

        render_basic_dialog(
            "snippet-delete",
            i18n::string("dialogs.snippet_delete.title"),
            Some(subtitle),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }
}
