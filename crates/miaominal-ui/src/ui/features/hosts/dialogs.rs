use super::import_labels::{
    localized_profile_import_issue_kind, localized_profile_import_issue_reason,
    localized_profile_import_source_label,
};
use crate::ui::components::animation::render_basic_dialog;
use crate::ui::components::{BasicDialogActionTone, basic_dialog_action_button};
use crate::ui::contracts::commands::AppCommand;
use crate::ui::features::hosts::{
    HostsController, PendingProfileDeleteState, PendingProfileImportResultState,
};
use crate::ui::i18n;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, Entity, SharedString, Window, div, px, rgb};

impl HostsController {
    pub(in crate::ui::features::hosts) fn render_profile_delete_prompt(
        controller: Entity<Self>,
        prompt: &PendingProfileDeleteState,
        revision: u64,
        exit_progress: Option<f32>,
    ) -> gpui_kit::AnyElement {
        let profile_name = if prompt.profile_name.trim().is_empty() {
            i18n::string("dialogs.profile_delete.untitled_profile")
        } else {
            prompt.profile_name.clone()
        };
        let subtitle = i18n::string_args(
            "dialogs.profile_delete.message",
            &[("profile_name", profile_name.as_str())],
        );

        let controller_cancel = controller.clone();
        let entity_confirm = controller.clone();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "profile-delete-cancel",
                    i18n::string("dialogs.profile_delete.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    controller_cancel.update(cx, |controller, cx| {
                        if controller
                            .overlay_revisions
                            .borrow()
                            .current("profile-delete")
                            == revision
                        {
                            controller.cancel_profile_delete(cx);
                        }
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "profile-delete-confirm",
                    i18n::string("dialogs.profile_delete.confirm"),
                    BasicDialogActionTone::Destructive,
                )
                .on_click(move |_, window, cx| {
                    entity_confirm.update(cx, |this, cx| {
                        if this.overlay_revisions.borrow().current("profile-delete") == revision {
                            this.confirm_profile_delete(window, cx);
                        }
                    });
                }),
            );

        render_basic_dialog(
            "profile-delete",
            i18n::string("dialogs.profile_delete.title"),
            Some(subtitle),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }

    pub(in crate::ui::features::hosts) fn render_profile_import_result(
        controller: Entity<Self>,
        prompt: &PendingProfileImportResultState,
        revision: u64,
        exit_progress: Option<f32>,
    ) -> gpui_kit::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let source = localized_profile_import_source_label(prompt.source);
        let imported_count = prompt.imported_count.to_string();
        let issue_count = prompt.issues.len().to_string();
        let title = i18n::string(if prompt.imported_count == 0 {
            "settings.connections.import_result.failed_title"
        } else {
            "settings.connections.import_result.partial_title"
        });
        let supporting_text = i18n::string_args(
            if prompt.imported_count == 0 {
                "settings.connections.import_result.failed_summary"
            } else {
                "settings.connections.import_result.partial_summary"
            },
            &[
                ("source", &source),
                ("imported", &imported_count),
                ("issues", &issue_count),
            ],
        );

        let issue_rows = prompt
            .issues
            .iter()
            .enumerate()
            .map(|(index, issue)| {
                let entry_name = if issue.entry_name.trim().is_empty() {
                    i18n::string("settings.connections.import_result.unknown_entry")
                } else {
                    issue.entry_name.clone()
                };
                let kind = localized_profile_import_issue_kind(issue.kind);
                let reason = localized_profile_import_issue_reason(&issue.reason);
                v_flex()
                    .id(SharedString::from(format!("profile-import-issue-{index}")))
                    .w_full()
                    .min_w(px(0.0))
                    .gap_1()
                    .p_3()
                    .rounded(px(14.0))
                    .bg(rgb(roles.surface_container_low))
                    .child(
                        h_flex()
                            .w_full()
                            .min_w(px(0.0))
                            .justify_between()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_sm()
                                    .text_color(rgb(roles.on_surface))
                                    .child(entry_name),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(rgb(roles.tertiary))
                                    .child(kind),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w(px(0.0))
                            .text_sm()
                            .line_height(miaominal_settings::scaled_line_height(18.0))
                            .text_color(rgb(roles.on_surface_variant))
                            .child(reason),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let body = div()
            .w_full()
            .max_h(px(360.0))
            .overflow_y_scrollbar()
            .child(v_flex().w_full().gap_2().children(issue_rows))
            .into_any_element();

        let actions = h_flex().justify_end().child(
            basic_dialog_action_button(
                "profile-import-result-close",
                i18n::string("settings.connections.import_result.close"),
                BasicDialogActionTone::Default,
            )
            .on_click(move |_, _, cx| {
                controller.update(cx, |controller, cx| {
                    if controller
                        .overlay_revisions
                        .borrow()
                        .current("profile-import-result")
                        == revision
                    {
                        controller.dismiss_profile_import_result(cx);
                    }
                });
            }),
        );

        render_basic_dialog(
            "profile-import-result",
            title,
            Some(supporting_text),
            Some(body),
            actions.into_any_element(),
            exit_progress,
        )
    }

    pub(in crate::ui::features::hosts) fn confirm_profile_delete(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.pending_profile_delete() else {
            return;
        };
        let snapshot = self.profile_delete_overlay(cx.entity());
        let options = self.managed_key_options.borrow().clone();
        if self.delete_profile_by_id(
            &pending.profile_id,
            &pending.profile_name,
            pending.reload_inputs_after_delete,
            options,
            window,
            cx,
        ) {
            self.take_pending_profile_delete();
            if let Some(snapshot) = snapshot {
                cx.emit(AppCommand::OverlayDismissed(snapshot));
            }
        }
    }
}
