use super::super::*;
use super::session_agent_utils::*;
use crate::ui::components::icon_button_with_tooltip;
use crate::ui::i18n;
use gpui::{Animation, AnimationExt as _, ClipboardEntry};
use std::time::Duration;

const SESSION_AGENT_SEND_PULSE_DURATION: Duration = Duration::from_millis(1100);

pub(in crate::ui::shell::layout) fn render_session_agent_composer(
    app: &AppView,
    entity: Entity<AppView>,
) -> gpui::AnyElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );
    let provider_select = app.panel_forms.settings.ai_provider_select.clone();
    let prompt_input = app.workspace_forms.agent.prompt_input.clone();
    let prompt_menu_input = prompt_input.clone();
    let pty_toggle_entity = entity.clone();
    let attach_entity = entity.clone();
    let paste_entity = entity.clone();
    let send_entity = entity;
    let waiting = app.session_agent.is_busy();
    let has_attachments = !app.session_agent.pending_attachments.is_empty();

    div()
        .flex_shrink_0()
        .p_2()
        .relative()
        .on_drop::<ExternalPaths>({
            let drop_entity = send_entity.clone();
            move |paths: &ExternalPaths, _window, cx| {
                let local_paths: Vec<std::path::PathBuf> = paths.paths().to_vec();
                drop_entity.update(cx, |this, cx| {
                    this.ingest_attachment_paths(local_paths, cx);
                });
            }
        })
        .child(
            v_flex()
                .w_full()
                .gap_2()
                .rounded(px(8.0))
                .bg(rgb(roles.surface_container_high))
                .p_2()
                .child(app.render_session_agent_target_chips(pty_toggle_entity.clone()))
                .when(has_attachments, |this| {
                    this.child(render_attachment_preview_row(
                        &app.session_agent.pending_attachments,
                        roles,
                        send_entity.clone(),
                    ))
                })
                .child(
                    div()
                        .w_full()
                        .min_h(px(86.0))
                        .max_h(px(190.0))
                        .rounded(px(6.0))
                        .relative()
                        .overflow_hidden()
                        .id("session-agent-prompt-input-menu")
                        .on_key_down({
                            let entity = paste_entity.clone();
                            move |event: &KeyDownEvent, _window, cx| {
                                handle_paste_key(event, entity.clone(), cx);
                            }
                        })
                        .child(
                            Input::new(&prompt_input)
                                .w_full()
                                .appearance(false)
                                .focus_bordered(false)
                                .p_1(),
                        )
                        .context_menu(move |menu, _window, cx| {
                            let state = prompt_menu_input.read(cx);
                            let has_selection = !state.selected_range().is_empty();
                            let has_text = !state.value().is_empty();
                            let focus = state.focus_handle(cx);
                            menu.action_context(focus)
                                .menu_with_disabled(
                                    i18n::string("workspace.menu.cut"),
                                    Box::new(gpui_component::input::Cut),
                                    !has_selection,
                                )
                                .menu_with_disabled(
                                    i18n::string("workspace.menu.copy"),
                                    Box::new(gpui_component::input::Copy),
                                    !has_selection,
                                )
                                .menu_with_disabled(
                                    i18n::string("workspace.menu.paste"),
                                    Box::new(gpui_component::input::Paste),
                                    cx.read_from_clipboard().is_none(),
                                )
                                .item(PopupMenuItem::separator())
                                .menu_with_disabled(
                                    i18n::string("workspace.menu.select_all"),
                                    Box::new(gpui_component::input::SelectAll),
                                    !has_text,
                                )
                        }),
                )
                .child(
                    h_flex()
                        .w_full()
                        .h(px(28.0))
                        .items_center()
                        .gap_2()
                        .child(
                            div().w(px(112.0)).min_w(px(0.0)).child(
                                md3_select(&provider_select)
                                    .small()
                                    .w_full()
                                    .bg(rgb(roles.surface_container_high)),
                            ),
                        )
                        .child(icon_button_with_tooltip(
                            AppIcon::Paperclip,
                            i18n::string("workspace.panel.agent.tooltips.attach_file"),
                            24.0,
                            8.0,
                            Some(roles.surface_container_high),
                            Some(text_muted),
                            None,
                            move |window, cx| {
                                let entity = attach_entity.clone();
                                entity.update(cx, |this, cx| {
                                    this.open_attachment_picker(window, cx);
                                });
                            },
                        ))
                        .child(icon_button_with_tooltip(
                            AppIcon::LaptopMinimal,
                            i18n::string(if app.session_agent.exec_mode.is_pty() {
                                "workspace.panel.agent.tooltips.disable_pty"
                            } else {
                                "workspace.panel.agent.tooltips.enable_pty"
                            }),
                            24.0,
                            8.0,
                            Some(if app.session_agent.exec_mode.is_pty() {
                                roles.secondary_container
                            } else {
                                roles.surface_container_high
                            }),
                            Some(if app.session_agent.exec_mode.is_pty() {
                                roles.on_secondary_container
                            } else {
                                text_muted
                            }),
                            None,
                            move |_window, cx| {
                                let entity = pty_toggle_entity.clone();
                                entity.update(cx, |this, cx| {
                                    this.session_agent.exec_mode =
                                        this.session_agent.exec_mode.toggle();
                                    cx.notify();
                                });
                            },
                        ))
                        .child(
                            div().w(px(80.0)).child(
                                md3_select(&app.workspace_forms.agent.agent_mode_select)
                                    .small()
                                    .w_full(),
                            ),
                        )
                        .child(div().flex_1())
                        .child(render_session_agent_token_usage(
                            &app.session_agent,
                            &app.settings_store,
                            text_muted,
                        ))
                        .child(div().min_w(px(4.0)))
                        .child(
                            div()
                                .id("session-agent-send-action")
                                .child(icon_button_with_tooltip(
                                    if waiting {
                                        AppIcon::Pause
                                    } else {
                                        AppIcon::ChevronUp
                                    },
                                    i18n::string(if waiting {
                                        "workspace.panel.agent.tooltips.stop_response"
                                    } else {
                                        "workspace.panel.agent.tooltips.send_message"
                                    }),
                                    26.0,
                                    8.0,
                                    Some(if waiting {
                                        roles.error_container
                                    } else {
                                        roles.primary
                                    }),
                                    Some(if waiting {
                                        roles.on_error_container
                                    } else {
                                        roles.on_primary
                                    }),
                                    None,
                                    move |window, cx| {
                                        let entity = send_entity.clone();
                                        entity.update(cx, |this, cx| {
                                            if this.session_agent.is_busy() {
                                                this.stop_session_agent_stream(cx);
                                            } else {
                                                this.submit_session_agent_prompt(window, cx);
                                            }
                                        });
                                    },
                                ))
                                .with_animation(
                                    SharedString::from(format!(
                                        "session-agent-send-state-{waiting}"
                                    )),
                                    if waiting {
                                        Animation::new(SESSION_AGENT_SEND_PULSE_DURATION)
                                            .repeat()
                                            .with_easing(gpui::bounce(gpui::ease_in_out))
                                    } else {
                                        short_feedback_animation()
                                    },
                                    move |element, delta| {
                                        if waiting {
                                            element.opacity(0.72 + delta * 0.28)
                                        } else {
                                            element.opacity(0.64 + delta * 0.36)
                                        }
                                    },
                                ),
                        ),
                ),
        )
        .into_any_element()
}

/// Handles Ctrl+V / Cmd+V in the composer: if the clipboard holds an image,
/// ingest it as an attachment; otherwise let the default text paste proceed.
fn handle_paste_key(event: &KeyDownEvent, entity: Entity<AppView>, cx: &mut gpui::App) {
    let keystroke = &event.keystroke;
    let is_paste = (keystroke.key == "v" || keystroke.key == "V")
        && (keystroke.modifiers.control || keystroke.modifiers.platform);
    if !is_paste {
        return;
    }
    let Some(item) = cx.read_from_clipboard() else {
        return;
    };
    for entry in item.entries() {
        if let ClipboardEntry::Image(image) = entry {
            let bytes = image.bytes.clone();
            let format = image.format;
            entity.update(cx, |this, cx| {
                this.ingest_clipboard_image(format, bytes, cx);
            });
            return;
        }
    }
}

/// Renders the horizontal attachment preview row shown above the prompt input.
fn render_attachment_preview_row(
    attachments: &[miaominal_core::chat_attachment::ChatAttachment],
    roles: miaominal_settings::theme::Md3Roles,
    entity: Entity<AppView>,
) -> gpui::AnyElement {
    h_flex()
        .id("session-agent-attachment-preview")
        .w_full()
        .gap_2()
        .overflow_x_scroll()
        .children(
            attachments
                .iter()
                .map(|attachment| render_attachment_chip(attachment, roles, entity.clone())),
        )
        .into_any_element()
}

fn attachment_tooltip(text: SharedString) -> impl Fn(&mut Window, &mut gpui::App) -> gpui::AnyView {
    move |window, cx| {
        gpui_component::tooltip::Tooltip::new(text.clone()).build(window, cx)
    }
}

fn render_attachment_chip(
    attachment: &miaominal_core::chat_attachment::ChatAttachment,
    roles: miaominal_settings::theme::Md3Roles,
    entity: Entity<AppView>,
) -> gpui::AnyElement {
    let attachment_id = SharedString::from(attachment.id.clone());
    let filename = SharedString::from(attachment.filename.clone());

    match &attachment.content {
        miaominal_core::chat_attachment::ChatAttachmentContent::Image(image) => {
            let data_uri = format!(
                "data:{};base64,{}",
                attachment.mime_type, image.thumbnail_base64
            );
            h_flex()
                .id(SharedString::from(format!("attachment-chip-{}", attachment.id)))
                .relative()
                .size(px(64.0))
                .rounded(px(6.0))
                .overflow_hidden()
                .bg(rgb(roles.surface_container))
                .child(
                    gpui::img(data_uri)
                        .w(px(64.0))
                        .h(px(64.0))
                        .object_fit(gpui::ObjectFit::Cover),
                )
                .child(
                    div()
                        .id(SharedString::from(format!(
                            "attachment-remove-{}",
                            attachment.id
                        )))
                        .absolute()
                        .top(px(2.0))
                        .right(px(2.0))
                        .size(px(18.0))
                        .rounded_full()
                        .bg(rgb(roles.surface_container_highest))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .tooltip(attachment_tooltip(filename.clone()))
                        .on_mouse_down(gpui::MouseButton::Left, {
                            let entity = entity.clone();
                            let id = attachment_id.clone();
                            move |_, _window, cx| {
                                entity.update(cx, |this, cx| {
                                    this.remove_pending_attachment(id.as_ref(), cx);
                                });
                            }
                        })
                        .child(Icon::new(AppIcon::Close).size(px(12.0))),
                )
                .into_any_element()
        }
        miaominal_core::chat_attachment::ChatAttachmentContent::TextFile(_text_file) => {
            h_flex()
                .id(SharedString::from(format!("attachment-chip-{}", attachment.id)))
                .relative()
                .h(px(64.0))
                .min_w(px(120.0))
                .max_w(px(200.0))
                .rounded(px(6.0))
                .gap_1()
                .px_2()
                .items_center()
                .bg(rgb(roles.surface_container))
                .child(
                    div().flex_shrink_0().child(
                        Icon::new(AppIcon::File)
                            .small()
                            .text_color(rgb(roles.primary)),
                    ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .text_color(rgb(roles.on_surface))
                        .child(truncate_with_ellipsis(filename.as_ref(), 20)),
                )
                .child(
                    div()
                        .id(SharedString::from(format!(
                            "attachment-remove-{}",
                            attachment.id
                        )))
                        .absolute()
                        .top(px(2.0))
                        .right(px(2.0))
                        .size(px(18.0))
                        .rounded_full()
                        .bg(rgb(roles.surface_container_highest))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .tooltip(attachment_tooltip(filename.clone()))
                        .on_mouse_down(gpui::MouseButton::Left, {
                            let entity = entity.clone();
                            let id = attachment_id.clone();
                            move |_, _window, cx| {
                                entity.update(cx, |this, cx| {
                                    this.remove_pending_attachment(id.as_ref(), cx);
                                });
                            }
                        })
                        .child(Icon::new(AppIcon::Close).size(px(12.0))),
                )
                .into_any_element()
        }
    }
}
