use super::super::*;
use super::session_agent_composer;
use super::session_agent_conversation;
use super::session_agent_history;
use crate::ui::components::icon_button_with_tooltip;
use crate::ui::i18n;
use gpui::AnimationExt as _;
use std::time::Duration;

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
const SESSION_AGENT_TEXT_DRAG_THRESHOLD: f32 = 3.0;

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
    controller: Entity<AgentController>,
    is_dragging: bool,
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
            move |event: &MouseDownEvent, _window, cx| {
                gpui_component::GlobalState::suppress_text_selection(cx);
                controller.update(cx, |controller, cx| {
                    let initial_width = controller.panel_width();
                    controller.set_panel_drag(Some(SessionAgentPanelDragState {
                        initial_pointer: f32::from(event.position.x),
                        initial_width,
                    }));
                    cx.notify();
                });
                cx.stop_propagation();
            },
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

impl AgentController {
    fn handle_session_agent_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bindings = &miaominal_settings::current_settings().key_bindings;
        let is_search = bindings.search.matches_keystroke(&event.keystroke);
        let is_escape = event.keystroke.key == "escape";

        if is_search {
            cx.stop_propagation();
            let panel_view = self.session_agent().panel_view;
            if panel_view == ChatPanelView::SessionList {
                self.open_session_filter(window, cx);
            } else {
                self.open_conversation_search(window, cx);
            }
            return;
        }

        if is_escape {
            cx.stop_propagation();
            if self.session_filter_open() {
                self.close_session_filter(cx);
            } else if self.conversation_search_open() {
                self.close_conversation_search(cx);
            }
        }
    }

    fn start_session_agent_auto_scroll(&mut self, pointer_y: f32, cx: &mut Context<Self>) {
        let generation = self.start_auto_scroll(pointer_y);

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
        if self.update_auto_scroll_pointer(pointer_y) {
            cx.notify();
        }
    }

    fn stop_session_agent_auto_scroll(&mut self, cx: &mut Context<Self>) {
        if self.stop_auto_scroll() {
            cx.notify();
        }
    }

    fn begin_session_agent_text_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.finish_text_drag(cx);
        let conversation = self.session_agent().conversation_view.as_ref().cloned();
        if let Some(conversation) = conversation.as_ref() {
            conversation.update(cx, |view, _cx| view.begin_selection_drag());
        }
        self.begin_text_drag(position, conversation);
    }

    fn update_session_agent_text_drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if event.pressed_button != Some(MouseButton::Left) {
            if self.text_drag_active() {
                self.finish_text_drag(cx);
            }
            return;
        }
        if self.text_drag_paused_tail() {
            return;
        }
        let Some(origin) = self.text_drag_origin() else {
            return;
        };
        let delta_x = f32::from(event.position.x - origin.x);
        let delta_y = f32::from(event.position.y - origin.y);
        if delta_x * delta_x + delta_y * delta_y
            < SESSION_AGENT_TEXT_DRAG_THRESHOLD * SESSION_AGENT_TEXT_DRAG_THRESHOLD
        {
            return;
        }

        let Some(conversation) = self.text_drag_conversation() else {
            return;
        };
        if conversation.read(cx).pause_tail_following() {
            self.set_text_drag_paused_tail(true);
            cx.notify();
        }
    }

    fn tick_session_agent_auto_scroll(&mut self, generation: u64, cx: &mut Context<Self>) -> bool {
        if !self.panel_open() || self.session_agent().panel_view != ChatPanelView::Conversation {
            self.stop_session_agent_auto_scroll(cx);
            return false;
        }

        let Some(auto_scroll) = self.auto_scroll() else {
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
        let conversation = self.session_agent().conversation_view.as_ref().cloned();
        if step.abs() >= 0.1
            && let Some(conversation) = conversation
        {
            conversation.read(cx).scroll_by(px(step));
            cx.notify();
        }

        true
    }

    fn render_session_agent_sidebar_toolbar(
        &self,
        entity: Entity<Self>,
        _window: &mut Window,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let icon_bg = roles.surface_container;
        let is_conversation = self.session_agent().panel_view != ChatPanelView::SessionList;
        let is_search_open = if is_conversation {
            self.conversation_search_open()
        } else {
            self.session_filter_open()
        };
        let search_tooltip = i18n::string(if is_conversation {
            if is_search_open {
                "workspace.panel.agent.tooltips.close_conversation_search"
            } else {
                "workspace.panel.agent.tooltips.search_conversation"
            }
        } else if is_search_open {
            "workspace.panel.agent.tooltips.close_history_search"
        } else {
            "workspace.panel.agent.tooltips.search_history"
        });
        let back_entity = entity.clone();
        let new_chat_entity = entity.clone();
        let search_controller = entity.clone();
        let close_entity = entity.clone();
        let edit_entity = entity.clone();
        let editing = is_conversation && self.editing_title();
        let title_input = self.title_input();
        let title_input_for_edit = title_input.clone();
        let display_text = if is_conversation {
            self.session_agent()
                .title
                .clone()
                .unwrap_or_else(|| i18n::string("workspace.panel.agent.sidebar_title"))
        } else {
            i18n::string("workspace.panel.agent.chat")
        };

        h_flex()
            .w_full()
            .h(px(30.0))
            .items_center()
            .gap_1()
            .when(is_conversation, |this| {
                this.child(div().id("session-agent-back-to-history").child(
                    icon_button_with_tooltip(
                        AppIcon::CornerLeftUp,
                        i18n::string("workspace.panel.agent.tooltips.back_to_history"),
                        26.0,
                        8.0,
                        Some(icon_bg),
                        Some(text_muted),
                        None,
                        move |_window, cx| {
                            let entity = back_entity.clone();
                            entity.update(cx, |this, cx| {
                                this.finish_text_drag(cx);
                                this.show_chat_history(cx);
                            });
                        },
                    ),
                ))
            })
            .child(
                div()
                    .id("session-agent-new-chat")
                    .child(icon_button_with_tooltip(
                        AppIcon::Plus,
                        i18n::string("workspace.panel.agent.tooltips.new_chat"),
                        26.0,
                        8.0,
                        Some(icon_bg),
                        Some(text_muted),
                        None,
                        move |window, cx| {
                            let entity = new_chat_entity.clone();
                            entity.update(cx, |this, cx| {
                                this.finish_text_drag(cx);
                                this.start_new_conversation(window, cx);
                            });
                        },
                    )),
            )
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
                                .child(HintedInput::new(&title_input).appearance(false).w_full()),
                        )
                    })
                    .when(!editing, move |this| {
                        let click_entity = edit_entity.clone();
                        let edit_title_input = title_input_for_edit.clone();
                        let text = display_text.clone();
                        this.child(
                            div()
                                .id("session-agent-title")
                                .overflow_x_hidden()
                                .text_ellipsis()
                                .when(is_conversation, move |this| {
                                    this.cursor_text().on_click(move |_click, window, cx| {
                                        click_entity.update(cx, |this, cx| {
                                            let current_title = this
                                                .session_agent()
                                                .title
                                                .clone()
                                                .unwrap_or_default();
                                            set_input_value(
                                                &edit_title_input,
                                                current_title,
                                                window,
                                                cx,
                                            );
                                            this.set_editing_title(true, cx);
                                        });
                                    })
                                })
                                .child(text.clone()),
                        )
                    }),
            )
            .child(
                div()
                    .id("session-agent-search")
                    .child(icon_button_with_tooltip(
                        AppIcon::Search,
                        search_tooltip,
                        26.0,
                        8.0,
                        Some(icon_bg),
                        Some(if is_search_open {
                            roles.primary
                        } else {
                            text_muted
                        }),
                        None,
                        move |window, cx| {
                            let controller = search_controller.clone();
                            controller.update(cx, |controller, cx| {
                                let panel_view = { controller.session_agent().panel_view };
                                if panel_view == ChatPanelView::SessionList {
                                    if controller.session_filter_open() {
                                        controller.close_session_filter(cx);
                                    } else {
                                        controller.open_session_filter(window, cx);
                                    }
                                } else if controller.conversation_search_open() {
                                    controller.close_conversation_search(cx);
                                } else {
                                    controller.open_conversation_search(window, cx);
                                }
                            });
                        },
                    )),
            )
            .child(
                div()
                    .id("session-agent-close")
                    .child(icon_button_with_tooltip(
                        AppIcon::PanelRight,
                        i18n::string("workspace.panel.agent.tooltips.close_panel"),
                        26.0,
                        8.0,
                        Some(icon_bg),
                        Some(text_muted),
                        None,
                        move |_window, cx| {
                            let entity = close_entity.clone();
                            entity.update(cx, |this, cx| {
                                this.finish_text_drag(cx);
                                this.set_panel_open(false);
                                cx.notify();
                            });
                        },
                    )),
            )
            .into_any_element()
    }

    pub(in crate::ui::shell::layout) fn render_session_agent_sidebar(
        &mut self,
        entity: Entity<Self>,
        settings: Entity<SettingsController>,
        terminal_originated_selection_drag_active: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let panel_width = clamp_session_agent_panel_width(self.panel_width());
        let is_dragging = self.panel_drag().is_some();

        card_surface(roles.surface_container, 16.0)
            .id("session-agent-sidebar")
            .relative()
            .w(px(panel_width))
            .h_full()
            .flex_shrink_0()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .child(render_session_agent_resize_handle(
                entity.clone(),
                is_dragging,
            ))
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
                            .child(
                                self.render_session_agent_sidebar_toolbar(entity.clone(), window),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.0))
                            .child(self.render_session_agent_panel(
                                entity,
                                settings,
                                terminal_originated_selection_drag_active,
                                window,
                                cx,
                            )),
                    ),
            )
            .into_any_element()
    }

    pub(in crate::ui::shell::layout) fn render_session_agent_panel(
        &mut self,
        entity: Entity<Self>,
        settings: Entity<SettingsController>,
        terminal_originated_selection_drag_active: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if self.session_agent().panel_view == ChatPanelView::SessionList {
            return self.render_session_agent_history_panel(entity, settings, window, cx);
        }

        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let message_column_width = session_agent_message_column_width(self.panel_width());
        let show_scrollable_messages =
            !self.session_agent().messages.is_empty() || self.session_agent().is_busy();
        let conversation = self.ensure_panel_conversation_view(cx);
        let conversation_list_state = conversation.read(cx).list_state();

        // Chat search state
        let search_input_entity = self.conversation_search_input();
        let search_match_count = self.conversation_search_match_count();
        let search_current_match = self.conversation_search_current_match();
        let search_status = self.conversation_search_status();
        let search_visibility = self.advance_conversation_search_bar(window);

        let close_search_entity = entity.clone();
        let next_entity = entity.clone();
        let prev_entity = entity.clone();

        div()
            .id("session-agent-panel-content")
            .size_full()
            .relative()
            .track_focus(&self.focus())
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
                            // Search overlay bar
                            .when_some(search_visibility, {
                                let search_input = search_input_entity.clone();
                                let close_ent = close_search_entity.clone();
                                let next_ent = next_entity.clone();
                                let prev_ent = prev_entity.clone();
                                let match_count = search_match_count;
                                let current_match = search_current_match;
                                let status_text = search_status.clone();
                                move |this, visibility| {
                                    this.child(
                                        v_flex()
                                            .w_full()
                                            .gap_1()
                                            .py_1()
                                            .opacity(visibility)
                                            .top(px((1.0 - visibility) * 8.0))
                                            .child(search_filter_input(
                                                &search_input.clone(),
                                                SearchInputStyle::Compact,
                                                {
                                                    // Suffix: match counter + prev/next + close
                                                    let close_ent = close_ent.clone();
                                                    let next_ent = next_ent.clone();
                                                    let prev_ent = prev_ent.clone();
                                                    let match_info = if let Some(ref st) = status_text {
                                                        st.clone()
                                                    } else if match_count > 0 {
                                                        format!("{}/{}", current_match.map_or(1, |c| c + 1), match_count)
                                                    } else {
                                                        String::new()
                                                    };
                                                    Some(
                                                        h_flex()
                                                            .gap_1()
                                                            .items_center()
                                                            .child(
                                                                div()
                                                                    .text_size(miaominal_settings::FontSize::Body.scaled())
                                                                    .text_color(rgb(text_muted))
                                                                    .child(match_info),
                                                            )
                                                            .child(icon_button_with_tooltip(
                                                                AppIcon::ChevronUp,
                                                                i18n::string("workspace.panel.agent.tooltips.previous_search_match"),
                                                                16.0,
                                                                4.0,
                                                                Some(roles.surface_container_high),
                                                                Some(text_muted),
                                                                None,
                                                                move |_window, cx| {
                                                                    prev_ent.update(cx, |controller, cx| {
                                                                        controller.navigate_conversation_search_prev(cx);
                                                                    });
                                                                },
                                                            ))
                                                            .child(icon_button_with_tooltip(
                                                                AppIcon::ChevronDown,
                                                                i18n::string("workspace.panel.agent.tooltips.next_search_match"),
                                                                16.0,
                                                                4.0,
                                                                Some(roles.surface_container_high),
                                                                Some(text_muted),
                                                                None,
                                                                move |_window, cx| {
                                                                    next_ent.update(cx, |controller, cx| {
                                                                        controller.navigate_conversation_search_next(cx);
                                                                    });
                                                                },
                                                            ))
                                                            .child(icon_button_with_tooltip(
                                                                AppIcon::PanelRight,
                                                                i18n::string("workspace.panel.agent.tooltips.close_search"),
                                                                16.0,
                                                                4.0,
                                                                Some(roles.surface_container_high),
                                                                Some(text_muted),
                                                                None,
                                                                move |_window, cx| {
                                                                    close_ent.update(cx, |controller, cx| {
                                                                        controller.close_conversation_search(cx);
                                                                    });
                                                                },
                                                            ))
                                                            .into_any_element(),
                                                    )
                                                },
                                            )),
                                    )
                                }
                            })
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
                                            .capture_any_mouse_down(cx.listener(
                                                 move |this, event: &MouseDownEvent, _window, cx| {
                                                    if event.button == MouseButton::Left {
                                                        this.begin_session_agent_text_drag(
                                                            event.position,
                                                            cx,
                                                        );
                                                    }
                                                    if this.auto_scroll().is_some() {
                                                        this.stop_session_agent_auto_scroll(cx);
                                                        cx.stop_propagation();
                                                    } else if event.button != MouseButton::Middle {
                                                        this.stop_session_agent_auto_scroll(cx);
                                                    }
                                                },
                                            ))
                                            .on_mouse_down(
                                                MouseButton::Middle,
                                                cx.listener(
                                                    move |this,
                                                          event: &MouseDownEvent,
                                                          _window,
                                                          cx| {
                                                        if this.auto_scroll().is_none() {
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
                                                        this.update_session_agent_text_drag(
                                                            event, cx,
                                                        );
                                                      this.update_session_agent_auto_scroll_pointer(
                                                          f32::from(event.position.y),
                                                          cx,
                                                      );
                                                  },
                                              ))
                                            .child(self.render_session_agent_messages(
                                                entity.clone(),
                                                message_column_width,
                                                terminal_originated_selection_drag_active,
                                                window,
                                                cx,
                                            ))
                                            .when(self.auto_scroll().is_some(), |this| {
                                                this.child(render_session_agent_auto_scroll_cursor_layer())
                                            }),
                                    )
                                    .vertical_scrollbar(&conversation_list_state)
                                    .into_any_element()
                            } else {
                                div()
                                    .id("session-agent-empty")
                                    .size_full()
                                    .overflow_hidden()
                                    .child(self.render_session_agent_messages(
                                        entity.clone(),
                                        message_column_width,
                                        terminal_originated_selection_drag_active,
                                        window,
                                        cx,
                                    ))
                                    .into_any_element()
                            })),
                    )
                    .child(self.render_session_agent_composer(
                        entity,
                        settings,
                        cx,
                    )),
            )
            .with_animation(
                "session-agent-conversation-view",
                container_transition_animation(),
                |element, delta| element.opacity(delta).top(px((1.0 - delta) * 8.0)),
            )
            .into_any_element()
    }

    fn render_session_agent_composer(
        &self,
        entity: Entity<Self>,
        settings: Entity<SettingsController>,
        cx: &App,
    ) -> gpui::AnyElement {
        session_agent_composer::render_session_agent_composer(self, entity, settings, cx)
    }

    fn render_session_agent_history_panel(
        &mut self,
        entity: Entity<Self>,
        settings: Entity<SettingsController>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let search_visibility = self.advance_session_filter_bar(window);
        session_agent_history::render_session_agent_history_panel(
            self,
            entity,
            settings,
            window,
            cx,
            search_visibility,
        )
    }

    fn render_session_agent_messages(
        &mut self,
        entity: Entity<Self>,
        message_column_width: f32,
        terminal_originated_selection_drag_active: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        session_agent_conversation::render_session_agent_messages(
            self,
            message_column_width,
            terminal_originated_selection_drag_active,
            entity,
            window,
            cx,
        )
    }
}
