use super::super::*;
use super::session_agent_mentions;
use super::session_agent_utils::*;
use crate::ui::components::icon_button_with_tooltip;
use crate::ui::i18n;
use gpui::{Animation, AnimationExt as _, ClipboardEntry};
use std::time::Duration;

const SESSION_AGENT_SEND_PULSE_DURATION: Duration = Duration::from_millis(1100);

fn ai_provider_kind_supports_vision(kind: miaominal_settings::AiProviderKind) -> bool {
    matches!(
        kind,
        miaominal_settings::AiProviderKind::OpenAi
            | miaominal_settings::AiProviderKind::Anthropic
            | miaominal_settings::AiProviderKind::Gemini
            | miaominal_settings::AiProviderKind::OpenRouter
            | miaominal_settings::AiProviderKind::Xai
    )
}

fn selected_ai_provider_kind(app: &AppView) -> Option<miaominal_settings::AiProviderKind> {
    let settings = app.settings_store.settings();
    let selected_id = settings.selected_ai_provider_id.as_deref()?;
    settings
        .ai_providers
        .iter()
        .find(|provider| provider.id == selected_id && provider.enabled)
        .map(|provider| provider.kind)
}

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
    let badge_entity = entity.clone();
    let mention_entity = entity.clone();
    let send_entity = entity;
    let waiting = app.session_agent.is_busy();
    let has_attachments = !app.session_agent.pending_attachments.is_empty();
    let has_pending_images = app
        .session_agent
        .pending_attachments
        .iter()
        .any(|attachment| attachment.is_image());
    let has_targets = !app.session_agent.selected_at_targets.is_empty();
    let at_mention_query = app.session_agent.at_mention_query.clone();
    let selected_provider_kind = selected_ai_provider_kind(app);
    let has_provider = selected_provider_kind.is_some();
    let image_text_fallback = has_pending_images
        && selected_provider_kind.is_some_and(|kind| !ai_provider_kind_supports_vision(kind));

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
                .when_some(at_mention_query, |this, query| {
                    this.child(
                        session_agent_mentions::render_session_agent_at_mention_popup(
                            app,
                            mention_entity.clone(),
                            query,
                        ),
                    )
                })
                .child(
                    v_flex()
                        .rounded(px(8.0))
                        .bg(rgb(roles.surface_container_high))
                        .p_2()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_h(px(86.0))
                                .max_h(px(190.0))
                                .rounded(px(6.0))
                                .relative()
                                .overflow_hidden()
                                .id("session-agent-prompt-input-menu")
                                .when(has_targets || has_attachments, |this| {
                                    this.child(div().flex_shrink_0().child(
                                        render_composer_badge_row(
                                            app,
                                            badge_entity.clone(),
                                            roles,
                                            has_targets,
                                            has_attachments,
                                        ),
                                    ))
                                })
                                .when(image_text_fallback, |this| {
                                    this.child(
                                        h_flex()
                                            .w_full()
                                            .gap_1()
                                            .items_center()
                                            .px_1()
                                            .py_1()
                                            .text_size(miaominal_settings::FontSize::Body.scaled())
                                            .text_color(rgb(text_muted))
                                            .child(Icon::new(AppIcon::Sparkles).small())
                                            .child(i18n::string(
                                                "workspace.panel.agent.messages.image_attachments_text_fallback",
                                            )),
                                    )
                                })
                                .child(
                                    div().flex_1().child(
                                        HintedInput::new(&prompt_input)
                                            .w_full()
                                            .appearance(false)
                                            .focus_bordered(false)
                                            .p_1()
                                            .hint_left(px(4.0))
                                            .hint_top(px(4.0))
                                            .hint_bottom(px(4.0)),
                                    ),
                                )
                                .on_key_down({
                                    let entity = paste_entity.clone();
                                    move |event: &KeyDownEvent, _window, cx| {
                                        handle_paste_key(event, entity.clone(), cx);
                                    }
                                })
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
                                            if !has_provider && !waiting {
                                                i18n::string(
                                                    "workspace.panel.agent.no_provider_configured",
                                                )
                                            } else {
                                                i18n::string(if waiting {
                                                "workspace.panel.agent.tooltips.stop_response"
                                            } else {
                                                "workspace.panel.agent.tooltips.send_message"
                                                })
                                            },
                                            26.0,
                                            8.0,
                                            Some(if waiting {
                                                roles.error_container
                                            } else if !has_provider {
                                                roles.surface_container_highest
                                            } else {
                                                roles.primary
                                            }),
                                            Some(if waiting {
                                                roles.on_error_container
                                            } else if !has_provider {
                                                text_muted
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
                                                        this.submit_session_agent_prompt(
                                                            window, cx,
                                                        );
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

/// Renders target-chips and attachment badges in a single flex-wrap row
/// using the same pill style (`.rounded(px(999.0))`). Attachment badges
/// show a filename with a Close button; target chips show `@name` with
/// a Close button.
fn render_composer_badge_row(
    app: &AppView,
    entity: Entity<AppView>,
    roles: miaominal_settings::theme::Md3Roles,
    has_targets: bool,
    has_attachments: bool,
) -> gpui::AnyElement {
    h_flex()
        .w_full()
        .gap_1()
        .flex_wrap()
        .when(has_targets, |this| {
            let candidates = app.session_agent_target_candidates();
            let names = app.session_agent.selected_at_targets.clone();
            this.children(names.into_iter().map(|name| {
                let remove_name = name.clone();
                let remove_entity = entity.clone();
                let resolved = candidates.iter().any(|candidate| {
                    candidate.name == name
                        || candidate
                            .name
                            .strip_prefix(&name)
                            .is_some_and(|suffix| suffix.starts_with(' '))
                });
                div()
                    .flex_none()
                    .px_2()
                    .py_1()
                    .rounded(px(999.0))
                    .bg(rgb(if resolved {
                        roles.secondary_container
                    } else {
                        roles.error_container
                    }))
                    .text_color(rgb(if resolved {
                        roles.on_secondary_container
                    } else {
                        roles.on_error_container
                    }))
                    .text_size(miaominal_settings::FontSize::Body.scaled())
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .child(format!("@{name}"))
                            .child(
                                div()
                                    .id("session-agent-target-remove")
                                    .size(px(16.0))
                                    .rounded(px(4.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .on_mouse_down(
                                        gpui::MouseButton::Left,
                                        move |_, _window, cx| {
                                            let entity = remove_entity.clone();
                                            let name = remove_name.clone();
                                            entity.update(cx, |this, cx| {
                                                this.remove_session_agent_at_target(name, cx);
                                            });
                                        },
                                    )
                                    .child(Icon::new(AppIcon::Close).size(px(12.0)).text_color(
                                        rgb(if resolved {
                                            roles.on_secondary_container
                                        } else {
                                            roles.on_error_container
                                        }),
                                    )),
                            ),
                    )
                    .into_any_element()
            }))
        })
        .when(has_attachments, |this| {
            let attachments = app.session_agent.pending_attachments.clone();
            this.children(attachments.iter().map(|attachment| {
                let attachment_id = attachment.id.clone();
                let filename = SharedString::from(attachment.filename.clone());
                let remove_entity = entity.clone();
                let remove_id = attachment_id.clone();
                let icon = match &attachment.content {
                    miaominal_core::chat_attachment::ChatAttachmentContent::Image(_) => {
                        AppIcon::Upload
                    }
                    miaominal_core::chat_attachment::ChatAttachmentContent::TextFile(_) => {
                        AppIcon::File
                    }
                };
                let bg = roles.secondary_container;
                let fg = roles.on_secondary_container;
                div()
                    .flex_none()
                    .px_2()
                    .py_1()
                    .rounded(px(999.0))
                    .bg(rgb(bg))
                    .text_size(miaominal_settings::FontSize::Body.scaled())
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .child(Icon::new(icon).small().text_color(rgb(fg)))
                            .child(
                                div()
                                    .min_w(px(0.0))
                                    .text_color(rgb(fg))
                                    .child(truncate_with_ellipsis(filename.as_ref(), 24)),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "attachment-badge-remove-{attachment_id}"
                                    )))
                                    .size(px(16.0))
                                    .rounded(px(4.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .on_mouse_down(
                                        gpui::MouseButton::Left,
                                        move |_, _window, cx| {
                                            let entity = remove_entity.clone();
                                            let id = remove_id.clone();
                                            entity.update(cx, |this, cx| {
                                                this.remove_pending_attachment(id.as_ref(), cx);
                                            });
                                        },
                                    )
                                    .child(
                                        Icon::new(AppIcon::Close)
                                            .size(px(12.0))
                                            .text_color(rgb(fg)),
                                    ),
                            ),
                    )
                    .into_any_element()
            }))
        })
        .into_any_element()
}
