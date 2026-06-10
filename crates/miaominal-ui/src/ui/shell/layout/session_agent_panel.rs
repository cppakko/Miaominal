use super::super::*;
use crate::ui::i18n;
use gpui_component::text::{TextView, TextViewStyle};

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
                                .child(self.render_session_agent_messages(window, cx)),
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
                                    AppIcon::ChevronUp,
                                    26.0,
                                    8.0,
                                    Some(if waiting {
                                        roles.surface_container_highest
                                    } else {
                                        roles.primary
                                    }),
                                    Some(if waiting {
                                        text_muted
                                    } else {
                                        roles.on_primary
                                    }),
                                    None,
                                    move |window, cx| {
                                        let entity = send_entity.clone();
                                        entity.update(cx, |this, cx| {
                                            this.submit_session_agent_prompt(window, cx);
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
                        self.render_session_agent_message(index, message, window, cx)
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
        content: String,
        color: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;

        let _ = (window, cx);

        TextView::markdown(id, content)
            .selectable(true)
            .style(
                TextViewStyle::default()
                    .paragraph_gap(gpui::rems(0.45))
                    .heading_font_size(|level, base| match level {
                        1 => base * 1.35,
                        2 => base * 1.2,
                        3 => base * 1.1,
                        _ => base,
                    })
                    .code_block(
                        gpui::StyleRefinement::default()
                            .bg(rgb(roles.surface_container_high))
                            .rounded(px(6.0))
                            .p_2()
                            .text_size(miaominal_settings::FontSize::Body.scaled()),
                    ),
            )
            .w_full()
            .min_w_0()
            .overflow_x_hidden()
            .text_size(miaominal_settings::FontSize::Input.scaled())
            .line_height(miaominal_settings::scaled_line_height(20.0))
            .text_color(rgb(color))
            .into_any_element()
    }

    fn render_session_agent_message(
        &self,
        index: usize,
        message: &SessionAgentMessage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let is_user = message.role == SessionAgentMessageRole::User;
        let is_error = message.role == SessionAgentMessageRole::Error;
        if message.role == SessionAgentMessageRole::Thinking {
            return self
                .render_session_agent_thinking(index, message, window, cx)
                .into_any_element();
        }
        if message.role == SessionAgentMessageRole::ToolCall {
            return self
                .render_session_agent_tool_call(index, message)
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
                    .child(self.render_session_agent_markdown(
                        SharedString::from(format!("session-agent-message-{index}-bubble")),
                        if is_user {
                            escape_markdown_text(&message.content)
                        } else {
                            message.content.clone()
                        },
                        fg,
                        window,
                        cx,
                    )),
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
                    .child(self.render_session_agent_markdown(
                        SharedString::from(format!("session-agent-message-{index}-thinking")),
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
                    .w_full()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(roles.outline_variant))
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
                    ),
            )
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_size(miaominal_settings::FontSize::Input.scaled())
                    .line_height(miaominal_settings::scaled_line_height(20.0))
                    .text_color(rgb(roles.on_surface))
                    .child(tool_call.summary.clone()),
            )
            .when_some(tool_call.confirmation_note.clone(), |this, note| {
                this.child(
                    div()
                        .px_3()
                        .pb_2()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .text_color(rgb(text_muted))
                        .child(note),
                )
            })
            .when(needs_confirmation, |this| {
                this.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .px_3()
                        .pb_3()
                        .child(
                            div()
                                .rounded(px(6.0))
                                .px_3()
                                .py_1()
                                .bg(rgb(roles.primary))
                                .text_color(rgb(roles.on_primary))
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .child("Allow"),
                        )
                        .child(
                            div()
                                .rounded(px(6.0))
                                .px_3()
                                .py_1()
                                .bg(rgb(roles.surface_container_highest))
                                .text_color(rgb(roles.on_surface))
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .child("Deny"),
                        ),
                )
            })
            .into_any_element()
    }
}

fn escape_markdown_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' | '`' | '*' | '_' | '{' | '}' | '[' | ']' | '(' | ')' | '#' | '+' | '-' | '.'
            | '!' | '|' | '>' | '~' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}
