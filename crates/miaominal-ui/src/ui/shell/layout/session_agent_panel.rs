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
        fallback_label: String,
        fallback_text: String,
        selected_text: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(selected_text) = selected_text {
            self.copy_session_agent_text(
                i18n::string("workspace.panel.agent.labels.selection"),
                selected_text,
                cx,
            );
        } else {
            self.copy_session_agent_text(fallback_label, fallback_text, cx);
        }
    }

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
            if self.session_agent.panel_view == ChatPanelView::SessionList {
                self.open_session_filter(window, cx);
            } else {
                self.open_conversation_search(window, cx);
            }
            return;
        }

        if is_escape {
            cx.stop_propagation();
            let forms = &self.workspace_forms.chat_search;
            if forms.session_filter_open {
                self.close_session_filter(cx);
            } else if forms.conversation_search_open {
                self.close_conversation_search(cx);
            }
        }
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

    fn begin_session_agent_text_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.finish_session_agent_text_drag(cx);
        self.workspace_state.session_agent_text_drag_origin = Some(position);
        if let Some(conversation) = self.session_agent.conversation_view.as_ref().cloned() {
            conversation.update(cx, |view, _cx| view.begin_selection_drag());
            self.workspace_state.session_agent_text_drag_conversation = Some(conversation);
        }
    }

    fn update_session_agent_text_drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if event.pressed_button != Some(MouseButton::Left) {
            if self
                .workspace_state
                .session_agent_text_drag_origin
                .is_some()
                || self
                    .workspace_state
                    .session_agent_text_drag_conversation
                    .is_some()
            {
                self.finish_session_agent_text_drag(cx);
            }
            return;
        }
        if self.workspace_state.session_agent_text_drag_paused_tail {
            return;
        }
        let Some(origin) = self.workspace_state.session_agent_text_drag_origin else {
            return;
        };
        let delta_x = f32::from(event.position.x - origin.x);
        let delta_y = f32::from(event.position.y - origin.y);
        if delta_x * delta_x + delta_y * delta_y
            < SESSION_AGENT_TEXT_DRAG_THRESHOLD * SESSION_AGENT_TEXT_DRAG_THRESHOLD
        {
            return;
        }

        let Some(conversation) = self
            .workspace_state
            .session_agent_text_drag_conversation
            .as_ref()
            .cloned()
        else {
            return;
        };
        if conversation.read(cx).pause_tail_following() {
            self.workspace_state.session_agent_text_drag_paused_tail = true;
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn finish_session_agent_text_drag(&mut self, cx: &mut Context<Self>) {
        self.workspace_state.session_agent_text_drag_origin = None;
        let Some(conversation) = self
            .workspace_state
            .session_agent_text_drag_conversation
            .take()
        else {
            self.workspace_state.session_agent_text_drag_paused_tail = false;
            return;
        };
        conversation.update(cx, |view, cx| view.finish_selection_drag(cx));
        if std::mem::take(&mut self.workspace_state.session_agent_text_drag_paused_tail) {
            conversation.read(cx).restore_tail_follow_mode();
        }
        cx.notify();
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
        if step.abs() >= 0.1
            && let Some(conversation) = self.session_agent.conversation_view.as_ref()
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
                            let entity = entity.clone();
                            entity.update(cx, |this, cx| {
                                this.start_session_agent_conversation(window, cx);
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
                                this.finish_session_agent_text_drag(cx);
                                this.panels.session_agent_panel_open = false;
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
        &mut self,
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
        let message_column_width =
            session_agent_message_column_width(self.workspace_state.session_agent_panel_width);
        let show_scrollable_messages =
            !self.session_agent.messages.is_empty() || self.session_agent.is_busy();
        let conversation = self.ensure_session_agent_conversation_view(cx);
        let conversation_list_state = conversation.read(cx).list_state();

        // Chat search state
        let is_search_open = self.workspace_forms.chat_search.conversation_search_open;
        let search_input_entity = self
            .workspace_forms
            .chat_search
            .conversation_search_input
            .clone();
        let search_match_count = self.workspace_forms.chat_search.match_count;
        let search_current_match = self.workspace_forms.chat_search.current_match;
        let search_status = self.workspace_forms.chat_search.status.clone();
        let search_visibility = self.advance_conversation_search_bar(window);
        let agent_title_input = self.workspace_forms.agent.title_input.clone();

        let search_button_entity = entity.clone();
        let close_search_entity = entity.clone();
        let next_entity = entity.clone();
        let prev_entity = entity.clone();

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
                                    .child(icon_button_with_tooltip(
                                        AppIcon::CornerLeftUp,
                                        i18n::string("workspace.panel.agent.tooltips.back_to_history"),
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
                                                {
                                                    let title_input = agent_title_input.clone();
                                                move |this| {
                                                    this.child(
                                                        div().flex_1().child(
                                                            HintedInput::new(&title_input)
                                                            .appearance(false)
                                                            .w_full(),
                                                        ),
                                                    )
                                                }
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
                                    )
                                    // Search toggle button
                                    .child(icon_button_with_tooltip(
                                        AppIcon::Search,
                                        i18n::string(if is_search_open {
                                            "workspace.panel.agent.tooltips.close_conversation_search"
                                        } else {
                                            "workspace.panel.agent.tooltips.search_conversation"
                                        }),
                                        24.0,
                                        8.0,
                                        Some(roles.surface_container_high),
                                        Some(if is_search_open { roles.primary } else { text_muted }),
                                        None,
                                        {
                                            let entity = search_button_entity.clone();
                                            move |window, cx| {
                                                entity.update(cx, |this, cx| {
                                                    if this.workspace_forms.chat_search.conversation_search_open {
                                                        this.close_conversation_search(cx);
                                                    } else {
                                                        this.open_conversation_search(window, cx);
                                                    }
                                                });
                                            }
                                        },
                                    )),
                            )
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
                                                                    prev_ent.update(cx, |this, cx| {
                                                                        this.navigate_conversation_search_prev(cx);
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
                                                                    next_ent.update(cx, |this, cx| {
                                                                        this.navigate_conversation_search_next(cx);
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
                                                                    close_ent.update(cx, |this, cx| {
                                                                        this.close_conversation_search(cx);
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
                                    .vertical_scrollbar(&conversation_list_state)
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
            .with_animation(
                "session-agent-conversation-view",
                container_transition_animation(),
                |element, delta| element.opacity(delta).top(px((1.0 - delta) * 8.0)),
            )
            .into_any_element()
    }

    fn render_session_agent_composer(&self, entity: Entity<Self>) -> gpui::AnyElement {
        session_agent_composer::render_session_agent_composer(self, entity)
    }

    fn render_session_agent_history_panel(
        &mut self,
        entity: Entity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let search_visibility = self.advance_session_filter_bar(window);
        session_agent_history::render_session_agent_history_panel(
            self,
            entity,
            window,
            cx,
            search_visibility,
        )
    }

    fn render_session_agent_messages(
        &mut self,
        message_column_width: f32,
        entity: Entity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        session_agent_conversation::render_session_agent_messages(
            self,
            message_column_width,
            entity,
            window,
            cx,
        )
    }
}

impl AppView {
    fn advance_conversation_search_bar(&mut self, window: &mut Window) -> Option<f32> {
        let forms = &mut self.workspace_forms.chat_search;
        if let Some(animation) = forms.conversation_search_animation {
            let duration_seconds = animation.duration.as_secs_f32();
            if duration_seconds <= f32::EPSILON {
                forms.conversation_search_visibility = animation.to;
                forms.conversation_search_animation = None;
            } else {
                let elapsed = Instant::now().saturating_duration_since(animation.started_at);
                let progress = (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0);
                let eased = progress * progress * (3.0 - 2.0 * progress);
                forms.conversation_search_visibility =
                    animation.from + (animation.to - animation.from) * eased;

                if progress >= 1.0 {
                    forms.conversation_search_visibility = animation.to;
                    forms.conversation_search_animation = None;
                } else {
                    window.request_animation_frame();
                }
            }
        }

        if forms.conversation_search_visibility <= f32::EPSILON && !forms.conversation_search_open {
            forms.conversation_search_visible = false;
            return None;
        }

        if forms.conversation_search_open || forms.conversation_search_visibility > f32::EPSILON {
            forms.conversation_search_visible = true;
            return Some(forms.conversation_search_visibility.clamp(0.0, 1.0));
        }

        forms.conversation_search_visible = false;
        None
    }

    fn advance_session_filter_bar(&mut self, window: &mut Window) -> Option<f32> {
        let forms = &mut self.workspace_forms.chat_search;
        if let Some(animation) = forms.session_filter_animation {
            let duration_seconds = animation.duration.as_secs_f32();
            if duration_seconds <= f32::EPSILON {
                forms.session_filter_visibility = animation.to;
                forms.session_filter_animation = None;
            } else {
                let elapsed = Instant::now().saturating_duration_since(animation.started_at);
                let progress = (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0);
                let eased = progress * progress * (3.0 - 2.0 * progress);
                forms.session_filter_visibility =
                    animation.from + (animation.to - animation.from) * eased;

                if progress >= 1.0 {
                    forms.session_filter_visibility = animation.to;
                    forms.session_filter_animation = None;
                } else {
                    window.request_animation_frame();
                }
            }
        }

        if forms.session_filter_visibility <= f32::EPSILON && !forms.session_filter_open {
            forms.session_filter_visible = false;
            return None;
        }

        if forms.session_filter_open || forms.session_filter_visibility > f32::EPSILON {
            forms.session_filter_visible = true;
            return Some(forms.session_filter_visibility.clamp(0.0, 1.0));
        }

        forms.session_filter_visible = false;
        None
    }
}
