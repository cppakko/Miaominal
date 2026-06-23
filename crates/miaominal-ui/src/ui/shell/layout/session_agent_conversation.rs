use super::super::*;
use super::session_agent_panel::SESSION_AGENT_USER_BUBBLE_MAX_WIDTH;
use super::session_agent_tool_parse::*;
use super::session_agent_tool_ui::*;
use super::session_agent_tools::*;
use super::session_agent_utils::*;
use crate::ui::components::{icon_button_with_tooltip, md3_spinner};
use crate::ui::i18n;
use gpui::{Animation, AnimationExt as _, ScrollAnchor};
use gpui_component::WindowExt as _;
use std::time::Duration;
use theme::ActiveTheme as _;

const SESSION_AGENT_MESSAGE_ENTER_OFFSET: f32 = 8.0;
const SESSION_AGENT_STATUS_PULSE_DURATION: Duration = Duration::from_millis(1100);
const SESSION_AGENT_TOOL_BODY_REVEAL_MAX_HEIGHT: f32 = 720.0;

fn session_agent_text_selectable(terminal_originated_selection_drag_active: bool) -> bool {
    !terminal_originated_selection_drag_active
}

fn render_session_agent_message_with_enter_motion<E>(
    element: E,
    enter_key: Option<u64>,
) -> gpui::AnyElement
where
    E: IntoElement + Styled + 'static,
{
    if let Some(enter_key) = enter_key {
        element
            .with_animation(
                SharedString::from(format!("session-agent-message-enter-{enter_key}")),
                list_enter_animation(),
                |element, delta| {
                    element
                        .opacity(delta)
                        .top(px((1.0 - delta) * SESSION_AGENT_MESSAGE_ENTER_OFFSET))
                },
            )
            .into_any_element()
    } else {
        element.into_any_element()
    }
}

fn render_session_agent_pulsing_spinner(
    stable_key: impl AsRef<str>,
    size: f32,
) -> gpui::AnyElement {
    div()
        .child(md3_spinner(size))
        .with_animation(
            SharedString::from(format!(
                "session-agent-pending-pulse-{}",
                stable_key.as_ref()
            )),
            Animation::new(SESSION_AGENT_STATUS_PULSE_DURATION)
                .repeat()
                .with_easing(gpui::bounce(gpui::ease_in_out)),
            |element, delta| element.opacity(0.46 + delta * 0.54),
        )
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_session_agent_messages(
    app: &AppView,
    message_column_width: f32,
    scroll_handle: Option<&ScrollHandle>,
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

    // Build block-level search match set + current match block
    let search_match_set: std::collections::HashSet<(usize, usize)> = app
        .session_agent
        .search_match_indices
        .iter()
        .copied()
        .collect();
    let current_match_block = app
        .session_agent
        .search_current_match
        .and_then(|c| app.session_agent.search_match_indices.get(c).copied());

    v_flex()
        .id("session-agent-message-scroll-content")
        .when_some(scroll_handle, |this, scroll_handle| {
            this.size_full()
                .track_scroll(scroll_handle)
                .overflow_y_scroll()
                .pb_2()
        })
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
                        &search_match_set,
                        current_match_block,
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
                    .child(render_session_agent_pulsing_spinner("task", 16.0))
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
    app: &AppView,
    id: impl Into<ElementId>,
    message: &SessionAgentMessage,
    _color: u32,
    _entity: Entity<AppView>,
    _window: &mut Window,
    _cx: &mut Context<AppView>,
) -> gpui::AnyElement {
    if message.content.trim().is_empty() {
        return div().into_any_element();
    }

    let id = id.into();
    let text_view_id = (id.clone(), "markdown");
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let on_surface_variant = roles.on_surface_variant;
    let selectable = session_agent_text_selectable(app.terminal_originated_selection_drag_active());

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
                            gpui_component::clipboard::Clipboard::new("copy").value(code.clone()),
                        )
                })
                .selectable(selectable),
        )
        .into_any_element()
}

fn render_session_agent_markdown_block(
    app: &AppView,
    id: SharedString,
    message: &SessionAgentMessage,
    block: String,
    fg: u32,
    entity: Entity<AppView>,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> gpui::AnyElement {
    render_session_agent_markdown(
        app,
        id,
        &SessionAgentMessage {
            role: message.role,
            content: block,
            tool_call: None,
            thinking: None,
            motion: SessionAgentMessageMotion::default(),
        },
        fg,
        entity,
        window,
        cx,
    )
}

fn render_session_agent_search_block(
    app: &AppView,
    message_column_width: f32,
    index: usize,
    block_idx: usize,
    message: &SessionAgentMessage,
    block: String,
    fg: u32,
    entity: Entity<AppView>,
    window: &mut Window,
    cx: &mut Context<AppView>,
    search_match_set: &std::collections::HashSet<(usize, usize)>,
    current_match_block: Option<(usize, usize)>,
) -> gpui::AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let is_match = search_match_set.contains(&(index, block_idx));
    let is_current = current_match_block == Some((index, block_idx));
    let anchor = if app.session_agent.search_scroll_target == Some((index, block_idx)) {
        let anchor =
            ScrollAnchor::for_handle(app.workspace_state.session_agent_scroll_handle.clone());
        anchor.scroll_to(window, cx);
        Some(anchor)
    } else {
        None
    };

    let block = div()
        .id(SharedString::from(format!(
            "session-agent-search-block-{index}-{block_idx}"
        )))
        .w(px(message_column_width))
        .max_w(px(message_column_width))
        .min_w_0()
        .flex_shrink_0()
        .anchor_scroll(anchor)
        .when(is_match, |this| {
            this.border_l_2()
                .border_color(rgb(if is_current {
                    roles.primary
                } else {
                    roles.primary_container
                }))
                .pl_1()
        })
        .child(
            div()
                .w_full()
                .min_w_0()
                .child(render_session_agent_markdown_block(
                    app,
                    SharedString::from(format!("session-agent-message-{index}-block-{block_idx}")),
                    message,
                    block,
                    fg,
                    entity,
                    window,
                    cx,
                )),
        )
        .when(is_current, |this| {
            this.bg(color_with_alpha(roles.primary, 0x12))
                .rounded(px(6.0))
        });

    if is_current {
        block
            .with_animation(
                SharedString::from(format!("session-agent-search-current-{index}-{block_idx}")),
                short_feedback_animation(),
                move |element, delta| {
                    element.bg(color_with_alpha(
                        roles.primary,
                        (42.0 * (1.0 - delta)).round() as u8,
                    ))
                },
            )
            .into_any_element()
    } else {
        block.into_any_element()
    }
}

pub(in crate::ui::shell::layout) fn render_session_agent_message(
    app: &AppView,
    message_column_width: f32,
    index: usize,
    message: &SessionAgentMessage,
    entity: Entity<AppView>,
    window: &mut Window,
    cx: &mut Context<AppView>,
    search_match_set: &std::collections::HashSet<(usize, usize)>,
    current_match_block: Option<(usize, usize)>,
) -> gpui::AnyElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let is_user = message.role == SessionAgentMessageRole::User;
    let is_error = message.role == SessionAgentMessageRole::Error;
    let search_active = app
        .session_agent
        .search_query
        .as_ref()
        .is_some_and(|query| !query.trim().is_empty());
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
        let blocks = split_message_into_blocks(&message.content);
        let assistant_ct = message.content.clone();
        let assistant_entity = entity.clone();
        let fg = roles.on_surface;

        return render_session_agent_message_with_enter_motion(
            div()
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
                .when(!search_active, |this| {
                    this.child(render_session_agent_markdown(
                        app,
                        SharedString::from(format!("session-agent-message-{index}-assistant")),
                        message,
                        fg,
                        entity.clone(),
                        window,
                        cx,
                    ))
                })
                .when(search_active, |this| {
                    this.child(v_flex().gap_2().children(blocks.iter().enumerate().map(
                        |(block_idx, block)| {
                            render_session_agent_search_block(
                                app,
                                message_column_width,
                                index,
                                block_idx,
                                message,
                                block.clone(),
                                fg,
                                entity.clone(),
                                window,
                                cx,
                                search_match_set,
                                current_match_block,
                            )
                        },
                    )))
                })
                .context_menu(move |menu, window, cx| {
                    let text = assistant_ct.clone();
                    let entity = assistant_entity.clone();
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
                }),
            message.motion.enter_key,
        );
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

    render_session_agent_message_with_enter_motion(
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
                    .when(!search_active, |this| {
                        this.child(render_session_agent_markdown(
                            app,
                            SharedString::from(format!("session-agent-message-{index}-plain")),
                            message,
                            fg,
                            entity.clone(),
                            window,
                            cx,
                        ))
                    })
                    .when(search_active, |this| {
                        this.children(
                            split_message_into_blocks(&message.content)
                                .iter()
                                .enumerate()
                                .map(|(block_idx, block)| {
                                    render_session_agent_search_block(
                                        app,
                                        message_column_width
                                            .min(SESSION_AGENT_USER_BUBBLE_MAX_WIDTH),
                                        index,
                                        block_idx,
                                        message,
                                        block.clone(),
                                        fg,
                                        entity.clone(),
                                        window,
                                        cx,
                                        search_match_set,
                                        current_match_block,
                                    )
                                }),
                        )
                    }),
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
            }),
        message.motion.enter_key,
    )
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
    if is_active_thinking {
        window.request_animation_frame();
    }
    let toggle_entity = entity.clone();

    render_session_agent_message_with_enter_motion(
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
            ),
        message.motion.enter_key,
    )
}

fn render_session_agent_tool_status_indicator(
    tool_id: &str,
    status: SessionAgentToolStatus,
    color: u32,
) -> gpui::AnyElement {
    let dot = div()
        .size(px(6.0))
        .rounded(px(999.0))
        .bg(rgb(color))
        .flex_shrink_0();
    let id = SharedString::from(format!("session-agent-tool-status-{tool_id}-{status:?}"));

    if matches!(
        status,
        SessionAgentToolStatus::Pending
            | SessionAgentToolStatus::WaitingForConfirmation
            | SessionAgentToolStatus::InProgress
    ) {
        dot.with_animation(
            id,
            Animation::new(SESSION_AGENT_STATUS_PULSE_DURATION)
                .repeat()
                .with_easing(gpui::bounce(gpui::ease_in_out)),
            |element, delta| element.opacity(0.38 + delta * 0.62),
        )
        .into_any_element()
    } else {
        dot.with_animation(id, short_feedback_animation(), |element, delta| {
            element.opacity(0.48 + delta * 0.52)
        })
        .into_any_element()
    }
}

fn render_session_agent_tool_header_leading(
    expanded: bool,
    tool_id: &str,
    status: SessionAgentToolStatus,
    status_color: u32,
    text_muted: u32,
) -> gpui::AnyElement {
    h_flex()
        .w(px(30.0))
        .flex_shrink_0()
        .items_center()
        .gap(px(5.0))
        .child(render_session_agent_tool_status_indicator(
            tool_id,
            status,
            status_color,
        ))
        .child(
            div()
                .size(px(14.0))
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(text_muted))
                .child(
                    Icon::from(if expanded {
                        AppIcon::ChevronDown
                    } else {
                        AppIcon::Next
                    })
                    .size(px(14.0)),
                ),
        )
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_session_agent_tool_call(
    app: &AppView,
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
        selectable: session_agent_text_selectable(app.terminal_originated_selection_drag_active()),
    };
    let syntax_theme = cx.theme().syntax().clone();

    let status_color = match tool_call.status {
        SessionAgentToolStatus::Pending | SessionAgentToolStatus::InProgress => roles.primary,
        SessionAgentToolStatus::WaitingForConfirmation => roles.tertiary,
        SessionAgentToolStatus::Completed => roles.secondary,
        SessionAgentToolStatus::Failed => roles.error,
        SessionAgentToolStatus::Rejected => text_muted,
    };

    render_session_agent_message_with_enter_motion(
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
                    .child(render_session_agent_tool_header_leading(
                        expanded,
                        &tool_call.id,
                        tool_call.status,
                        status_color,
                        text_muted,
                    ))
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
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                cx.stop_propagation();
                            })
                            .child(icon_button_with_tooltip(
                                AppIcon::Copy,
                                i18n::string("workspace.panel.agent.tooltips.copy_tool_call"),
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
                let body = if is_run_shell {
                    render_run_shell_tool_body(tool_call, tool_colors, &syntax_theme)
                } else {
                    render_structured_tool_body(tool_call, tool_colors)
                };

                this.child(div().w_full().overflow_hidden().child(body).with_animation(
                    SharedString::from(format!("session-agent-tool-body-reveal-{}", tool_call.id)),
                    container_transition_animation(),
                    |element, delta| {
                        element.max_h(px(delta * SESSION_AGENT_TOOL_BODY_REVEAL_MAX_HEIGHT))
                    },
                ))
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
                                .child(i18n::string("workspace.panel.agent.tool_actions.approve")),
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
                                .child(i18n::string("workspace.panel.agent.tool_actions.deny")),
                        ),
                )
            }),
        message.motion.enter_key,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_agent_text_is_selectable_without_terminal_originated_drag() {
        assert!(session_agent_text_selectable(false));
    }

    #[test]
    fn session_agent_text_is_not_selectable_during_terminal_originated_drag() {
        assert!(!session_agent_text_selectable(true));
    }
}
