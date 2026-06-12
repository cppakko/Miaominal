use super::super::*;
use crate::ui::i18n;
use theme::ActiveTheme as _;
use zed_markdown::{MarkdownElement, MarkdownStyle};

const SESSION_AGENT_PANEL_HORIZONTAL_PADDING: f32 = 24.0;
const SESSION_AGENT_PANEL_MIN_WIDTH: f32 = 300.0;
const SESSION_AGENT_PANEL_MAX_WIDTH: f32 = 720.0;
const SESSION_AGENT_PANEL_RESIZE_HANDLE_WIDTH: f32 = 8.0;
const SESSION_AGENT_USER_BUBBLE_MAX_WIDTH: f32 = 420.0;

#[derive(Clone, Copy)]
struct SessionAgentPanelResizeMarker;

impl Render for SessionAgentPanelResizeMarker {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1.0)).h(px(1.0))
    }
}

pub(in crate::ui::shell::layout) fn clamp_session_agent_panel_width(width: f32) -> f32 {
    width.clamp(SESSION_AGENT_PANEL_MIN_WIDTH, SESSION_AGENT_PANEL_MAX_WIDTH)
}

fn session_agent_message_column_width(panel_width: f32) -> f32 {
    (panel_width - SESSION_AGENT_PANEL_HORIZONTAL_PADDING)
        .max(SESSION_AGENT_PANEL_MIN_WIDTH - SESSION_AGENT_PANEL_HORIZONTAL_PADDING)
}

fn render_session_agent_resize_handle(
    is_dragging: bool,
    cx: &mut Context<AppView>,
) -> gpui::AnyElement {
    div()
        .id("session-agent-sidebar-resize-handle")
        .absolute()
        .top(px(0.0))
        .left(px(0.0))
        .bottom(px(0.0))
        .w(px(SESSION_AGENT_PANEL_RESIZE_HANDLE_WIDTH))
        .cursor_col_resize()
        .occlude()
        .child(
            div()
                .absolute()
                .left(px(3.0))
                .top(px(12.0))
                .bottom(px(12.0))
                .w(px(1.0))
                .rounded(px(999.0))
                
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                this.workspace_state.session_agent_panel_drag = Some(SessionAgentPanelDragState {
                    initial_pointer: f32::from(event.position.x),
                    initial_width: this.workspace_state.session_agent_panel_width,
                });
                cx.stop_propagation();
                cx.notify();
            }),
        )
        .hover(move |this| {
            if is_dragging {
                this
            } else {
                this.cursor_col_resize()
            }
        })
        .on_drag(
            SessionAgentPanelResizeMarker,
            |marker, _offset, _window, cx| cx.new(|_| *marker),
        )
        .into_any_element()
}

impl AppView {
    fn render_session_agent_sidebar_toolbar(&self, entity: Entity<Self>) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let icon_bg = roles.surface_container;
        let close_entity = entity.clone();

        h_flex()
            .w_full()
            .h(px(30.0))
            .items_center()
            .gap_1()
            .child(div().id("session-agent-new-chat").child(icon_button(
                AppIcon::Plus,
                26.0,
                8.0,
                Some(icon_bg),
                Some(text_muted),
                None,
                move |window, cx| {
                    let entity = entity.clone();
                    entity.update(cx, |this, cx| {
                        this.reset_session_agent_chat(window, cx);
                    });
                },
            )))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .pl_1()
                    .text_size(miaominal_settings::FontSize::Input.scaled())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(roles.on_surface))
                    .child(i18n::string("workspace.panel.agent.sidebar_title")),
            )
            .child(div().id("session-agent-close").child(icon_button(
                AppIcon::PanelRight,
                26.0,
                8.0,
                Some(icon_bg),
                Some(text_muted),
                None,
                move |_window, cx| {
                    let entity = close_entity.clone();
                    entity.update(cx, |this, cx| {
                        this.panels.session_agent_panel_open = false;
                        cx.notify();
                    });
                },
            )))
            .into_any_element()
    }

    pub(in crate::ui::shell::layout) fn render_session_agent_sidebar(
        &self,
        entity: Entity<Self>,
        _session: &SessionTabState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let panel_width =
            clamp_session_agent_panel_width(self.workspace_state.session_agent_panel_width);
        let is_dragging = self.workspace_state.session_agent_panel_drag.is_some();

        card_surface(roles.surface_container, 16.0)
            .id("session-agent-sidebar")
            .relative()
            .w(px(panel_width))
            .h_full()
            .flex_shrink_0()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .child(render_session_agent_resize_handle(is_dragging, cx))
            .child(
                v_flex()
                    .size_full()
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(42.0))
                            .flex_shrink_0()
                            .items_center()
                            .px_2()
                            .child(self.render_session_agent_sidebar_toolbar(entity.clone())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.0))
                            .child(self.render_session_agent_panel(entity, window, cx)),
                    ),
            )
            .into_any_element()
    }

    pub(in crate::ui::shell::layout) fn render_session_agent_panel(
        &self,
        entity: Entity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let provider_select = self.panel_forms.settings.ai_provider_select.clone();
        let prompt_input = self.workspace_forms.agent.prompt_input.clone();
        let send_entity = entity.clone();
        let waiting = self.session_agent.is_busy();
        let agent_scroll_handle = self.workspace_state.session_agent_scroll_handle.clone();
        let message_column_width =
            session_agent_message_column_width(self.workspace_state.session_agent_panel_width);

        v_flex()
            .id("session-agent-panel-content")
            .size_full()
            .overflow_hidden()
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .px_3()
                    .pt_2()
                    .gap_3()
                    .child(
                        h_flex().w_full().h(px(28.0)).items_center().gap_2().child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_size(miaominal_settings::FontSize::Subheading.scaled())
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(roles.on_surface))
                                .child(i18n::string("workspace.panel.agent.chat")),
                        ),
                    )
                    .child(
                        div().flex_1().min_h_0().child(
                            div()
                                .id("session-agent-scroll")
                                .size_full()
                                .overflow_x_hidden()
                                .track_scroll(&agent_scroll_handle)
                                .overflow_y_scroll()
                                .pb_2()
                                .child(self.render_session_agent_messages(
                                    message_column_width,
                                    entity.clone(),
                                    window,
                                    cx,
                                ))
                                .vertical_scrollbar(&agent_scroll_handle),
                        ),
                    ),
            )
            .child(
                div().flex_shrink_0().p_2().child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .rounded(px(8.0))
                        .bg(rgb(roles.surface_container_high))
                        .p_2()
                        .child(
                            div()
                                .w_full()
                                .min_h(px(86.0))
                                .max_h(px(190.0))
                                .rounded(px(6.0))
                                .overflow_hidden()
                                .child(
                                    Input::new(&prompt_input)
                                        .w_full()
                                        .appearance(false)
                                        .focus_bordered(false)
                                        .p_1(),
                                ),
                        )
                        .child(
                            h_flex()
                                .w_full()
                                .h(px(28.0))
                                .items_center()
                                .gap_2()
                                .child(icon_button(
                                    AppIcon::Plus,
                                    24.0,
                                    8.0,
                                    Some(roles.surface_container_high),
                                    Some(text_muted),
                                    None,
                                    |_window, _cx| {},
                                ))
                                .child(div().h(px(16.0)).w(px(1.0)).bg(rgb(roles.outline_variant)))
                                .child(
                                    div().w(px(112.0)).min_w(px(0.0)).child(
                                        md3_select(&provider_select)
                                            .small()
                                            .w_full()
                                            .bg(rgb(roles.surface_container_high)),
                                    ),
                                )
                                .child(icon_button(
                                    AppIcon::Sliders,
                                    24.0,
                                    8.0,
                                    Some(roles.surface_container_high),
                                    Some(text_muted),
                                    None,
                                    |_window, _cx| {},
                                ))
                                .child(div().flex_1())
                                .child(icon_button(
                                    if waiting {
                                        AppIcon::Pause
                                    } else {
                                        AppIcon::ChevronUp
                                    },
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
                                )),
                        ),
                ),
            )
            .into_any_element()
    }

    fn render_session_agent_messages(
        &self,
        message_column_width: f32,
        entity: Entity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );

        if self.session_agent.messages.is_empty() && !self.session_agent.is_busy() {
            return div()
                .flex_1()
                .min_h(px(180.0))
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
                self.session_agent
                    .messages
                    .iter()
                    .enumerate()
                    .map(|(index, message)| {
                        self.render_session_agent_message(
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
            .when(self.session_agent.has_pending_task(), |this| {
                this.child(
                    div()
                        .w(px(message_column_width))
                        .max_w(px(message_column_width))
                        .flex_shrink_0()
                        .pl_3()
                        .py_1()
                        .border_l_2()
                        .border_color(rgb(roles.outline_variant))
                        .text_size(miaominal_settings::FontSize::Input.scaled())
                        .text_color(rgb(text_muted))
                        .child(i18n::string("workspace.panel.agent.thinking")),
                )
            })
            .into_any_element()
    }

    fn render_session_agent_markdown(
        &self,
        id: impl Into<ElementId>,
        message: &SessionAgentMessage,
        color: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;

        let text_color = gpui::Hsla::from(rgb(color));
        let muted_color = gpui::Hsla::from(rgb(crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 70 } else { 45 },
        )));
        let link_color = gpui::Hsla::from(rgb(roles.primary));
        let code_background = gpui::Hsla::from(rgb(roles.surface_container_high));
        let border_color = gpui::Hsla::from(rgb(roles.outline_variant));

        let mut base_text_style = window.text_style();
        base_text_style.refine(&gpui::TextStyleRefinement {
            font_size: Some(miaominal_settings::FontSize::Input.scaled().into()),
            line_height: Some(miaominal_settings::scaled_line_height(21.0).into()),
            color: Some(text_color),
            ..Default::default()
        });

        // Entity<Markdown> is always pre-built in the state-mutation path (append_*_delta /
        // ensure_*_markdown). We only read it here — never create or update it during render.
        let Some(markdown) = message.markdown_entity.clone() else {
            // Fallback: entity not yet allocated (empty/pending content), show plain text.
            return div()
                .id(id)
                .w_full()
                .min_w_0()
                .min_h(px(20.0))
                .text_size(miaominal_settings::FontSize::Input.scaled())
                .line_height(miaominal_settings::scaled_line_height(21.0))
                .text_color(text_color)
                .child(message.content.clone())
                .into_any_element();
        };

        let style = MarkdownStyle {
            base_text_style,
            selection_background_color: gpui::Hsla::from(rgb(roles.primary)).opacity(0.28),
            rule_color: border_color,
            block_quote_border_color: border_color,
            code_block_overflow_x_scroll: true,
            code_block: gpui::StyleRefinement {
                padding: gpui::EdgesRefinement {
                    top: Some(gpui::DefiniteLength::Absolute(
                        gpui::AbsoluteLength::Pixels(px(8.0)),
                    )),
                    left: Some(gpui::DefiniteLength::Absolute(
                        gpui::AbsoluteLength::Pixels(px(10.0)),
                    )),
                    right: Some(gpui::DefiniteLength::Absolute(
                        gpui::AbsoluteLength::Pixels(px(10.0)),
                    )),
                    bottom: Some(gpui::DefiniteLength::Absolute(
                        gpui::AbsoluteLength::Pixels(px(8.0)),
                    )),
                },
                margin: gpui::EdgesRefinement {
                    top: Some(gpui::Length::Definite(px(6.0).into())),
                    left: Some(gpui::Length::Definite(px(0.0).into())),
                    right: Some(gpui::Length::Definite(px(0.0).into())),
                    bottom: Some(gpui::Length::Definite(px(8.0).into())),
                },
                border_style: Some(gpui::BorderStyle::Solid),
                border_widths: gpui::EdgesRefinement {
                    top: Some(gpui::AbsoluteLength::Pixels(px(1.0))),
                    left: Some(gpui::AbsoluteLength::Pixels(px(1.0))),
                    right: Some(gpui::AbsoluteLength::Pixels(px(1.0))),
                    bottom: Some(gpui::AbsoluteLength::Pixels(px(1.0))),
                },
                border_color: Some(border_color),
                background: Some(code_background.into()),
                text: gpui::TextStyleRefinement {
                    font_size: Some(miaominal_settings::FontSize::Body.scaled().into()),
                    color: Some(text_color),
                    ..Default::default()
                },
                ..Default::default()
            },
            inline_code: gpui::TextStyleRefinement {
                font_size: Some(miaominal_settings::FontSize::Body.scaled().into()),
                background_color: Some(code_background),
                color: Some(text_color),
                ..Default::default()
            },
            block_quote: gpui::TextStyleRefinement {
                color: Some(muted_color),
                ..Default::default()
            },
            link: gpui::TextStyleRefinement {
                color: Some(link_color),
                underline: Some(gpui::UnderlineStyle {
                    color: Some(link_color.opacity(0.65)),
                    thickness: px(1.0),
                    ..Default::default()
                }),
                ..Default::default()
            },
            heading_level_styles: Some(zed_markdown::HeadingLevelStyles {
                h1: Some(gpui::TextStyleRefinement {
                    font_size: Some(miaominal_settings::FontSize::Subheading.scaled().into()),
                    font_weight: Some(FontWeight::SEMIBOLD),
                    ..Default::default()
                }),
                h2: Some(gpui::TextStyleRefinement {
                    font_size: Some(miaominal_settings::FontSize::Input.scaled().into()),
                    font_weight: Some(FontWeight::SEMIBOLD),
                    ..Default::default()
                }),
                h3: Some(gpui::TextStyleRefinement {
                    font_size: Some(miaominal_settings::FontSize::Input.scaled().into()),
                    font_weight: Some(FontWeight::SEMIBOLD),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            heading_border_color: Some(border_color),
            syntax: cx.theme().syntax().clone(),
            ..Default::default()
        };

        div()
            .id(id)
            .w_full()
            .min_w_0()
            .min_h(px(20.0))
            .overflow_x_hidden()
            .child(
                MarkdownElement::new(markdown, style)
                    .w_full()
                    .min_w_0()
                    .min_h(px(20.0))
                    .overflow_x_hidden(),
            )
            .into_any_element()
    }

    fn render_session_agent_message(
        &self,
        message_column_width: f32,
        index: usize,
        message: &SessionAgentMessage,
        entity: Entity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let is_user = message.role == SessionAgentMessageRole::User;
        let is_error = message.role == SessionAgentMessageRole::Error;
        if message.role == SessionAgentMessageRole::Thinking {
            if message.content.trim().is_empty() {
                return div().into_any_element();
            }
            return self
                .render_session_agent_thinking(
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
            return self
                .render_session_agent_tool_call(message_column_width, index, message, entity, cx)
                .into_any_element();
        }
        if message.role == SessionAgentMessageRole::Assistant {
            return div()
                .w(px(message_column_width))
                .max_w(px(message_column_width))
                .min_w_0()
                .flex_shrink_0()
                .overflow_x_hidden()
                .px_1()
                .py_1()
                .child(self.render_session_agent_markdown(
                    SharedString::from(format!("session-agent-message-{index}-assistant")),
                    message,
                    roles.on_surface,
                    window,
                    cx,
                ))
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
                            .child(message.content.clone()),
                    ),
            )
            .into_any_element()
    }

    fn render_session_agent_thinking(
        &self,
        message_column_width: f32,
        index: usize,
        message: &SessionAgentMessage,
        entity: Entity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
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
                    .border_l_2()
                    .border_color(rgb(roles.outline_variant))
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
                            .child(format_duration_ms(elapsed_ms))
                            .child(format!("~{token_count} tok")),
                    )
                    .when(expanded, |this| {
                        this.child(self.render_session_agent_markdown(
                            SharedString::from(format!("session-agent-message-{index}-thinking")),
                            message,
                            text_muted,
                            window,
                            cx,
                        ))
                    }),
            )
            .into_any_element()
    }

    fn render_session_agent_tool_call(
        &self,
        message_column_width: f32,
        index: usize,
        message: &SessionAgentMessage,
        entity: Entity<Self>,
        cx: &mut Context<Self>,
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
}

#[derive(Clone, Copy)]
struct ToolTerminalColors {
    surface: u32,
    surface_container_lowest: u32,
    outline_variant: u32,
    on_surface: u32,
    error: u32,
    text_muted: u32,
}

fn render_structured_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    if arguments_are_streaming(tool_call) && tool_call.name != "apply_patch" {
        return render_preparing_tool_body(tool_call, colors);
    }

    match tool_call.name.as_str() {
        "apply_patch" => render_apply_patch_tool_body(tool_call, colors),
        "read" => render_read_tool_body(tool_call, colors),
        "list" => render_list_tool_body(tool_call, colors),
        "glob" => render_glob_tool_body(tool_call, colors),
        "grep" => render_grep_tool_body(tool_call, colors),
        "start_job" => render_start_job_tool_body(tool_call, colors),
        "list_jobs" => render_list_jobs_tool_body(tool_call, colors),
        "poll_job" => render_poll_job_tool_body(tool_call, colors),
        "stop_job" => render_job_tool_body(tool_call, colors),
        "web_search" => render_web_search_tool_body(tool_call, colors),
        "web_fetch" => render_web_fetch_tool_body(tool_call, colors),
        "workspace_info" => render_workspace_info_tool_body(tool_call, colors),
        "ask_user" | "approval" => render_approval_tool_body(tool_call, colors),
        _ => render_generic_tool_body(tool_call, colors),
    }
}

fn render_run_shell_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
    syntax_theme: &::theme::SyntaxTheme,
) -> gpui::AnyElement {
    let command = parse_run_shell_command(&tool_call.arguments)
        .or_else(|| partial_json_string_field(&tool_call.arguments, "command"))
        .unwrap_or_else(|| {
            if arguments_are_streaming(tool_call) {
                "Preparing command...".to_string()
            } else {
                "No command".to_string()
            }
        });
    let result = if tool_has_result_status(tool_call) {
        tool_call
            .confirmation_note
            .as_deref()
            .and_then(parse_run_shell_result)
    } else {
        None
    };
    let result_block = result
        .map(|result| {
            (
                format!("Result - exit {}", result.exit_status),
                result.display_text(),
                result.exit_status != 0,
            )
        })
        .or_else(|| {
            tool_display_result(tool_call).map(|result| ("Result".to_string(), result, false))
        });

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_bash_highlighted_command_block(
            "Command",
            &command,
            colors,
            syntax_theme,
        ))
        .when_some(result_block, |this, (label, content, error)| {
            this.child(render_tool_terminal_block(&label, content, colors, error))
        })
        .into_any_element()
}

fn render_apply_patch_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let patch = string_field(args.as_ref(), "patch")
        .or_else(|| partial_json_string_field(&tool_call.arguments, "patch"))
        .unwrap_or_else(|| "Preparing patch...".to_string());
    let patch_ready = !patch.trim().is_empty() && patch != "Preparing patch...";
    let base_dir = string_field(args.as_ref(), "base_dir")
        .or_else(|| partial_json_string_field(&tool_call.arguments, "base_dir"))
        .unwrap_or_else(|| ".".to_string());
    let files = patch_paths(&patch);
    let summary = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "summary"))
        .or_else(|| tool_display_result(tool_call));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![
                ("Base".to_string(), base_dir),
                (
                    "Files".to_string(),
                    if files.is_empty() {
                        if patch_ready {
                            "No files detected".to_string()
                        } else {
                            "Detecting files...".to_string()
                        }
                    } else {
                        files.join(", ")
                    },
                ),
            ],
            colors,
        ))
        .child(render_tool_terminal_block("Diff", patch, colors, false))
        .when_some(
            summary.filter(|summary| !summary.trim().is_empty()),
            |this, summary| {
                this.child(render_tool_terminal_block(
                    "Patch Output",
                    summary,
                    colors,
                    false,
                ))
            },
        )
        .into_any_element()
}

fn render_read_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let path = string_field(args.as_ref(), "path").unwrap_or_else(|| "(unknown path)".to_string());
    let range = match (
        number_field(args.as_ref(), "start_line"),
        number_field(args.as_ref(), "end_line"),
    ) {
        (Some(start), Some(end)) => format!("{start}-{end}"),
        (Some(start), None) => format!("{start}+"),
        _ => "default".to_string(),
    };
    let content = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "content"));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![("Path".to_string(), path), ("Lines".to_string(), range)],
            colors,
        ))
        .when_some(content, |this, content| {
            this.child(render_tool_terminal_block(
                "Content", content, colors, false,
            ))
        })
        .into_any_element()
}

fn render_list_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let path = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "path"))
        .or_else(|| string_field(args.as_ref(), "path"))
        .unwrap_or_else(|| ".".to_string());
    let entries = output
        .as_ref()
        .and_then(|value| value.get("entries"))
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .take(80)
                .filter_map(|entry| {
                    let name = entry.get("name")?.as_str()?;
                    let kind = entry
                        .get("entry_type")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("item");
                    Some(format!("{kind:>9}  {name}"))
                })
                .collect::<Vec<_>>()
                .join("\n")
        });

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![("Path".to_string(), path)],
            colors,
        ))
        .when_some(entries, |this, entries| {
            this.child(render_tool_terminal_block(
                "Entries", entries, colors, false,
            ))
        })
        .into_any_element()
}

fn render_glob_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let root = string_field(args.as_ref(), "root").unwrap_or_else(|| ".".to_string());
    let pattern = string_field(args.as_ref(), "pattern").unwrap_or_else(|| "(pattern)".to_string());
    let entries = list_entries_text(output.as_ref());

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![("Root".to_string(), root), ("Pattern".to_string(), pattern)],
            colors,
        ))
        .when_some(entries, |this, entries| {
            this.child(render_tool_terminal_block(
                "Matches", entries, colors, false,
            ))
        })
        .into_any_element()
}

fn render_grep_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let root = string_field(args.as_ref(), "root").unwrap_or_else(|| ".".to_string());
    let pattern = string_field(args.as_ref(), "pattern").unwrap_or_else(|| "(pattern)".to_string());
    let content = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "content"));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![("Root".to_string(), root), ("Pattern".to_string(), pattern)],
            colors,
        ))
        .when_some(content, |this, content| {
            this.child(render_tool_terminal_block(
                "Matches", content, colors, false,
            ))
        })
        .into_any_element()
}

fn render_start_job_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let command = string_field(args.as_ref(), "command").unwrap_or_else(|| "(command)".to_string());
    let cwd = string_field(args.as_ref(), "cwd").unwrap_or_else(|| ".".to_string());
    let mut fields = vec![("Cwd".to_string(), cwd)];
    if let Some(job_id) = output
        .as_ref()
        .and_then(|value| value.get("job_id"))
        .map(display_json_value)
    {
        fields.push(("Job".to_string(), job_id));
    }

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(fields, colors))
        .child(render_tool_terminal_block(
            "Command", command, colors, false,
        ))
        .into_any_element()
}

fn render_job_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let job_id = args
        .as_ref()
        .and_then(|value| value.get("job_id"))
        .map(display_json_value)
        .unwrap_or_else(|| "(job)".to_string());
    let content = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "content"))
        .or_else(|| tool_display_result(tool_call));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![("Job".to_string(), job_id)],
            colors,
        ))
        .when_some(content, |this, content| {
            this.child(render_tool_terminal_block("Result", content, colors, false))
        })
        .into_any_element()
}

fn render_list_jobs_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let output = tool_output_value(tool_call);
    let content = output
        .as_ref()
        .and_then(|value| value.get("jobs"))
        .map(display_json_value)
        .or_else(|| tool_display_result(tool_call));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .when_some(content, |this, content| {
            this.child(render_tool_terminal_block("Jobs", content, colors, false))
        })
        .into_any_element()
}

fn render_poll_job_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let result = output.as_ref().and_then(|value| value.get("result"));
    let job_id = args
        .as_ref()
        .and_then(|value| value.get("job_id"))
        .or_else(|| result.and_then(|value| value.get("job_id")))
        .map(display_json_value)
        .unwrap_or_else(|| "(job)".to_string());
    let mut fields = vec![("Job".to_string(), job_id)];
    if let Some(status) = result.and_then(|value| string_field(Some(value), "status")) {
        fields.push(("Status".to_string(), status));
    }
    if let Some(exit_status) = result
        .and_then(|value| value.get("exit_status"))
        .filter(|value| !value.is_null())
        .map(display_json_value)
    {
        fields.push(("Exit".to_string(), exit_status));
    }
    let stdout = result.and_then(|value| string_field(Some(value), "stdout"));
    let stderr = result.and_then(|value| string_field(Some(value), "stderr"));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(fields, colors))
        .when_some(
            stdout.filter(|text| !text.trim().is_empty()),
            |this, stdout| this.child(render_tool_terminal_block("Stdout", stdout, colors, false)),
        )
        .when_some(
            stderr.filter(|text| !text.trim().is_empty()),
            |this, stderr| this.child(render_tool_terminal_block("Stderr", stderr, colors, true)),
        )
        .when(result.is_none(), |this| {
            this.when_some(tool_display_result(tool_call), |this, content| {
                this.child(render_tool_terminal_block("Result", content, colors, false))
            })
        })
        .into_any_element()
}

fn render_web_search_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let query = string_field(args.as_ref(), "query").unwrap_or_else(|| "(query)".to_string());
    let results = output
        .as_ref()
        .and_then(|value| value.get("results"))
        .map(display_json_value);

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![("Query".to_string(), query)],
            colors,
        ))
        .when_some(results, |this, results| {
            this.child(render_tool_terminal_block(
                "Results", results, colors, false,
            ))
        })
        .into_any_element()
}

fn render_web_fetch_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let url = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "url"))
        .or_else(|| string_field(args.as_ref(), "url"))
        .unwrap_or_else(|| "(url)".to_string());
    let content = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "content"));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![("Url".to_string(), url)],
            colors,
        ))
        .when_some(content, |this, content| {
            this.child(render_tool_terminal_block(
                "Content", content, colors, false,
            ))
        })
        .into_any_element()
}

fn render_workspace_info_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let output = tool_output_value(tool_call);
    let fields = output
        .as_ref()
        .map(|value| {
            vec![
                (
                    "Host".to_string(),
                    string_field(Some(value), "host").unwrap_or_default(),
                ),
                (
                    "User".to_string(),
                    string_field(Some(value), "user").unwrap_or_default(),
                ),
                (
                    "Cwd".to_string(),
                    string_field(Some(value), "cwd").unwrap_or_default(),
                ),
                (
                    "Shell".to_string(),
                    string_field(Some(value), "shell").unwrap_or_default(),
                ),
            ]
        })
        .unwrap_or_else(|| vec![("Status".to_string(), pending_or_note(tool_call))]);

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(fields, colors))
        .into_any_element()
}

fn render_approval_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let message = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "message"))
        .or_else(|| string_field(args.as_ref(), "message"))
        .unwrap_or_else(|| pending_or_note(tool_call));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_terminal_block(
            "Approval", message, colors, false,
        ))
        .into_any_element()
}

fn render_generic_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    if arguments_are_streaming(tool_call) {
        return render_preparing_tool_body(tool_call, colors);
    }

    let args = tool_arguments_value(&tool_call.arguments);
    let fields = args
        .as_ref()
        .and_then(serde_json::Value::as_object)
        .map(|object| {
            object
                .iter()
                .take(8)
                .map(|(key, value)| (title_case_key(key), display_json_value(value)))
                .collect::<Vec<_>>()
        })
        .filter(|fields| !fields.is_empty());
    let result = tool_display_result(tool_call);

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .when_some(fields, |this, fields| {
            this.child(render_tool_field_grid(fields, colors))
        })
        .when_some(result, |this, result| {
            this.child(render_tool_terminal_block("Result", result, colors, false))
        })
        .into_any_element()
}

fn render_preparing_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_terminal_block(
            "Request",
            preparing_tool_text(&tool_call.name),
            colors,
            false,
        ))
        .into_any_element()
}

fn render_tool_terminal_block(
    label: &str,
    content: String,
    colors: ToolTerminalColors,
    error: bool,
) -> gpui::AnyElement {
    render_tool_terminal_block_content(
        label,
        div()
            .font_family("JetBrains Mono")
            .text_size(miaominal_settings::FontSize::Body.scaled())
            .line_height(miaominal_settings::scaled_line_height(18.0))
            .text_color(rgb(if error {
                colors.error
            } else {
                colors.on_surface
            }))
            .child(if content.trim().is_empty() {
                "(no output)".to_string()
            } else {
                content
            })
            .into_any_element(),
        colors,
    )
}

fn render_tool_terminal_block_content(
    label: &str,
    content: gpui::AnyElement,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let material = miaominal_settings::current_theme().material;
    let terminal_bg = if material.dark {
        colors.surface_container_lowest
    } else {
        colors.surface
    };

    v_flex()
        .w_full()
        .overflow_hidden()
        .rounded(px(6.0))
        .border_1()
        .border_color(rgb(colors.outline_variant))
        .bg(rgb(terminal_bg))
        .child(
            div()
                .w_full()
                .px_2()
                .py_1()
                .border_b_1()
                .border_color(rgb(colors.outline_variant))
                .text_size(miaominal_settings::FontSize::Body.scaled())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(colors.text_muted))
                .child(label.to_string()),
        )
        .child(
            div()
                .w_full()
                .min_h(px(34.0))
                .max_h(px(220.0))
                .overflow_y_scrollbar()
                .px_2()
                .py_2()
                .child(content),
        )
        .into_any_element()
}

fn render_tool_field_grid(
    fields: Vec<(String, String)>,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    v_flex()
        .w_full()
        .gap_1()
        .children(fields.into_iter().map(|(label, value)| {
            h_flex()
                .w_full()
                .gap_2()
                .items_start()
                .child(
                    div()
                        .w(px(62.0))
                        .flex_shrink_0()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(colors.text_muted))
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .line_height(miaominal_settings::scaled_line_height(18.0))
                        .text_color(rgb(colors.on_surface))
                        .child(value),
                )
                .into_any_element()
        }))
        .into_any_element()
}

#[derive(Debug, Clone)]
struct RunShellDisplayResult {
    stdout: String,
    stderr: String,
    exit_status: i64,
    timed_out: bool,
    truncated: bool,
}

impl RunShellDisplayResult {
    fn display_text(&self) -> String {
        let mut lines = Vec::new();
        if !self.stdout.trim().is_empty() {
            lines.push(self.stdout.clone());
        }
        if !self.stderr.trim().is_empty() {
            lines.push(self.stderr.clone());
        }
        if self.timed_out {
            lines.push("Command timed out.".to_string());
        }
        if self.truncated {
            lines.push("Output truncated.".to_string());
        }
        lines.join("\n")
    }
}

fn parse_run_shell_command(arguments: &str) -> Option<String> {
    tool_arguments_value(arguments)?
        .get("command")?
        .as_str()
        .map(ToOwned::to_owned)
}

fn parse_run_shell_result(note: &str) -> Option<RunShellDisplayResult> {
    let value: serde_json::Value = serde_json::from_str(note).ok()?;
    let output = value.get("output")?;
    let result = output.get("result")?;
    Some(RunShellDisplayResult {
        stdout: result.get("stdout")?.as_str()?.to_string(),
        stderr: result.get("stderr")?.as_str()?.to_string(),
        exit_status: result.get("exit_status")?.as_i64()?,
        timed_out: result.get("timed_out")?.as_bool()?,
        truncated: result.get("truncated")?.as_bool()?,
    })
}

fn tool_arguments_value(arguments: &str) -> Option<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_str(arguments).ok()?;
    Some(value.get("arguments").unwrap_or(&value).clone())
}

fn tool_response_value(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
) -> Option<serde_json::Value> {
    if !tool_has_result_status(tool_call) {
        return None;
    }

    let note = tool_call.confirmation_note.as_deref()?;
    serde_json::from_str(note).ok()
}

fn tool_output_value(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
) -> Option<serde_json::Value> {
    let response = tool_response_value(tool_call)?;
    let output = response.get("output")?;
    if let Some(kind) = output.get("kind").and_then(serde_json::Value::as_str) {
        if kind == "patch" {
            return Some(output.clone());
        }
    }
    Some(output.clone())
}

fn string_field(value: Option<&serde_json::Value>, key: &str) -> Option<String> {
    value?
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn number_field(value: Option<&serde_json::Value>, key: &str) -> Option<i64> {
    value?.get(key).and_then(serde_json::Value::as_i64)
}

fn list_entries_text(value: Option<&serde_json::Value>) -> Option<String> {
    let entries = value?.get("entries")?.as_array()?;
    Some(
        entries
            .iter()
            .take(100)
            .map(display_json_value)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn pending_or_note(tool_call: &crate::ui::shell::state::SessionAgentToolCall) -> String {
    tool_display_result(tool_call).unwrap_or_else(|| pending_result_text(tool_call))
}

fn tool_has_result_status(tool_call: &crate::ui::shell::state::SessionAgentToolCall) -> bool {
    matches!(
        tool_call.status,
        SessionAgentToolStatus::Completed
            | SessionAgentToolStatus::Failed
            | SessionAgentToolStatus::Rejected
    )
}

fn pending_result_text(tool_call: &crate::ui::shell::state::SessionAgentToolCall) -> String {
    match tool_call.status {
        SessionAgentToolStatus::Pending => "Preparing request...".to_string(),
        SessionAgentToolStatus::WaitingForConfirmation => "Waiting for approval...".to_string(),
        SessionAgentToolStatus::InProgress => "Waiting for result...".to_string(),
        SessionAgentToolStatus::Completed => "No output".to_string(),
        SessionAgentToolStatus::Failed => "Tool failed before returning output.".to_string(),
        SessionAgentToolStatus::Rejected => "Tool was rejected.".to_string(),
    }
}

fn arguments_are_streaming(tool_call: &crate::ui::shell::state::SessionAgentToolCall) -> bool {
    matches!(
        tool_call.status,
        SessionAgentToolStatus::Pending
            | SessionAgentToolStatus::WaitingForConfirmation
            | SessionAgentToolStatus::InProgress
    ) && !tool_call.arguments.trim().is_empty()
        && tool_call.arguments.trim() != "No arguments"
        && tool_arguments_value(&tool_call.arguments).is_none()
}

fn preparing_tool_text(tool_name: &str) -> String {
    match tool_name {
        "read" => "Preparing file read...".to_string(),
        "list" => "Preparing directory listing...".to_string(),
        "glob" => "Preparing file search...".to_string(),
        "grep" => "Preparing text search...".to_string(),
        "start_job" => "Preparing background job...".to_string(),
        "poll_job" => "Preparing job status request...".to_string(),
        "stop_job" => "Preparing job stop request...".to_string(),
        "web_search" => "Preparing web search...".to_string(),
        "web_fetch" => "Preparing web fetch...".to_string(),
        "workspace_info" => "Preparing workspace info request...".to_string(),
        "ask_user" | "approval" => "Preparing approval prompt...".to_string(),
        _ => "Preparing request...".to_string(),
    }
}

fn tool_display_result(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
) -> Option<String> {
    if !tool_has_result_status(tool_call) {
        return None;
    }

    tool_output_value(tool_call)
        .map(|value| display_json_value(&value))
        .or_else(|| {
            let note = tool_call.confirmation_note.as_deref()?;
            if note.trim().is_empty() {
                return None;
            }
            serde_json::from_str::<serde_json::Value>(note)
                .ok()
                .map(|value| display_json_value(&value))
                .or_else(|| Some(note.to_string()))
        })
        .filter(|text| !text.trim().is_empty())
}

fn partial_json_string_field(arguments: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let start = arguments.find(&needle)?;
    let after_key = &arguments[start + needle.len()..];
    let colon = after_key.find(':')?;
    let after_colon = after_key[colon + 1..].trim_start();
    let mut chars = after_colon.chars();
    if chars.next()? != '"' {
        return None;
    }

    let mut value = String::new();
    let mut escaped = false;
    for ch in chars {
        if escaped {
            match ch {
                '"' => value.push('"'),
                '\\' => value.push('\\'),
                '/' => value.push('/'),
                'b' => value.push('\u{0008}'),
                'f' => value.push('\u{000C}'),
                'n' => value.push('\n'),
                'r' => value.push('\r'),
                't' => value.push('\t'),
                _ => value.push(ch),
            }
            escaped = false;
            continue;
        }

        match ch {
            '\\' => escaped = true,
            '"' => break,
            _ => value.push(ch),
        }
    }

    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn display_json_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Array(values) => values
            .iter()
            .take(100)
            .map(display_json_value)
            .collect::<Vec<_>>()
            .join("\n"),
        serde_json::Value::Object(object) => object
            .iter()
            .take(20)
            .map(|(key, value)| format!("{}: {}", title_case_key(key), display_json_value(value)))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn title_case_key(key: &str) -> String {
    match key {
        "path" => "Path".to_string(),
        "root" => "Root".to_string(),
        "pattern" => "Pattern".to_string(),
        "query" => "Query".to_string(),
        "url" => "Url".to_string(),
        "command" => "Command".to_string(),
        "cwd" => "Cwd".to_string(),
        "job_id" => "Job".to_string(),
        "base_dir" => "Base".to_string(),
        "content" => "Content".to_string(),
        "summary" => "Summary".to_string(),
        "message" => "Message".to_string(),
        _ => key.replace('_', " "),
    }
}

fn patch_paths(patch: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for line in patch.lines() {
        let path = line
            .strip_prefix("--- ")
            .or_else(|| line.strip_prefix("+++ "))
            .map(str::trim)
            .and_then(|path| {
                if path == "/dev/null" {
                    None
                } else {
                    Some(path.trim_start_matches("a/").trim_start_matches("b/"))
                }
            });
        if let Some(path) = path
            && !paths.iter().any(|existing| existing == path)
        {
            paths.push(path.to_string());
        }
    }
    paths
}

fn estimate_session_agent_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    chars.saturating_add(3) / 4
}

fn format_duration_ms(ms: u128) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else {
        let seconds = ms as f64 / 1_000.0;
        format!("{seconds:.1}s")
    }
}

fn format_tool_call_copy_text(tool_call: &crate::ui::shell::state::SessionAgentToolCall) -> String {
    let mut text = format!(
        "Tool: {}\nStatus: {:?}\nArguments:\n{}",
        tool_call.name, tool_call.status, tool_call.arguments
    );
    if let Some(result) = tool_call.confirmation_note.as_ref() {
        text.push_str("\n\nResult:\n");
        text.push_str(result);
    }
    text
}

/// Renders a terminal-style block with syntax-highlighted bash command text.
/// Uses tree-sitter directly for synchronous, lightweight, zero-entity highlighting.
/// This avoids any Entity<Markdown> creation inside the render path, preventing stack overflow.
fn render_bash_highlighted_command_block(
    label: &str,
    command: &str,
    colors: ToolTerminalColors,
    syntax_theme: &::theme::SyntaxTheme,
) -> gpui::AnyElement {
    let base_color = gpui::Hsla::from(rgb(colors.on_surface));
    let highlights = collect_bash_highlights(command, syntax_theme);
    let text: SharedString = if command.trim().is_empty() {
        "(no command)".into()
    } else {
        command.to_string().into()
    };
    let content = div()
        .font_family("JetBrains Mono")
        .text_size(miaominal_settings::FontSize::Body.scaled())
        .line_height(miaominal_settings::scaled_line_height(18.0))
        .text_color(base_color)
        .child(gpui::StyledText::new(text).with_highlights(highlights))
        .into_any_element();
    render_tool_terminal_block_content(label, content, colors)
}

/// Cached compiled bash highlight query. Compiled once and reused across renders.
static BASH_HIGHLIGHT_QUERY: std::sync::OnceLock<Option<tree_sitter::Query>> =
    std::sync::OnceLock::new();

fn get_bash_highlight_query() -> Option<&'static tree_sitter::Query> {
    BASH_HIGHLIGHT_QUERY
        .get_or_init(|| {
            let lang: tree_sitter::Language = tree_sitter_bash::LANGUAGE.into();
            tree_sitter::Query::new(&lang, tree_sitter_bash::HIGHLIGHT_QUERY).ok()
        })
        .as_ref()
}

/// Synchronously collects syntax highlight spans for a bash command string
/// using tree-sitter, without creating any GPUI entities.
fn collect_bash_highlights(
    command: &str,
    syntax_theme: &::theme::SyntaxTheme,
) -> Vec<(std::ops::Range<usize>, gpui::HighlightStyle)> {
    use tree_sitter::StreamingIterator as _;

    let Some(query) = get_bash_highlight_query() else {
        return Vec::new();
    };

    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_bash::LANGUAGE.into();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(command.as_bytes(), None) else {
        return Vec::new();
    };

    let capture_names = query.capture_names();
    let mut raw: Vec<(usize, usize, gpui::HighlightStyle)> = Vec::new();

    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), command.as_bytes());
    loop {
        matches.advance();
        let Some(m) = matches.get() else { break };
        for capture in m.captures {
            let start = capture.node.start_byte();
            let end = capture.node.end_byte();
            if start >= end || end > command.len() {
                continue;
            }
            if !command.is_char_boundary(start) || !command.is_char_boundary(end) {
                continue;
            }
            let name_idx = capture.index as usize;
            let Some(capture_name) = capture_names.get(name_idx) else {
                continue;
            };
            let Some(style) = syntax_theme.style_for_name(capture_name) else {
                continue;
            };
            if style != gpui::HighlightStyle::default() {
                raw.push((start, end, style));
            }
        }
    }

    // Sort by start byte; keep only non-overlapping ranges (first match wins).
    raw.sort_by_key(|(start, _, _)| *start);
    let mut result: Vec<(std::ops::Range<usize>, gpui::HighlightStyle)> = Vec::new();
    let mut last_end = 0usize;
    for (start, end, style) in raw {
        if start >= last_end {
            result.push((start..end, style));
            last_end = end;
        }
    }
    result
}
