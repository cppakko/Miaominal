use super::super::*;
use super::session_agent_panel::SESSION_AGENT_USER_BUBBLE_MAX_WIDTH;
use super::session_agent_tool_parse::*;
use super::session_agent_tool_ui::*;
use super::session_agent_tools::*;
use super::session_agent_utils::*;
use crate::ui::components::icon_button_with_tooltip;
use crate::ui::features::agent::conversation_view::SessionAgentMessageView;
use crate::ui::i18n;
use gpui_kit::AnimationExt as _;
use gpui_kit::component::ElementExt as _;
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::text::{TextView, TextViewState};

const SESSION_AGENT_MESSAGE_ENTER_OFFSET: f32 = 8.0;
const SESSION_AGENT_TOOL_BODY_REVEAL_MAX_HEIGHT_REM: f32 = 45.0;

fn session_agent_text_selectable(terminal_originated_selection_drag_active: bool) -> bool {
    !terminal_originated_selection_drag_active
}

fn render_session_agent_message_with_enter_motion<E>(
    element: E,
    enter_key: Option<u64>,
) -> gpui_kit::AnyElement
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

pub(in crate::ui::features::agent) fn render_session_agent_messages(
    agent: &mut AgentController,
    message_column_width: f32,
    terminal_originated_selection_drag_active: bool,
    entity: Entity<AgentController>,
    _window: &mut Window,
    cx: &mut Context<AgentController>,
) -> gpui_kit::AnyElement {
    let material = miaominal_settings::current_theme().material;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );

    if agent.session_agent().messages.is_empty() && !agent.session_agent().is_busy() {
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

    let conversation = match agent.panel_conversation_view() {
        Some(conversation) => conversation,
        None => agent.ensure_conversation_view(cx),
    };
    let (generating_view, list_state) = {
        let conversation = conversation.read(cx);
        (conversation.generating_view(), conversation.list_state())
    };

    // Search is intentionally projected only into visible rows. Normal streaming rows never
    // split their full Markdown source into blocks.
    let search_match_set: std::collections::HashSet<(usize, usize)> = agent
        .session_agent()
        .search_match_indices
        .iter()
        .copied()
        .collect();
    let current_match_block = agent
        .session_agent()
        .search_current_match
        .and_then(|c| agent.session_agent().search_match_indices.get(c).copied());

    gpui_kit::list(
        list_state,
        cx.processor(move |agent, index: usize, window, cx| {
            let (message_view, generating, message_count) = {
                let conversation = conversation.read(cx);
                (
                    conversation.message(index),
                    conversation.is_generating(),
                    conversation.message_count(),
                )
            };
            let Some(message_view) = message_view else {
                if generating && index == message_count {
                    return div()
                        .w(px(message_column_width))
                        .max_w(px(message_column_width))
                        .flex_shrink_0()
                        .child(generating_view.clone())
                        .into_any_element();
                }
                return div().into_any_element();
            };
            let row_focus_handle = message_view.focus_handle(cx);

            let selectable =
                session_agent_text_selectable(terminal_originated_selection_drag_active);
            let (role, expanded, content_empty) = {
                let view = message_view.read(cx);
                (
                    view.role(),
                    view.message()
                        .thinking
                        .as_ref()
                        .is_some_and(|thinking| thinking.expanded),
                    view.content().trim().is_empty(),
                )
            };
            let search_active = agent
                .session_agent()
                .search_query
                .as_ref()
                .is_some_and(|query| !query.trim().is_empty());
            let needs_markdown = !content_empty
                && !search_active
                && match role {
                    SessionAgentMessageRole::ToolCall => false,
                    SessionAgentMessageRole::Thinking => expanded,
                    _ => true,
                };
            let markdown_state = if needs_markdown {
                message_view.update(cx, |view, cx| view.set_selectable(selectable, cx));
                message_view.read(cx).markdown_state()
            } else {
                None
            };

            let (message, estimated_tokens, elapsed_thinking_ms) =
                message_view.update(cx, |view, _cx| {
                    let mut message = view.render_snapshot(search_active);
                    message.motion.enter_key = view.active_enter_motion_key();
                    (message, view.estimated_tokens(), view.elapsed_thinking_ms())
                });
            let rendered = render_session_agent_message(
                agent,
                message_column_width,
                terminal_originated_selection_drag_active,
                index,
                &message,
                content_empty,
                message_view.clone(),
                markdown_state,
                estimated_tokens,
                elapsed_thinking_ms,
                entity.clone(),
                window,
                cx,
                &search_match_set,
                current_match_block,
            );
            div()
                .w_full()
                .pb_2()
                .track_focus(&row_focus_handle)
                .child(rendered)
                .into_any_element()
        }),
    )
    .with_sizing_behavior(gpui_kit::ListSizingBehavior::Auto)
    .size_full()
    .w(px(message_column_width))
    .max_w(px(message_column_width))
    .min_w_0()
    .overflow_x_hidden()
    .pb_2()
    .into_any_element()
}

#[allow(
    clippy::too_many_arguments,
    reason = "the renderer receives independent message, selection, sizing, and GPUI context inputs"
)]
pub(in crate::ui::features::agent) fn render_session_agent_markdown(
    terminal_originated_selection_drag_active: bool,
    id: impl Into<ElementId>,
    message: &SessionAgentMessage,
    markdown_state: Option<Entity<TextViewState>>,
    _color: u32,
    _entity: Entity<AgentController>,
    _window: &mut Window,
    _cx: &mut Context<AgentController>,
) -> gpui_kit::AnyElement {
    if markdown_state.is_none() && message.content.trim().is_empty() {
        return div().into_any_element();
    }

    let id = id.into();
    let text_view_id = (id.clone(), "markdown");
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let on_surface_variant = roles.on_surface_variant;
    let selectable = session_agent_text_selectable(terminal_originated_selection_drag_active);

    let text_view = if let Some(state) = markdown_state {
        TextView::new(&state)
    } else {
        TextView::markdown(text_view_id, message.content.clone())
    };

    div()
        .id(id)
        .w_full()
        .min_w_0()
        .min_h(px(20.0))
        .overflow_x_hidden()
        .child(
            text_view
                .code_block_actions(move |code_block, _window, _cx| {
                    let code = code_block.code();
                    let language = code_block.lang().unwrap_or_else(|| "text".into());

                    gpui_kit::component::h_flex()
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
                            gpui_kit::component::clipboard::Clipboard::new("copy")
                                .value(code.clone()),
                        )
                })
                .selectable(selectable),
        )
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_session_agent_markdown_block(
    terminal_originated_selection_drag_active: bool,
    id: SharedString,
    message: &SessionAgentMessage,
    block: String,
    fg: u32,
    entity: Entity<AgentController>,
    window: &mut Window,
    cx: &mut Context<AgentController>,
) -> gpui_kit::AnyElement {
    render_session_agent_markdown(
        terminal_originated_selection_drag_active,
        id,
        &SessionAgentMessage {
            role: message.role,
            content: block,
            tool_call: None,
            thinking: None,
            motion: SessionAgentMessageMotion::default(),
            attachments: Default::default(),
        },
        None,
        fg,
        entity,
        window,
        cx,
    )
}

#[allow(clippy::too_many_arguments)]
fn render_session_agent_search_block(
    agent: &AgentController,
    message_column_width: f32,
    terminal_originated_selection_drag_active: bool,
    index: usize,
    block_idx: usize,
    message: &SessionAgentMessage,
    block: String,
    fg: u32,
    entity: Entity<AgentController>,
    window: &mut Window,
    cx: &mut Context<AgentController>,
    search_match_set: &std::collections::HashSet<(usize, usize)>,
    current_match_block: Option<(usize, usize)>,
) -> gpui_kit::AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let is_match = search_match_set.contains(&(index, block_idx));
    let is_current = current_match_block == Some((index, block_idx));
    let should_scroll = agent.session_agent().search_scroll_target == Some((index, block_idx))
        && agent.panel_open()
        && agent.session_agent().panel_view == ChatPanelView::Conversation
        && agent.text_drag_conversation().is_none();
    let scroll_controller = entity.clone();

    let block = div()
        .id(SharedString::from(format!(
            "session-agent-search-block-{index}-{block_idx}"
        )))
        .w(px(message_column_width))
        .max_w(px(message_column_width))
        .min_w_0()
        .flex_shrink_0()
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
                    terminal_originated_selection_drag_active,
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
        })
        .when(should_scroll, move |this| {
            this.on_prepaint(move |bounds, _window, cx| {
                let block_top = bounds.top();
                let scroll_controller = scroll_controller.clone();
                cx.defer(move |cx| {
                    scroll_controller.update(cx, |controller, cx| {
                        let target = (index, block_idx);
                        let conversation = {
                            let state = controller.session_agent();
                            if state.search_scroll_target != Some(target) {
                                return;
                            }
                            state.conversation_view.as_ref().cloned()
                        };
                        let Some(conversation) = conversation else {
                            return;
                        };
                        let list_state = conversation.read(cx).list_state();
                        let Some(item_bounds) = list_state.bounds_for_item(index) else {
                            return;
                        };
                        let offset = block_top - item_bounds.top();
                        conversation.read(cx).scroll_to(index, offset.max(px(0.0)));
                        controller.clear_conversation_search_scroll_target(target, cx);
                    });
                });
            })
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

#[allow(clippy::too_many_arguments)]
pub(in crate::ui::features::agent) fn render_session_agent_message(
    agent: &AgentController,
    message_column_width: f32,
    terminal_originated_selection_drag_active: bool,
    index: usize,
    message: &SessionAgentMessage,
    content_empty: bool,
    message_view: Entity<SessionAgentMessageView>,
    markdown_state: Option<Entity<TextViewState>>,
    estimated_tokens: usize,
    elapsed_thinking_ms: Option<u128>,
    entity: Entity<AgentController>,
    window: &mut Window,
    cx: &mut Context<AgentController>,
    search_match_set: &std::collections::HashSet<(usize, usize)>,
    current_match_block: Option<(usize, usize)>,
) -> gpui_kit::AnyElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let is_user = message.role == SessionAgentMessageRole::User;
    let is_error = message.role == SessionAgentMessageRole::Error;
    let search_active = agent
        .session_agent()
        .search_query
        .as_ref()
        .is_some_and(|query| !query.trim().is_empty());
    let context_menu_entity = entity.clone();
    let context_menu_message_view = message_view.clone();
    if message.role == SessionAgentMessageRole::Thinking {
        if content_empty {
            return div().into_any_element();
        }
        return render_session_agent_thinking(
            terminal_originated_selection_drag_active,
            message_column_width,
            index,
            message,
            message_view,
            markdown_state,
            estimated_tokens,
            elapsed_thinking_ms,
            entity,
            window,
            cx,
        )
        .into_any_element();
    }
    if content_empty && message.role == SessionAgentMessageRole::Assistant {
        return div().into_any_element();
    }
    if message.role == SessionAgentMessageRole::ToolCall {
        return render_session_agent_tool_call(
            agent,
            message_column_width,
            terminal_originated_selection_drag_active,
            index,
            message,
            message_view,
            entity,
            cx,
        )
        .into_any_element();
    }
    if message.role == SessionAgentMessageRole::Assistant {
        let blocks = search_active.then(|| split_message_into_blocks(&message.content));
        let assistant_entity = entity.clone();
        let assistant_message_view = message_view.clone();
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
                        terminal_originated_selection_drag_active,
                        SharedString::from(format!("session-agent-message-{index}-assistant")),
                        message,
                        markdown_state.clone(),
                        fg,
                        entity.clone(),
                        window,
                        cx,
                    ))
                })
                .when(search_active, |this| {
                    this.child(
                        v_flex().gap_2().children(
                            blocks
                                .as_deref()
                                .unwrap_or_default()
                                .iter()
                                .enumerate()
                                .map(|(block_idx, block)| {
                                    render_session_agent_search_block(
                                        agent,
                                        message_column_width,
                                        terminal_originated_selection_drag_active,
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
                        ),
                    )
                })
                .context_menu(move |menu, window, cx| {
                    let entity = assistant_entity.clone();
                    let message_view = assistant_message_view.clone();
                    let selected_text = gpui_kit::base::TextSelection::selected_text(window, cx);
                    let selected_text = (!selected_text.trim().is_empty()).then_some(selected_text);
                    menu.item(
                        PopupMenuItem::new(i18n::string("workspace.menu.copy")).on_click(
                            move |_, _window, cx| {
                                let text = message_view.read(cx).content().to_string();
                                let text = selected_text.clone().unwrap_or(text);
                                entity.update(cx, |controller, cx| {
                                    controller.copy_text(
                                        i18n::string("workspace.panel.agent.labels.message"),
                                        text,
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
        SessionAgentMessageRole::ToolCall => i18n::string("workspace.panel.agent.tool"),
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
                            terminal_originated_selection_drag_active,
                            SharedString::from(format!("session-agent-message-{index}-plain")),
                            message,
                            markdown_state,
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
                                        agent,
                                        message_column_width
                                            .min(SESSION_AGENT_USER_BUBBLE_MAX_WIDTH),
                                        terminal_originated_selection_drag_active,
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
                    })
                    .when(is_user && !message.attachments.is_empty(), |this| {
                        this.child(render_session_agent_message_attachments(
                            &message.attachments,
                            roles,
                        ))
                    }),
            )
            .context_menu(move |menu, window, cx| {
                let entity = context_menu_entity.clone();
                let message_view = context_menu_message_view.clone();
                let selected_text = gpui_kit::base::TextSelection::selected_text(window, cx);
                let selected_text = (!selected_text.trim().is_empty()).then_some(selected_text);
                menu.item(
                    PopupMenuItem::new(i18n::string("workspace.menu.copy")).on_click(
                        move |_, _window, cx| {
                            let text = message_view.read(cx).content().to_string();
                            let text = selected_text.clone().unwrap_or(text);
                            entity.update(cx, |controller, cx| {
                                controller.copy_text(
                                    i18n::string("workspace.panel.agent.labels.message"),
                                    text,
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

#[allow(
    clippy::too_many_arguments,
    reason = "the renderer receives independent animation, message, layout, and GPUI context inputs"
)]
pub(in crate::ui::features::agent) fn render_session_agent_thinking(
    terminal_originated_selection_drag_active: bool,
    message_column_width: f32,
    index: usize,
    message: &SessionAgentMessage,
    message_view: Entity<SessionAgentMessageView>,
    markdown_state: Option<Entity<TextViewState>>,
    token_count: usize,
    elapsed_thinking_ms: Option<u128>,
    entity: Entity<AgentController>,
    window: &mut Window,
    cx: &mut Context<AgentController>,
) -> gpui_kit::AnyElement {
    let material = miaominal_settings::current_theme().material;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );
    let expanded = message
        .thinking
        .as_ref()
        .is_some_and(|thinking| thinking.expanded);
    let elapsed_ms = elapsed_thinking_ms.unwrap_or_default();
    let is_active_thinking = message
        .thinking
        .as_ref()
        .is_some_and(|t| t.elapsed_ms.is_none());
    let toggle_controller = entity.clone();
    let toggle_message_view = message_view;

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
                                toggle_controller.update(cx, |controller, cx| {
                                    let snapshot = controller.toggle_thinking_expanded(index, cx);
                                    if let Some(snapshot) = snapshot {
                                        toggle_message_view.update(cx, |view, cx| {
                                            view.set_message_snapshot(snapshot, cx);
                                        });
                                    }
                                });
                            })
                            .child(
                                div()
                                    .size(px(14.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        Icon::new(if expanded {
                                            AppIcon::ChevronDown
                                        } else {
                                            AppIcon::Next
                                        })
                                        .size(px(14.0)),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(i18n::string("workspace.panel.agent.thinking_title")),
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
                            terminal_originated_selection_drag_active,
                            SharedString::from(format!("session-agent-message-{index}-thinking")),
                            message,
                            markdown_state,
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
) -> gpui_kit::AnyElement {
    let dot = div()
        .size(px(6.0))
        .rounded(px(999.0))
        .bg(rgb(color))
        .flex_shrink_0();
    let id = SharedString::from(format!("session-agent-tool-status-{tool_id}-{status:?}"));

    dot.with_animation(id, short_feedback_animation(), move |element, delta| {
        let base = if matches!(
            status,
            SessionAgentToolStatus::Pending
                | SessionAgentToolStatus::WaitingForConfirmation
                | SessionAgentToolStatus::InProgress
        ) {
            0.38
        } else {
            0.48
        };
        element.opacity(base + delta * (1.0 - base))
    })
    .into_any_element()
}

fn render_session_agent_tool_header_leading(
    expanded: bool,
    tool_id: &str,
    status: SessionAgentToolStatus,
    status_color: u32,
    text_muted: u32,
) -> gpui_kit::AnyElement {
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

fn session_agent_tool_title(tool_name: &str) -> String {
    match tool_name {
        "workspace_info" => i18n::string("workspace.panel.agent.tool_titles.workspace_info"),
        "read" => i18n::string("workspace.panel.agent.tool_titles.read"),
        "list" => i18n::string("workspace.panel.agent.tool_titles.list"),
        "glob" => i18n::string("workspace.panel.agent.tool_titles.glob"),
        "grep" => i18n::string("workspace.panel.agent.tool_titles.grep"),
        "apply_patch" => i18n::string("workspace.panel.agent.tool_titles.apply_patch"),
        "run_shell" => i18n::string("workspace.panel.agent.tool_titles.run_shell"),
        "start_job" => i18n::string("workspace.panel.agent.tool_titles.start_job"),
        "list_jobs" => i18n::string("workspace.panel.agent.tool_titles.list_jobs"),
        "poll_job" => i18n::string("workspace.panel.agent.tool_titles.poll_job"),
        "stop_job" => i18n::string("workspace.panel.agent.tool_titles.stop_job"),
        "edit_file" => i18n::string("workspace.panel.agent.tool_titles.edit_file"),
        "write_file" => i18n::string("workspace.panel.agent.tool_titles.write_file"),
        "job_start" => i18n::string("workspace.panel.agent.tool_titles.job_start"),
        "job_poll" => i18n::string("workspace.panel.agent.tool_titles.job_poll"),
        "job_stop" => i18n::string("workspace.panel.agent.tool_titles.job_stop"),
        "terminal_read" => i18n::string("workspace.panel.agent.tool_titles.terminal_read"),
        "update_plan" => i18n::string("workspace.panel.agent.tool_titles.update_plan"),
        "web_search" => i18n::string("workspace.panel.agent.tool_titles.web_search"),
        "web_fetch" => i18n::string("workspace.panel.agent.tool_titles.web_fetch"),
        "ask_user" | "approval" => i18n::string("workspace.panel.agent.tool_titles.ask_user"),
        _ => tool_name.to_string(),
    }
}

/// MD3 state layer: `overlay` at `opacity` over `base` (both 0xRRGGBB).
fn state_layer(overlay: u32, base: u32, opacity: f32) -> u32 {
    let mix = |shift: u32| {
        let o = ((overlay >> shift) & 0xff) as f32;
        let b = ((base >> shift) & 0xff) as f32;
        ((b + (o - b) * opacity).round().clamp(0.0, 255.0) as u32) << shift
    };
    mix(16) | mix(8) | mix(0)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AgentActionButtonKind {
    Primary,
    Tonal,
    Choice,
}

fn render_session_agent_text_action_button(
    id: SharedString,
    label: String,
    description: Option<String>,
    kind: AgentActionButtonKind,
    roles: miaominal_settings::theme::Md3Roles,
    cx: &App,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> gpui_kit::AnyElement {
    use gpui_kit::component::button::ButtonCustomVariant;

    let primary = kind == AgentActionButtonKind::Primary;
    let full_width = kind == AgentActionButtonKind::Choice;
    let (background, foreground) = if primary {
        (roles.primary, roles.on_primary)
    } else {
        (roles.secondary_container, roles.on_secondary_container)
    };
    Button::new(id)
        .custom(
            ButtonCustomVariant::new(cx)
                .color(rgb(background).into())
                .foreground(rgb(foreground).into())
                .hover(rgb(state_layer(foreground, background, 0.08)).into())
                .active(rgb(state_layer(foreground, background, 0.12)).into()),
        )
        .small()
        .rounded_full()
        .flex_shrink_0()
        .accessibility_label(label.clone())
        .when(full_width, |this| this.w_full())
        .map(|button| match description {
            None => button.label(label),
            Some(description) => button.h_auto().min_h_8().py_2().rounded_lg().child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .whitespace_normal()
                    .gap_1()
                    .child(div().font_weight(FontWeight::MEDIUM).child(label))
                    .child(
                        div()
                            .text_color(rgb(if primary {
                                roles.on_primary
                            } else {
                                roles.on_secondary_container
                            }))
                            .child(description),
                    ),
            ),
        })
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_click(window, cx);
        })
        .into_any_element()
}

fn render_session_agent_approval_actions(
    agent: &AgentController,
    tool_id: &str,
    entity: Entity<AgentController>,
    roles: miaominal_settings::theme::Md3Roles,
    cx: &App,
) -> gpui_kit::AnyElement {
    let (grant_prefix, editable, _target) =
        agent
            .pending_approval_details(tool_id)
            .unwrap_or((None, None, String::new()));
    let input_mode = agent.approval_input_mode(tool_id);
    let input = agent.ask_user_input();
    let allow_tool_id = tool_id.to_string();
    let allow_entity = entity.clone();
    let deny_tool_id = tool_id.to_string();
    let deny_entity = entity.clone();
    let menu_tool_id = tool_id.to_string();
    let menu_entity = entity.clone();
    let submit_entity = entity;

    v_flex()
        .w_full()
        .gap_2()
        .px_3()
        .pb_3()
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(render_session_agent_text_action_button(
                    SharedString::from(format!("agent-approve-{tool_id}")),
                    i18n::string("workspace.panel.agent.tool_actions.approve"),
                    None,
                    AgentActionButtonKind::Primary,
                    roles,
                    cx,
                    move |_window, cx| {
                        let tool_id = allow_tool_id.clone();
                        allow_entity.update(cx, |controller, cx| {
                            controller.approve_session_agent_tool_call(tool_id, cx);
                        });
                    },
                ))
                .child(render_session_agent_text_action_button(
                    SharedString::from(format!("agent-deny-{tool_id}")),
                    i18n::string("workspace.panel.agent.tool_actions.deny"),
                    None,
                    AgentActionButtonKind::Tonal,
                    roles,
                    cx,
                    move |_window, cx| {
                        let tool_id = deny_tool_id.clone();
                        deny_entity.update(cx, |controller, cx| {
                            controller.deny_tool_call_with_reason(tool_id, String::new(), cx);
                        });
                    },
                ))
                .child(
                    Button::new(SharedString::from(format!(
                        "session-agent-approval-more-{tool_id}"
                    )))
                    .ghost()
                    .small()
                    .dropdown_caret(true)
                    .label(i18n::string("workspace.panel.agent.tool_actions.more"))
                    .dropdown_menu(move |menu, _, _| {
                        let mut menu = menu.min_w(220.0);
                        if let Some(prefix) = grant_prefix.clone() {
                            let tool_id = menu_tool_id.clone();
                            let entity = menu_entity.clone();
                            menu = menu.item(
                                PopupMenuItem::new(i18n::string_args(
                                    "workspace.panel.agent.tool_actions.approve_session",
                                    &[("prefix", &prefix)],
                                ))
                                .on_click(move |_, _, cx| {
                                    let tool_id = tool_id.clone();
                                    entity.update(cx, |controller, cx| {
                                        controller
                                            .approve_session_agent_tool_call_for_session_scope(
                                                tool_id, cx,
                                            );
                                    });
                                }),
                            );
                        }
                        if editable.is_some() {
                            let tool_id = menu_tool_id.clone();
                            let entity = menu_entity.clone();
                            menu = menu.item(
                                PopupMenuItem::new(i18n::string(
                                    "workspace.panel.agent.tool_actions.edit",
                                ))
                                .on_click(move |_, window, cx| {
                                    let tool_id = tool_id.clone();
                                    entity.update(cx, |controller, cx| {
                                        controller.begin_approval_input(
                                            tool_id,
                                            ApprovalInputMode::Edit,
                                            window,
                                            cx,
                                        );
                                    });
                                }),
                            );
                        }
                        let tool_id = menu_tool_id.clone();
                        let entity = menu_entity.clone();
                        menu.item(
                            PopupMenuItem::new(i18n::string(
                                "workspace.panel.agent.tool_actions.deny_with_reason",
                            ))
                            .on_click(move |_, window, cx| {
                                let tool_id = tool_id.clone();
                                entity.update(cx, |controller, cx| {
                                    controller.begin_approval_input(
                                        tool_id,
                                        ApprovalInputMode::DenyReason,
                                        window,
                                        cx,
                                    );
                                });
                            }),
                        )
                    }),
                ),
        )
        .when_some(input_mode, |this, mode| {
            this.child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div().flex_1().min_w(px(0.0)).child(
                            HintedInput::new(&input)
                                .w_full()
                                .border_0()
                                .rounded(px(6.0))
                                .bg(rgb(roles.surface_container_highest))
                                .text_color(rgb(roles.on_surface)),
                        ),
                    )
                    .child(icon_button_with_tooltip(
                        AppIcon::Send,
                        i18n::string(match mode {
                            ApprovalInputMode::Edit => {
                                "workspace.panel.agent.tool_actions.approve_edited"
                            }
                            ApprovalInputMode::DenyReason => {
                                "workspace.panel.agent.tool_actions.deny"
                            }
                        }),
                        26.0,
                        8.0,
                        Some(roles.primary),
                        Some(roles.on_primary),
                        None,
                        move |window, cx| {
                            submit_entity.update(cx, |controller, cx| {
                                controller.submit_approval_input(window, cx);
                            });
                        },
                    )),
            )
        })
        .into_any_element()
}

fn render_session_agent_ask_user_actions(
    agent: &AgentController,
    tool_call: &SessionAgentToolCall,
    entity: Entity<AgentController>,
    roles: miaominal_settings::theme::Md3Roles,
    cx: &App,
) -> gpui_kit::AnyElement {
    let prompt = parse_ask_user_prompt(tool_call);
    let input = agent.ask_user_input();
    let submit_entity = entity.clone();

    v_flex()
        .w_full()
        .gap_2()
        .px_3()
        .pb_3()
        .when(!prompt.choices.is_empty(), |this| {
            this.child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .children(prompt.choices.iter().enumerate().map(|(index, choice)| {
                        let choice_entity = entity.clone();
                        let tool_id = tool_call.id.clone();
                        let choice_id = SharedString::from(format!(
                            "session-agent-ask-user-choice-{}-{index}",
                            tool_call.id.as_str()
                        ));
                        let answer = choice.label.clone();
                        div()
                            .id(choice_id)
                            .w_full()
                            .child(render_session_agent_text_action_button(
                                SharedString::from(format!(
                                    "agent-answer-{}-{index}",
                                    tool_call.id
                                )),
                                choice.label.clone(),
                                choice.description.clone(),
                                AgentActionButtonKind::Choice,
                                roles,
                                cx,
                                move |window, cx| {
                                    let tool_id = tool_id.clone();
                                    let answer = answer.clone();
                                    choice_entity.update(cx, |controller, cx| {
                                        controller.submit_session_agent_user_answer(
                                            tool_id,
                                            answer,
                                            Some(index),
                                            false,
                                            window,
                                            cx,
                                        );
                                    });
                                },
                            ))
                            .into_any_element()
                    })),
            )
        })
        .when(prompt.allow_custom, |this| {
            this.child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div().flex_1().min_w(px(0.0)).child(
                            HintedInput::new(&input)
                                .w_full()
                                .border_0()
                                .rounded(px(6.0))
                                .bg(rgb(roles.surface_container_highest))
                                .text_color(rgb(roles.on_surface)),
                        ),
                    )
                    .child(icon_button_with_tooltip(
                        AppIcon::Send,
                        i18n::string("workspace.panel.agent.tooltips.submit_answer"),
                        26.0,
                        8.0,
                        Some(roles.primary),
                        Some(roles.on_primary),
                        None,
                        move |window, cx| {
                            let entity = submit_entity.clone();
                            entity.update(cx, |controller, cx| {
                                controller.submit_active_session_agent_user_answer(window, cx);
                            });
                        },
                    )),
            )
        })
        .into_any_element()
}

#[allow(
    clippy::too_many_arguments,
    reason = "the renderer receives independent tool state, selection, sizing, and GPUI context inputs"
)]
pub(in crate::ui::features::agent) fn render_session_agent_tool_call(
    agent: &AgentController,
    message_column_width: f32,
    _terminal_originated_selection_drag_active: bool,
    index: usize,
    message: &SessionAgentMessage,
    message_view: Entity<SessionAgentMessageView>,
    entity: Entity<AgentController>,
    cx: &mut Context<AgentController>,
) -> gpui_kit::AnyElement {
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
    let is_ask_user = tool_call.name == "ask_user";
    let tool_id = tool_call.id.clone();
    let tool_call_id =
        SharedString::from(format!("session-agent-tool-call-{}", tool_call.id.as_str()));
    let tool_call_header_id = SharedString::from(format!(
        "session-agent-tool-call-header-{}",
        tool_call.id.as_str()
    ));
    let toggle_controller = entity.clone();
    let toggle_message_view = message_view;
    let copy_all_entity = entity.clone();
    let copy_all_text = format_tool_call_copy_text(tool_call);
    let tool_colors = ToolTerminalColors {
        surface: roles.surface,
        surface_container_lowest: roles.surface_container_lowest,
        on_surface: roles.on_surface,
        error: roles.error,
        text_muted,
    };
    let status_color = match tool_call.status {
        SessionAgentToolStatus::Pending | SessionAgentToolStatus::InProgress => roles.primary,
        SessionAgentToolStatus::WaitingForConfirmation => roles.tertiary,
        SessionAgentToolStatus::Completed => roles.secondary,
        SessionAgentToolStatus::Failed => roles.error,
        SessionAgentToolStatus::Rejected => text_muted,
    };

    render_session_agent_message_with_enter_motion(
        v_flex()
            .id(tool_call_id)
            .w(px(message_column_width))
            .max_w(px(message_column_width))
            .min_w_0()
            .flex_shrink_0()
            .overflow_hidden()
            .rounded_lg()
            .bg(rgb(roles.surface_container_high))
            .child(
                h_flex()
                    .id(tool_call_header_id)
                    .w_full()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .when(!expanded, |this| this.py_2())
                    .when(expanded, |this| this.pt_2())
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        toggle_controller.update(cx, |controller, cx| {
                            let snapshot =
                                controller.toggle_tool_call_expanded(&tool_id, index, cx);
                            if let Some(snapshot) = snapshot {
                                toggle_message_view.update(cx, |view, cx| {
                                    view.set_message_snapshot(snapshot, cx);
                                });
                            }
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
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(miaominal_settings::FontSize::Input.scaled())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(roles.on_surface))
                                    .child(session_agent_tool_title(&tool_call.name)),
                            )
                            .when_some(
                                (!expanded)
                                    .then(|| tool_call_summary_line(tool_call))
                                    .flatten(),
                                |this, summary| {
                                    this.child(render_tool_summary_line(summary, tool_colors))
                                },
                            ),
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
                                    copy_all_entity.update(cx, |controller, cx| {
                                        controller.copy_text(
                                            i18n::string("workspace.panel.agent.labels.tool_call"),
                                            text,
                                            cx,
                                        );
                                    });
                                },
                            )),
                    ),
            )
            .when(expanded, |this| {
                let body = if is_run_shell {
                    render_run_shell_tool_body(tool_call, tool_colors)
                } else {
                    render_structured_tool_body(tool_call, tool_colors)
                };

                this.child(div().w_full().overflow_hidden().child(body).with_animation(
                    SharedString::from(format!("session-agent-tool-body-reveal-{}", tool_call.id)),
                    container_transition_animation(),
                    |element, delta| {
                        // Limit only during the reveal. A permanent outer cap
                        // can clip the second block at larger interface sizes;
                        // command and result viewports own their own limits.
                        element.when(delta < 1.0, |element| {
                            element.max_h(gpui_kit::rems(
                                delta * SESSION_AGENT_TOOL_BODY_REVEAL_MAX_HEIGHT_REM,
                            ))
                        })
                    },
                ))
            })
            .when(needs_confirmation && expanded, |this| {
                if is_ask_user {
                    this.child(render_session_agent_ask_user_actions(
                        agent,
                        tool_call,
                        entity.clone(),
                        roles,
                        cx,
                    ))
                } else {
                    this.child(render_session_agent_approval_actions(
                        agent,
                        &tool_call.id,
                        entity.clone(),
                        roles,
                        cx,
                    ))
                }
            }),
        message.motion.enter_key,
    )
}

/// Renders attachment filename chips inside a user message bubble.
fn render_session_agent_message_attachments(
    attachments: &[miaominal_core::chat_attachment::ChatAttachment],
    roles: miaominal_settings::theme::Md3Roles,
) -> gpui_kit::AnyElement {
    v_flex()
        .w_full()
        .items_start()
        .gap_1()
        .children(attachments.iter().map(|attachment| {
            let item_id = SharedString::from(format!(
                "session-agent-message-attachment-{}",
                attachment.id.as_str()
            ));
            let icon = match &attachment.content {
                miaominal_core::chat_attachment::ChatAttachmentContent::Image(_) => AppIcon::Upload,
                miaominal_core::chat_attachment::ChatAttachmentContent::TextFile(_) => {
                    AppIcon::File
                }
            };
            h_flex()
                .id(item_id)
                .flex_shrink_0()
                .gap_1()
                .px_2()
                .py_1()
                .rounded(px(6.0))
                .bg(rgb(roles.surface_container))
                .items_center()
                .child(Icon::new(icon).small().text_color(rgb(roles.primary)))
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .text_color(rgb(roles.on_surface))
                        .child(truncate_with_ellipsis(&attachment.filename, 24)),
                )
                .into_any_element()
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{Role, TestAppContext};
    use std::{cell::Cell, rc::Rc};

    struct ApprovalButtonsFixture {
        approved: Rc<Cell<usize>>,
        denied: Rc<Cell<usize>>,
    }

    impl Render for ApprovalButtonsFixture {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let roles = miaominal_settings::current_theme().material.roles;
            let approved = self.approved.clone();
            let denied = self.denied.clone();
            h_flex()
                .tab_group()
                .w(px(282.0))
                .gap_2()
                .child(render_session_agent_text_action_button(
                    "approve-fixture".into(),
                    "Approve".into(),
                    None,
                    AgentActionButtonKind::Primary,
                    roles,
                    cx,
                    move |_, _| approved.set(approved.get() + 1),
                ))
                .child(render_session_agent_text_action_button(
                    "deny-fixture".into(),
                    "Deny".into(),
                    None,
                    AgentActionButtonKind::Tonal,
                    roles,
                    cx,
                    move |_, _| denied.set(denied.get() + 1),
                ))
                .child(
                    Button::new("more-fixture")
                        .ghost()
                        .small()
                        .label("More")
                        .dropdown_caret(true),
                )
        }
    }

    #[gpui_kit::test]
    fn approval_actions_fit_the_small_panel_and_support_keyboard(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let approved = Rc::new(Cell::new(0));
        let denied = Rc::new(Cell::new(0));
        let (a, d) = (approved.clone(), denied.clone());
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|_| ApprovalButtonsFixture {
                approved: a,
                denied: d,
            });
            Root::new(view, window, cx)
        });
        cx.update(|window, cx| {
            window.render_frame(cx);
            let approve = window.find("approve-fixture");
            assert_eq!(approve.role(), Some(Role::Button));
            let deny = window.find("deny-fixture");
            let more = window.find("more-fixture");
            assert_eq!(approve.bounds().top(), deny.bounds().top());
            assert_eq!(approve.bounds().top(), more.bounds().top());
            assert!(
                more.bounds().right() <= px(282.0),
                "actions must fit on one line"
            );
            window.click("approve-fixture", cx);
            assert_eq!(approved.get(), 1, "pointer activation must fire once");
            // Native buttons intentionally keep pointer clicks from stealing
            // focus; keyboard traversal starts at the first action.
            // App-level Tab routing calls focus_next; the isolated fixture
            // exercises that same focus traversal without the app shell.
            window.focus_next(cx);
            window.render_frame(cx);
            assert_eq!(window.find("approve-fixture").focused(), Some(true));
            window.focus_next(cx);
            window.render_frame(cx);
            assert_eq!(window.find("deny-fixture").focused(), Some(true));
            window.press("enter", cx);
            assert_eq!(denied.get(), 1, "keyboard activation must fire once");
        });
    }

    #[test]
    fn session_agent_text_is_selectable_without_terminal_originated_drag() {
        assert!(session_agent_text_selectable(false));
    }

    #[test]
    fn session_agent_text_is_not_selectable_during_terminal_originated_drag() {
        assert!(!session_agent_text_selectable(true));
    }
}
