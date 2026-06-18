use super::super::*;
use crate::ui::i18n;
use std::time::Duration;
use super::session_agent_composer;
use super::session_agent_conversation;
use super::session_agent_history;
use super::session_agent_mentions;

const SESSION_AGENT_PANEL_HORIZONTAL_PADDING: f32 = 24.0;
const SESSION_AGENT_PANEL_MIN_WIDTH: f32 = 300.0;
const SESSION_AGENT_PANEL_MAX_WIDTH: f32 = 720.0;
const SESSION_AGENT_PANEL_RESIZE_HANDLE_WIDTH: f32 = 8.0;
const SESSION_AGENT_SCROLLBAR_GUTTER: f32 = 16.0;
pub(in crate::ui::shell::layout) const SESSION_AGENT_USER_BUBBLE_MAX_WIDTH: f32 = 420.0;
const SESSION_AGENT_AUTO_SCROLL_INTERVAL: Duration = Duration::from_millis(16);
const SESSION_AGENT_AUTO_SCROLL_DEAD_ZONE: f32 = 12.0;
const SESSION_AGENT_AUTO_SCROLL_SPEED: f32 = 0.55;
const SESSION_AGENT_AUTO_SCROLL_MAX_STEP: f32 = 72.0;

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
    let reserved_width = SESSION_AGENT_PANEL_HORIZONTAL_PADDING + SESSION_AGENT_SCROLLBAR_GUTTER;
    (panel_width - reserved_width).max(SESSION_AGENT_PANEL_MIN_WIDTH - reserved_width)
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
                .rounded(px(999.0)),
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

fn render_session_agent_auto_scroll_cursor_layer() -> gpui::AnyElement {
    canvas(
        |bounds, window, _cx| window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
        |_bounds, _hitbox, window, _cx| {
            window.set_window_cursor_style(CursorStyle::ResizeUpDown);
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .right_0()
    .bottom_0()
    .into_any_element()
}

impl AppView {
    pub(in crate::ui::shell::layout) fn copy_session_agent_message_or_selection(
        &mut self,
        fallback_label: &str,
        fallback_text: String,
        selected_text: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(selected_text) = selected_text {
            self.copy_session_agent_text("selection", selected_text, cx);
        } else {
            self.copy_session_agent_text(fallback_label, fallback_text, cx);
        }
    }

    fn handle_session_agent_key_down(
        &mut self,
        _event: &KeyDownEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        // Ctrl/Cmd+C is now handled by gpui-component Root's window-level selection.
        // This handler can be removed or used for other session agent shortcuts.
    }

    fn start_session_agent_auto_scroll(&mut self, pointer_y: f32, cx: &mut Context<Self>) {
        self.workspace_state.session_agent_auto_scroll_generation = self
            .workspace_state
            .session_agent_auto_scroll_generation
            .wrapping_add(1);
        let generation = self.workspace_state.session_agent_auto_scroll_generation;
        self.workspace_state.session_agent_auto_scroll = Some(SessionAgentAutoScrollState {
            anchor_y: pointer_y,
            pointer_y,
            generation,
        });

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(SESSION_AGENT_AUTO_SCROLL_INTERVAL)
                    .await;

                let keep_scrolling = this
                    .update(cx, |this, cx| {
                        this.tick_session_agent_auto_scroll(generation, cx)
                    })
                    .unwrap_or(false);

                if !keep_scrolling {
                    break;
                }
            }
        })
        .detach();
    }

    fn update_session_agent_auto_scroll_pointer(&mut self, pointer_y: f32, cx: &mut Context<Self>) {
        if let Some(auto_scroll) = self.workspace_state.session_agent_auto_scroll.as_mut() {
            auto_scroll.pointer_y = pointer_y;
            cx.notify();
        }
    }

    fn stop_session_agent_auto_scroll(&mut self, cx: &mut Context<Self>) {
        if self
            .workspace_state
            .session_agent_auto_scroll
            .take()
            .is_some()
        {
            self.workspace_state.session_agent_auto_scroll_generation = self
                .workspace_state
                .session_agent_auto_scroll_generation
                .wrapping_add(1);
            cx.notify();
        }
    }

    fn tick_session_agent_auto_scroll(&mut self, generation: u64, cx: &mut Context<Self>) -> bool {
        if !self.panels.session_agent_panel_open
            || self.session_agent.panel_view != ChatPanelView::Conversation
        {
            self.stop_session_agent_auto_scroll(cx);
            return false;
        }

        let Some(auto_scroll) = self.workspace_state.session_agent_auto_scroll.as_ref() else {
            return false;
        };
        if auto_scroll.generation != generation {
            return false;
        }

        let distance = auto_scroll.pointer_y - auto_scroll.anchor_y;
        let active_distance = distance.abs() - SESSION_AGENT_AUTO_SCROLL_DEAD_ZONE;
        if active_distance <= 0.0 {
            return true;
        }

        let step = (active_distance * SESSION_AGENT_AUTO_SCROLL_SPEED)
            .min(SESSION_AGENT_AUTO_SCROLL_MAX_STEP)
            * distance.signum();
        let scroll_handle = &self.workspace_state.session_agent_scroll_handle;
        let current_offset = scroll_handle.offset();
        let max_offset = scroll_handle.max_offset();
        let next_y = (f32::from(current_offset.y) - step).clamp(-f32::from(max_offset.y), 0.0);

        if (next_y - f32::from(current_offset.y)).abs() >= 0.1 {
            scroll_handle.set_offset(Point::new(current_offset.x, px(next_y)));
            cx.notify();
        }

        true
    }

    fn render_session_agent_sidebar_toolbar(
        &self,
        entity: Entity<Self>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let icon_bg = roles.surface_container;
        let close_entity = entity.clone();
        let edit_entity = entity.clone();
        let editing = self.workspace_forms.agent.editing_title;
        let title_input = self.workspace_forms.agent.title_input.clone();
        let display_text = self
            .session_agent
            .title
            .clone()
            .unwrap_or_else(|| i18n::string("workspace.panel.agent.sidebar_title"));

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
                        this.start_session_agent_conversation(window, cx);
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
                    .when(editing, move |this| {
                        this.child(
                            div()
                                .flex_1()
                                .child(Input::new(&title_input).appearance(false).w_full()),
                        )
                    })
                    .when(!editing, move |this| {
                        let click_entity = edit_entity.clone();
                        let text = display_text.clone();
                        this.child(
                            div()
                                .id("session-agent-title")
                                .overflow_x_hidden()
                                .text_ellipsis()
                                .cursor_text()
                                .on_click(move |_click, window, cx| {
                                    click_entity.update(cx, |this, cx| {
                                        let current_title =
                                            this.session_agent.title.clone().unwrap_or_default();
                                        set_input_value(
                                            &this.workspace_forms.agent.title_input,
                                            current_title,
                                            window,
                                            cx,
                                        );
                                        this.workspace_forms.agent.editing_title = true;
                                        cx.notify();
                                    });
                                })
                                .child(text.clone()),
                        )
                    }),
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
                            .child(self.render_session_agent_sidebar_toolbar(
                                entity.clone(),
                                window,
                                cx,
                            )),
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
        if self.session_agent.panel_view == ChatPanelView::SessionList {
            return self.render_session_agent_history_panel(entity, window, cx);
        }

        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let agent_scroll_handle = self.workspace_state.session_agent_scroll_handle.clone();
        let message_column_width =
            session_agent_message_column_width(self.workspace_state.session_agent_panel_width);
        let show_scrollable_messages =
            !self.session_agent.messages.is_empty() || self.session_agent.is_busy();

        div()
            .id("session-agent-panel-content")
            .size_full()
            .relative()
            .track_focus(&self.session_agent_focus)
            .on_key_down(cx.listener(Self::handle_session_agent_key_down))
            .child(
                v_flex()
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
                                h_flex()
                                    .w_full()
                                    .h(px(28.0))
                                    .items_center()
                                    .gap_2()
                                    .child(icon_button(
                                        AppIcon::CornerLeftUp,
                                        24.0,
                                        8.0,
                                        Some(roles.surface_container_high),
                                        Some(text_muted),
                                        None,
                                        {
                                            let entity = entity.clone();
                                            move |_window, cx| {
                                                let entity = entity.clone();
                                                entity.update(cx, |this, cx| {
                                                    this.show_session_agent_history(cx);
                                                });
                                            }
                                        },
                                    ))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .text_size(
                                                miaominal_settings::FontSize::Subheading.scaled(),
                                            )
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(roles.on_surface))
                                            .when(
                                                self.workspace_forms.agent.editing_title,
                                                move |this| {
                                                    this.child(
                                                        div().flex_1().child(
                                                            Input::new(
                                                                &self
                                                                    .workspace_forms
                                                                    .agent
                                                                    .title_input
                                                                    .clone(),
                                                            )
                                                            .appearance(false)
                                                            .w_full(),
                                                        ),
                                                    )
                                                },
                                            )
                                            .when(!self.workspace_forms.agent.editing_title, {
                                                let click_entity = entity.clone();
                                                let display_text = self
                                                    .session_agent
                                                    .title
                                                    .clone()
                                                    .unwrap_or_else(|| {
                                                        i18n::string("workspace.panel.agent.chat")
                                                    });
                                                move |this: Div| {
                                                    this.child(
                                                        div()
                                                            .id("session-agent-conversations-title")
                                                            .overflow_x_hidden()
                                                            .text_ellipsis()
                                                            .cursor_text()
                                                            .on_click(move |_click, window, cx| {
                                                                click_entity.update(
                                                                    cx,
                                                                    |this, cx| {
                                                                        let current_title = this
                                                                            .session_agent
                                                                            .title
                                                                            .clone()
                                                                            .unwrap_or_default();
                                                                        set_input_value(
                                                                            &this
                                                                                .workspace_forms
                                                                                .agent
                                                                                .title_input,
                                                                            current_title,
                                                                            window,
                                                                            cx,
                                                                        );
                                                                        this.workspace_forms
                                                                            .agent
                                                                            .editing_title = true;
                                                                        cx.notify();
                                                                    },
                                                                );
                                                            })
                                                            .child(display_text),
                                                    )
                                                }
                                            }),
                                    ),
                            )
                            .child(div().flex_1().min_h_0().child(if show_scrollable_messages {
                                div()
                                    .relative()
                                    .size_full()
                                    .min_h_0()
                                    .child(
                                        div()
                                            .id("session-agent-scroll")
                                            .size_full()
                                            .overflow_x_hidden()
                                            .track_scroll(&agent_scroll_handle)
                                            .overflow_y_scroll()
                                            .capture_any_mouse_down(cx.listener(
                                                move |this, event: &MouseDownEvent, _window, cx| {
                                                    this.stop_session_agent_follow_bottom(true);
                                                    if this
                                                        .workspace_state
                                                        .session_agent_auto_scroll
                                                        .is_some()
                                                    {
                                                        this.stop_session_agent_auto_scroll(cx);
                                                        cx.stop_propagation();
                                                    } else if event.button != MouseButton::Middle {
                                                        this.stop_session_agent_auto_scroll(cx);
                                                    }
                                                },
                                            ))
                                            .on_scroll_wheel(cx.listener(
                                                move |this,
                                                      event: &ScrollWheelEvent,
                                                      window,
                                                      cx| {
                                                    this.handle_session_agent_scroll_wheel(
                                                        event, window, cx,
                                                    );
                                                },
                                            ))
                                            .on_mouse_down(
                                                MouseButton::Middle,
                                                cx.listener(
                                                    move |this,
                                                          event: &MouseDownEvent,
                                                          _window,
                                                          cx| {
                                                        if this
                                                            .workspace_state
                                                            .session_agent_auto_scroll
                                                            .is_none()
                                                        {
                                                            this.start_session_agent_auto_scroll(
                                                                f32::from(event.position.y),
                                                                cx,
                                                            );
                                                            cx.stop_propagation();
                                                        }
                                                    },
                                                ),
                                            )
                                            .on_mouse_move(cx.listener(
                                                move |this,
                                                      event: &MouseMoveEvent,
                                                      _window,
                                                      cx| {
                                                    this.update_session_agent_auto_scroll_pointer(
                                                        f32::from(event.position.y),
                                                        cx,
                                                    );
                                                },
                                            ))
                                            .pb_2()
                                            .child(self.render_session_agent_messages(
                                                message_column_width,
                                                entity.clone(),
                                                window,
                                                cx,
                                            ))
                                            .when(
                                                self.workspace_state
                                                    .session_agent_auto_scroll
                                                    .is_some(),
                                                |this| {
                                                    this.child(render_session_agent_auto_scroll_cursor_layer())
                                                },
                                            ),
                                    )
                                    .vertical_scrollbar(&agent_scroll_handle)
                                    .into_any_element()
                            } else {
                                div()
                                    .id("session-agent-empty")
                                    .size_full()
                                    .overflow_hidden()
                                    .child(self.render_session_agent_messages(
                                        message_column_width,
                                        entity.clone(),
                                        window,
                                        cx,
                                    ))
                                    .into_any_element()
                            })),
                    )
                    .child(self.render_session_agent_composer(entity.clone())),
            )
            .into_any_element()
    }

    fn render_session_agent_composer(&self, entity: Entity<Self>) -> gpui::AnyElement {
        session_agent_composer::render_session_agent_composer(self, entity)
    }

    fn render_session_agent_history_panel(
        &self,
        entity: Entity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        session_agent_history::render_session_agent_history_panel(self, entity, window, cx)
    }

    /// Renders the @-mention candidate popup as a window-root-level absolute overlay.
    /// Must be called from `Render::render()` so the absolute positioning escapes all
    /// overflow-hidden containers inside the sidebar panel hierarchy.
    pub(in crate::ui::shell) fn render_session_agent_at_mention_overlay(
        &self,
        entity: Entity<Self>,
        query: String,
    ) -> gpui::AnyElement {
        session_agent_mentions::render_session_agent_at_mention_overlay(self, entity, query)
    }

    pub(in crate::ui::shell::layout) fn render_session_agent_target_chips(&self, entity: Entity<Self>) -> gpui::AnyElement {
        session_agent_mentions::render_session_agent_target_chips(self, entity)
    }

    fn render_session_agent_messages(
        &self,
        message_column_width: f32,
        entity: Entity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        session_agent_conversation::render_session_agent_messages(self, message_column_width, entity, window, cx)
    }
}
