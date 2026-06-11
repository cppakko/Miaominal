use super::super::*;
use crate::ui::i18n;
use zed_markdown::{Markdown, MarkdownElement, MarkdownStyle};

const SESSION_AGENT_PANEL_HORIZONTAL_PADDING: f32 = 24.0;
const SESSION_AGENT_MESSAGE_COLUMN_WIDTH: f32 =
    super::workspace::SESSION_MONITOR_PANEL_WIDTH - SESSION_AGENT_PANEL_HORIZONTAL_PADDING;

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

        card_surface(roles.surface_container, 16.0)
            .id("session-agent-sidebar")
            .w(px(super::workspace::SESSION_MONITOR_PANEL_WIDTH))
            .h_full()
            .flex_shrink_0()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
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
        let waiting = self.session_agent.is_waiting();

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
                                .size_full()
                                .overflow_x_hidden()
                                .overflow_y_scrollbar()
                                .pb_2()
                                .child(self.render_session_agent_messages(
                                    entity.clone(),
                                    window,
                                    cx,
                                )),
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
                                            if this.session_agent.is_waiting() {
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

        if self.session_agent.messages.is_empty() && !self.session_agent.is_waiting() {
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
            .w(px(SESSION_AGENT_MESSAGE_COLUMN_WIDTH))
            .max_w(px(SESSION_AGENT_MESSAGE_COLUMN_WIDTH))
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
                            index,
                            message,
                            entity.clone(),
                            window,
                            cx,
                        )
                        .into_any_element()
                    }),
            )
            .when(self.session_agent.is_waiting(), |this| {
                this.child(
                    div()
                        .w(px(SESSION_AGENT_MESSAGE_COLUMN_WIDTH))
                        .max_w(px(SESSION_AGENT_MESSAGE_COLUMN_WIDTH))
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
        message_index: usize,
        content: String,
        color: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        self.render_session_agent_markdown_with_mode(
            id,
            message_index,
            content,
            color,
            false,
            window,
            cx,
        )
    }

    fn render_session_agent_text(
        &self,
        id: impl Into<ElementId>,
        message_index: usize,
        content: String,
        color: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        self.render_session_agent_markdown_with_mode(
            id,
            message_index,
            content,
            color,
            true,
            window,
            cx,
        )
    }

    fn render_session_agent_markdown_with_mode(
        &self,
        id: impl Into<ElementId>,
        message_index: usize,
        content: String,
        color: u32,
        text_only: bool,
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

        let cache_key = (message_index, text_only);
        let cached = self
            .session_agent
            .markdown_cache
            .borrow()
            .get(&cache_key)
            .cloned();
        let markdown = match cached {
            Some((ref cached_content, ref entity)) if cached_content == &content => entity.clone(),
            Some((_, entity)) => {
                entity.update(cx, |md, cx| md.reset(content.clone().into(), cx));
                self.session_agent
                    .markdown_cache
                    .borrow_mut()
                    .insert(cache_key, (content.clone(), entity.clone()));
                entity
            }
            None => {
                let entity = cx.new(|cx| {
                    if text_only {
                        Markdown::new_text(content.clone().into(), cx)
                    } else {
                        Markdown::new(content.clone().into(), None, None, cx)
                    }
                });
                self.session_agent
                    .markdown_cache
                    .borrow_mut()
                    .insert(cache_key, (content.clone(), entity.clone()));
                entity
            }
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
                .render_session_agent_thinking(index, message, window, cx)
                .into_any_element();
        }
        if message.role == SessionAgentMessageRole::ToolCall {
            return self
                .render_session_agent_tool_call(index, message, entity)
                .into_any_element();
        }
        if message.role == SessionAgentMessageRole::Assistant {
            return div()
                .w(px(SESSION_AGENT_MESSAGE_COLUMN_WIDTH))
                .max_w(px(SESSION_AGENT_MESSAGE_COLUMN_WIDTH))
                .min_w_0()
                .flex_shrink_0()
                .overflow_x_hidden()
                .px_1()
                .py_1()
                .child(self.render_session_agent_markdown(
                    SharedString::from(format!("session-agent-message-{index}-assistant")),
                    index,
                    message.content.clone(),
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
            .w(px(SESSION_AGENT_MESSAGE_COLUMN_WIDTH))
            .max_w(px(SESSION_AGENT_MESSAGE_COLUMN_WIDTH))
            .min_w_0()
            .flex_shrink_0()
            .overflow_x_hidden()
            .when(is_user, |this| this.justify_end())
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(292.0))
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
        index: usize,
        message: &SessionAgentMessage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );

        div()
            .w(px(SESSION_AGENT_MESSAGE_COLUMN_WIDTH))
            .max_w(px(SESSION_AGENT_MESSAGE_COLUMN_WIDTH))
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
                            .gap_2()
                            .items_center()
                            .text_size(miaominal_settings::FontSize::Body.scaled())
                            .text_color(rgb(text_muted))
                            .child("Thinking"),
                    )
                    .child(self.render_session_agent_text(
                        SharedString::from(format!("session-agent-message-{index}-thinking")),
                        index,
                        message.content.clone(),
                        text_muted,
                        window,
                        cx,
                    )),
            )
            .into_any_element()
    }

    fn render_session_agent_tool_call(
        &self,
        index: usize,
        message: &SessionAgentMessage,
        entity: Entity<Self>,
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
        let tool_id = tool_call.id.clone();
        let allow_tool_id = tool_call.id.clone();
        let deny_tool_id = tool_call.id.clone();
        let allow_entity = entity.clone();
        let deny_entity = entity.clone();
        let copy_args_entity = entity.clone();
        let copy_result_entity = entity.clone();
        let copy_arguments = tool_call.arguments.clone();
        let copy_result = tool_call.confirmation_note.clone();

        v_flex()
            .id(("session-agent-tool-call", index))
            .w(px(SESSION_AGENT_MESSAGE_COLUMN_WIDTH))
            .max_w(px(SESSION_AGENT_MESSAGE_COLUMN_WIDTH))
            .min_w_0()
            .flex_shrink_0()
            .overflow_hidden()
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(if needs_confirmation {
                roles.primary
            } else {
                roles.outline_variant
            }))
            .bg(rgb(roles.surface_container_high))
            .child(
                h_flex()
                    .id(("session-agent-tool-call-header", index))
                    .w_full()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(roles.outline_variant))
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
                            .cursor_pointer()
                            .rounded(px(6.0))
                            .px_2()
                            .py_1()
                            .bg(rgb(roles.surface_container_highest))
                            .text_size(miaominal_settings::FontSize::Body.scaled())
                            .text_color(rgb(roles.on_surface))
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                cx.stop_propagation();
                                let text = copy_arguments.clone();
                                copy_args_entity.update(cx, |this, cx| {
                                    this.copy_session_agent_text("tool arguments", text, cx);
                                });
                            })
                            .child("Copy args"),
                    )
                    .when_some(copy_result, |this, result| {
                        this.child(
                            div()
                                .cursor_pointer()
                                .rounded(px(6.0))
                                .px_2()
                                .py_1()
                                .bg(rgb(roles.surface_container_highest))
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .text_color(rgb(roles.on_surface))
                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                    cx.stop_propagation();
                                    let text = result.clone();
                                    copy_result_entity.update(cx, |this, cx| {
                                        this.copy_session_agent_text("tool result", text, cx);
                                    });
                                })
                                .child("Copy result"),
                        )
                    }),
            )
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_size(miaominal_settings::FontSize::Body.scaled())
                    .line_height(miaominal_settings::scaled_line_height(18.0))
                    .text_color(rgb(if expanded {
                        roles.on_surface
                    } else {
                        text_muted
                    }))
                    .when(!expanded, |this| {
                        this.overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(tool_call.summary.clone())
                    })
                    .when(expanded, |this| this.child(tool_call.summary.clone())),
            )
            .when(expanded, |this| {
                this.when_some(tool_call.confirmation_note.clone(), |this, note| {
                    this.child(
                        div()
                            .px_3()
                            .pb_2()
                            .text_size(miaominal_settings::FontSize::Body.scaled())
                            .line_height(miaominal_settings::scaled_line_height(18.0))
                            .text_color(rgb(text_muted))
                            .child(note),
                    )
                })
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
