use super::super::*;
use super::session_agent_panel::SESSION_AGENT_USER_BUBBLE_MAX_WIDTH;
use super::session_agent_utils::*;
use super::session_agent_tool_ui::*;
use super::session_agent_tool_parse::*;
use super::session_agent_tools::*;
use crate::ui::components::md3_spinner;
use crate::ui::i18n;
use gpui_component::WindowExt as _;
use theme::ActiveTheme as _;

pub(in crate::ui::shell::layout) fn render_session_agent_messages(
    app: &AppView,
    message_column_width: f32,
    entity: Entity<AppView>,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> gpui::AnyElement {
    let material = miaominal_settings::current_theme().material;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );

    if app.session_agent.messages.is_empty() && !app.session_agent.is_busy() {
        return div()
            .size_full()
            .w_full()
            .items_center()
            .justify_center()
            .flex()
            .text_center()
            .text_size(miaominal_settings::FontSize::Input.scaled())
            .text_color(rgb(text_muted))
            .child(i18n::string("workspace.panel.agent.empty"))
            .into_any_element();
    }

    v_flex()
        .w(px(message_column_width))
        .max_w(px(message_column_width))
        .min_w_0()
        .overflow_x_hidden()
        .gap_2()
        .children(
            app.session_agent
                .messages
                .iter()
                .enumerate()
                .map(|(index, message)| {
                    render_session_agent_message(
                        app,
                        message_column_width,
                        index,
                        message,
                        entity.clone(),
                        window,
                        cx,
                    )
                    .into_any_element()
                }),
        )
        .when(app.session_agent.has_pending_task(), |this| {
            this.child(
                h_flex()
                    .w(px(message_column_width))
                    .max_w(px(message_column_width))
                    .flex_shrink_0()
                    .pl_3()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .child(md3_spinner(16.0))
                    .child(
                        div()
                            .text_size(miaominal_settings::FontSize::Input.scaled())
                            .text_color(rgb(text_muted))
                            .child(i18n::string("workspace.panel.agent.thinking")),
                    ),
            )
        })
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_session_agent_markdown(
    _app: &AppView,
    id: impl Into<ElementId>,
    message: &SessionAgentMessage,
    _color: u32,
    _entity: Entity<AppView>,
    _window: &mut Window,
    _cx: &mut Context<AppView>,
) -> gpui::AnyElement {
    let id = id.into();
    let text_view_id = (id.clone(), "markdown");
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let on_surface_variant = roles.on_surface_variant;

    div()
        .id(id)
        .w_full()
        .min_w_0()
        .min_h(px(20.0))
        .overflow_x_hidden()
        .child(
            gpui_component::text::TextView::markdown(text_view_id, message.content.clone())
                .code_block_actions(move |code_block, _window, _cx| {
                    let code = code_block.code();
                    let language = code_block.lang().unwrap_or_else(|| "text".into());

                    gpui_component::h_flex()
                        .gap_1()
                        .items_center()
                        .child(
                            // Language badge
                            div()
                                .px_1()
                                .rounded(px(4.0))
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .text_color(rgb(on_surface_variant))
                                .child(language.to_string()),
                        )
                        .child(
                            gpui_component::clipboard::Clipboard::new("copy")
                                .value(code.clone()),
                        )
                })
                .selectable(true),
        )
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_session_agent_message(
    app: &AppView,
    message_column_width: f32,
    index: usize,
    message: &SessionAgentMessage,
    entity: Entity<AppView>,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> gpui::AnyElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let is_user = message.role == SessionAgentMessageRole::User;
    let is_error = message.role == SessionAgentMessageRole::Error;
    let context_menu_entity = entity.clone();
    let context_menu_text = message.content.clone();
    if message.role == SessionAgentMessageRole::Thinking {
        if message.content.trim().is_empty() {
            return div().into_any_element();
        }
        return render_session_agent_thinking(
            app,
            message_column_width,
            index,
            message,
            entity,
            window,
            cx,
        )
        .into_any_element();
    }
    if message.role == SessionAgentMessageRole::ToolCall {
        return render_session_agent_tool_call(
            app,
            message_column_width,
            index,
            message,
            entity,
            cx,
        )
        .into_any_element();
    }
    if message.role == SessionAgentMessageRole::Assistant {
        return div()
            .id(SharedString::from(format!(
                "session-agent-message-menu-{index}-assistant"
            )))
            .w(px(message_column_width))
            .max_w(px(message_column_width))
            .min_w_0()
            .flex_shrink_0()
            .overflow_x_hidden()
            .px_1()
            .py_1()
            .child(render_session_agent_markdown(
                app,
                SharedString::from(format!("session-agent-message-{index}-assistant")),
                message,
                roles.on_surface,
                entity.clone(),
                window,
                cx,
            ))
            .context_menu(move |menu, window, cx| {
                let text = context_menu_text.clone();
                let entity = context_menu_entity.clone();
                let selected_text = window.selected_text(cx);
                let selected_text = (!selected_text.trim().is_empty()).then_some(selected_text);
                menu.item(
                    PopupMenuItem::new(i18n::string("workspace.menu.copy")).on_click(
                        move |_, _window, cx| {
                            let text = text.clone();
                            let selected_text = selected_text.clone();
                            entity.update(cx, |this, cx| {
                                this.copy_session_agent_message_or_selection(
                                    "message",
                                    text,
                                    selected_text,
                                    cx,
                                );
                            });
                        },
                    ),
                )
            })
            .into_any_element();
    }

    let bg = if is_user {
        roles.primary_container
    } else if is_error {
        roles.error_container
    } else {
        roles.surface_container_high
    };
    let fg = if is_user {
        roles.on_primary_container
    } else if is_error {
        roles.on_error_container
    } else {
        roles.on_surface
    };
    let label = match message.role {
        SessionAgentMessageRole::User => i18n::string("workspace.panel.agent.you"),
        SessionAgentMessageRole::Assistant => i18n::string("workspace.panel.agent.assistant"),
        SessionAgentMessageRole::Thinking => i18n::string("workspace.panel.agent.thinking"),
        SessionAgentMessageRole::ToolCall => "Tool".into(),
        SessionAgentMessageRole::Error => i18n::string("workspace.panel.agent.error"),
    };

    h_flex()
        .id(SharedString::from(format!(
            "session-agent-message-menu-{index}-plain"
        )))
        .w(px(message_column_width))
        .max_w(px(message_column_width))
        .min_w_0()
        .flex_shrink_0()
        .overflow_x_hidden()
        .when(is_user, |this| this.justify_end())
        .child(
            v_flex()
                .w_full()
                .max_w(px(
                    message_column_width.min(SESSION_AGENT_USER_BUBBLE_MAX_WIDTH)
                ))
                .min_w(px(0.0))
                .gap_1()
                .rounded(px(8.0))
                .bg(rgb(bg))
                .px_3()
                .py_2()
                .when(is_error, |this| {
                    this.child(
                        div()
                            .text_size(miaominal_settings::FontSize::Body.scaled())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(fg))
                            .child(label),
                    )
                })
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .text_size(miaominal_settings::FontSize::Input.scaled())
                        .line_height(miaominal_settings::scaled_line_height(21.0))
                        .text_color(rgb(fg))
                        .child(render_session_agent_markdown(
                            app,
                            SharedString::from(format!("session-agent-message-{index}-plain")),
                            message,
                            fg,
                            entity.clone(),
                            window,
                            cx,
                        )),
                ),
        )
        .context_menu(move |menu, window, cx| {
            let text = context_menu_text.clone();
            let entity = context_menu_entity.clone();
            let selected_text = window.selected_text(cx);
            let selected_text = (!selected_text.trim().is_empty()).then_some(selected_text);
            menu.item(
                PopupMenuItem::new(i18n::string("workspace.menu.copy")).on_click(
                    move |_, _window, cx| {
                        let text = text.clone();
                        let selected_text = selected_text.clone();
                        entity.update(cx, |this, cx| {
                            this.copy_session_agent_message_or_selection(
                                "message",
                                text,
                                selected_text,
                                cx,
                            );
                        });
                    },
                ),
            )
        })
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_session_agent_thinking(
    app: &AppView,
    message_column_width: f32,
    index: usize,
    message: &SessionAgentMessage,
    entity: Entity<AppView>,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> gpui::AnyElement {
    let material = miaominal_settings::current_theme().material;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );
    let expanded = message
        .thinking
        .as_ref()
        .is_some_and(|thinking| thinking.expanded);
    let elapsed_ms = message
        .thinking
        .as_ref()
        .and_then(|thinking| thinking.elapsed_ms)
        .unwrap_or_else(|| {
            message
                .thinking
                .as_ref()
                .map(|thinking| thinking.started_at.elapsed().as_millis())
                .unwrap_or(0)
        });
    let token_count = estimate_session_agent_tokens(&message.content);
    let is_active_thinking = message
        .thinking
        .as_ref()
        .is_some_and(|t| t.elapsed_ms.is_none());
    let toggle_entity = entity.clone();

    div()
        .w(px(message_column_width))
        .max_w(px(message_column_width))
        .min_w_0()
        .flex_shrink_0()
        .overflow_x_hidden()
        .child(
            v_flex()
                .gap_1()
                .pl_3()
                .child(
                    h_flex()
                        .id(("session-agent-thinking-header", index))
                        .gap_2()
                        .items_center()
                        .cursor_pointer()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .text_color(rgb(text_muted))
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            toggle_entity.update(cx, |this, cx| {
                                this.session_agent.toggle_thinking_expanded(index);
                                cx.notify();
                            });
                        })
                        .child(
                            div()
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .child(if expanded { "v" } else { ">" }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Thinking"),
                        )
                        .when(is_active_thinking, |this| {
                            this.child(format_duration_ms(elapsed_ms))
                        })
                        .when(is_active_thinking, |this| {
                            this.child(format!("~{token_count} tok"))
                        }),
                )
                .when(expanded, |this| {
                    this.child(render_session_agent_markdown(
                        app,
                        SharedString::from(format!("session-agent-message-{index}-thinking")),
                        message,
                        text_muted,
                        entity.clone(),
                        window,
                        cx,
                    ))
                }),
        )
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_session_agent_tool_call(
    _app: &AppView,
    message_column_width: f32,
    index: usize,
    message: &SessionAgentMessage,
    entity: Entity<AppView>,
    cx: &mut Context<AppView>,
) -> gpui::AnyElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );
    let Some(tool_call) = message.tool_call.as_ref() else {
        return div().into_any_element();
    };

    let status_label = match tool_call.status {
        SessionAgentToolStatus::Pending => "Pending",
        SessionAgentToolStatus::WaitingForConfirmation => "Waiting for confirmation",
        SessionAgentToolStatus::InProgress => "Running",
        SessionAgentToolStatus::Completed => "Completed",
        SessionAgentToolStatus::Failed => "Failed",
        SessionAgentToolStatus::Rejected => "Rejected",
    };
    let needs_confirmation = matches!(
        tool_call.status,
        SessionAgentToolStatus::WaitingForConfirmation
    );
    let expanded = tool_call.expanded;
    let is_run_shell = tool_call.name == "run_shell";
    let tool_id = tool_call.id.clone();
    let allow_tool_id = tool_call.id.clone();
    let deny_tool_id = tool_call.id.clone();
    let allow_entity = entity.clone();
    let deny_entity = entity.clone();
    let copy_all_entity = entity.clone();
    let copy_all_text = format_tool_call_copy_text(tool_call);
    let tool_colors = ToolTerminalColors {
        surface: roles.surface,
        surface_container_lowest: roles.surface_container_lowest,
        outline_variant: roles.outline_variant,
        on_surface: roles.on_surface,
        error: roles.error,
        text_muted,
    };
    let syntax_theme = cx.theme().syntax().clone();

    v_flex()
        .id(("session-agent-tool-call", index))
        .w(px(message_column_width))
        .max_w(px(message_column_width))
        .min_w_0()
        .flex_shrink_0()
        .overflow_hidden()
        .rounded(px(8.0))
        .bg(rgb(roles.surface_container_high))
        .child(
            h_flex()
                .id(("session-agent-tool-call-header", index))
                .w_full()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .when(expanded, |this| {
                    this.border_b_1().border_color(rgb(roles.outline_variant))
                })
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    entity.update(cx, |this, cx| {
                        this.session_agent.toggle_tool_call_expanded(&tool_id);
                        cx.notify();
                    });
                })
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .text_color(rgb(text_muted))
                        .child(if expanded { "v" } else { ">" }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(miaominal_settings::FontSize::Input.scaled())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(roles.on_surface))
                        .child(tool_call.name.clone()),
                )
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .text_color(rgb(text_muted))
                        .child(status_label),
                )
                .child(
                    div()
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                        })
                        .child(icon_button(
                            AppIcon::Copy,
                            24.0,
                            6.0,
                            Some(roles.surface_container_highest),
                            Some(roles.on_surface),
                            None,
                            move |_window, cx| {
                                let text = copy_all_text.clone();
                                copy_all_entity.update(cx, |this, cx| {
                                    this.copy_session_agent_text("tool call", text, cx);
                                });
                            },
                        )),
                ),
        )
        .when(expanded, |this| {
            if is_run_shell {
                this.child(render_run_shell_tool_body(
                    tool_call,
                    tool_colors,
                    &syntax_theme,
                ))
            } else {
                this.child(render_structured_tool_body(tool_call, tool_colors))
            }
        })
        .when(needs_confirmation && expanded, |this| {
            this.child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .px_3()
                    .pb_3()
                    .child(
                        div()
                            .cursor_pointer()
                            .rounded(px(6.0))
                            .px_3()
                            .py_1()
                            .bg(rgb(roles.primary))
                            .text_color(rgb(roles.on_primary))
                            .text_size(miaominal_settings::FontSize::Body.scaled())
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                cx.stop_propagation();
                                let tool_id = allow_tool_id.clone();
                                allow_entity.update(cx, |this, cx| {
                                    this.approve_session_agent_tool_call(tool_id, cx);
                                });
                            })
                            .child("Allow"),
                    )
                    .child(
                        div()
                            .cursor_pointer()
                            .rounded(px(6.0))
                            .px_3()
                            .py_1()
                            .bg(rgb(roles.surface_container_highest))
                            .text_color(rgb(roles.on_surface))
                            .text_size(miaominal_settings::FontSize::Body.scaled())
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                cx.stop_propagation();
                                let tool_id = deny_tool_id.clone();
                                deny_entity.update(cx, |this, cx| {
                                    this.deny_session_agent_tool_call(tool_id, cx);
                                });
                            })
                            .child("Deny"),
                    ),
            )
        })
        .into_any_element()
}
