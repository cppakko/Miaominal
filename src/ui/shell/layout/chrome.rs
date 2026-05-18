use super::super::*;
use crate::ui::i18n;
use gpui::{StyleRefinement, WindowControlArea, point};
use std::time::{Duration, Instant};

const TOPBAR_TAB_TITLE_CHARS: usize = 20;
const TOPBAR_TAB_GAP: f32 = 8.0;
const TOPBAR_SECTION_GAP: f32 = 12.0;
const TOPBAR_ACTION_BUTTON_WIDTH: f32 = 36.0;
const TOPBAR_HORIZONTAL_PADDING: f32 = 32.0;
const TOPBAR_WINDOW_CONTROLS_WIDTH: f32 = TOPBAR_ACTION_BUTTON_WIDTH * 3.0 + TOPBAR_TAB_GAP * 2.0;
const TOPBAR_TAB_STRIP_INNER_PADDING: f32 = 8.0;
const MACOS_TRAFFIC_LIGHT_PADDING: f32 = 71.0;

fn topbar_window_controls_width(window: &Window) -> f32 {
    if cfg!(target_os = "macos") {
        if window.is_fullscreen() {
            0.0
        } else {
            MACOS_TRAFFIC_LIGHT_PADDING
        }
    } else {
        TOPBAR_WINDOW_CONTROLS_WIDTH
    }
}

fn window_controls_on_left() -> bool {
    cfg!(target_os = "macos")
}

fn show_macos_traffic_light_space(window: &Window) -> bool {
    cfg!(target_os = "macos") && !window.is_fullscreen()
}

#[derive(Clone)]
struct VisibleTopbarTab {
    tab_index: usize,
    snapshot: TopbarTabSnapshot,
}

#[derive(Clone)]
struct ExitingTopbarTabRenderState {
    snapshot: TopbarTabSnapshot,
    visibility: f32,
    active_strength: f32,
}

fn topbar_transition_raw_progress(started_at: Instant, duration: Duration) -> f32 {
    let duration_seconds = duration.as_secs_f32();
    if duration_seconds <= f32::EPSILON {
        return 1.0;
    }

    let elapsed = Instant::now().saturating_duration_since(started_at);
    (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0)
}

fn topbar_transition_progress(started_at: Instant, duration: Duration) -> f32 {
    let progress = topbar_transition_raw_progress(started_at, duration);
    progress * progress * (3.0 - 2.0 * progress)
}

fn blend_rgb(from: u32, to: u32, progress: f32) -> u32 {
    let progress = progress.clamp(0.0, 1.0);
    let mix = |from: u32, to: u32| -> u32 {
        (from as f32 + (to as f32 - from as f32) * progress).round() as u32
    };

    let from_red = (from >> 16) & 0xff;
    let from_green = (from >> 8) & 0xff;
    let from_blue = from & 0xff;
    let to_red = (to >> 16) & 0xff;
    let to_green = (to >> 8) & 0xff;
    let to_blue = to & 0xff;

    (mix(from_red, to_red) << 16) | (mix(from_green, to_green) << 8) | mix(from_blue, to_blue)
}

fn topbar_tab_icon(kind: TopbarTabVisualKind) -> Option<AppIcon> {
    match kind {
        TopbarTabVisualKind::Hosts => None,
        TopbarTabVisualKind::Session => Some(AppIcon::LaptopMinimal),
        TopbarTabVisualKind::Sftp => Some(AppIcon::FolderSymlink),
    }
}

fn tab_is_error_status(tab: &TabState) -> bool {
    match &tab.kind {
        TabKind::Session(session) => matches!(
            session.connection_state,
            SessionConnectionState::Failed { .. } | SessionConnectionState::Disconnected
        ),
        TabKind::Sftp(_) => {
            let error = i18n::string("session.status.error");
            let closed = i18n::string("session.status.closed");
            tab.status == error || tab.status == closed
        }
        TabKind::Hosts => false,
    }
}

fn tab_needs_attention(tab: &TabState) -> bool {
    tab.as_session()
        .is_some_and(|session| session.pending_host_key.is_some())
}

fn tab_is_connected_status(tab: &TabState) -> bool {
    tab.as_session()
        .is_some_and(|session| matches!(session.connection_state, SessionConnectionState::Ready))
}

pub(in crate::ui::shell) fn tab_status_indicator_color(
    tab: &TabState,
    has_activity: bool,
) -> Option<u32> {
    let roles = settings::current_theme().material.roles;

    if tab_is_error_status(tab) {
        Some(roles.error)
    } else if has_activity || tab_needs_attention(tab) {
        Some(roles.primary)
    } else {
        None
    }
}

pub(super) fn tab_status_color(tab: &TabState) -> u32 {
    let roles = settings::current_theme().material.roles;

    if tab_is_error_status(tab) {
        roles.error
    } else if tab_needs_attention(tab) || tab_is_connected_status(tab) {
        roles.primary
    } else {
        roles.on_surface_variant
    }
}

pub(in crate::ui::shell) fn status_indicator(color: u32, size: f32) -> impl IntoElement {
    div()
        .flex_shrink_0()
        .size(px(size))
        .rounded(px(999.0))
        .bg(rgb(color))
}

fn format_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let n_f = n as f64;
    if n_f >= GB {
        format!("{:.1} GB", n_f / GB)
    } else if n_f >= MB {
        format!("{:.1} MB", n_f / MB)
    } else if n_f >= KB {
        format!("{:.1} KB", n_f / KB)
    } else {
        format!("{n} B")
    }
}

fn scroll_topbar_tabs(scroll_handle: &ScrollHandle, event: &ScrollWheelEvent) -> bool {
    let scroll_delta = match event.delta {
        ScrollDelta::Pixels(point) => f32::from(point.x + point.y),
        ScrollDelta::Lines(point) => (point.x + point.y) * 40.0,
    };
    if scroll_delta.abs() < 0.1 {
        return false;
    }

    let current_offset = scroll_handle.offset();
    let max_offset = scroll_handle.max_offset();
    let next_x = (f32::from(current_offset.x) + scroll_delta).clamp(-f32::from(max_offset.x), 0.0);

    if (next_x - f32::from(current_offset.x)).abs() < 0.1 {
        return false;
    }

    scroll_handle.set_offset(point(px(next_x), current_offset.y));
    true
}

fn topbar_add_button(entity: Entity<AppView>, scroll_handle: ScrollHandle) -> impl IntoElement {
    let roles = settings::current_theme().material.roles;
    let scroll_entity = entity.clone();

    div()
        .flex_shrink_0()
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_up(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(
            Button::new(SharedString::from("top-add-host"))
                .ghost()
                .occlude()
                .flex_shrink_0()
                .w(px(TOPBAR_ACTION_BUTTON_WIDTH))
                .h(px(TOPBAR_ACTION_BUTTON_WIDTH))
                .rounded(px(99.0))
                .bg(rgb(roles.surface_container_highest))
                .on_scroll_wheel(move |event: &ScrollWheelEvent, _, cx| {
                    if scroll_topbar_tabs(&scroll_handle, event) {
                        scroll_entity.update(cx, |_, cx| cx.notify());
                        cx.stop_propagation();
                    }
                })
                .child(Icon::from(AppIcon::Plus))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.open_hosts_tab(cx));
                }),
        )
}

fn topbar_drag_region(id: &'static str) -> impl IntoElement {
    div()
        .id(SharedString::from(id))
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |event: &MouseDownEvent, window, cx| {
            if event.click_count == 1 {
                window.start_window_move();
            }
            cx.stop_propagation();
        })
        .on_mouse_up(MouseButton::Left, |event: &MouseUpEvent, window, cx| {
            if event.click_count == 2 {
                if cfg!(target_os = "macos") {
                    window.titlebar_double_click();
                } else {
                    window.zoom_window();
                }
            }
            cx.stop_propagation();
        })
}

fn build_tab_context_menu(
    menu: PopupMenu,
    entity: Entity<AppView>,
    index: usize,
    is_session: bool,
) -> PopupMenu {
    let rename_entity = entity.clone();
    let close_others_entity = entity.clone();
    let duplicate_entity = entity.clone();
    let sftp_entity = entity.clone();
    let close_entity = entity;

    let menu = menu.item(
        PopupMenuItem::new(i18n::string("chrome.menu.rename_tab")).on_click(
            move |_, window, cx| {
                let entity = rename_entity.clone();
                entity.update(cx, |this, cx| this.begin_rename_tab(index, window, cx));
            },
        ),
    );

    let menu = if is_session {
        menu.item(
            PopupMenuItem::new(i18n::string("chrome.menu.duplicate_profile")).on_click(
                move |_, window, cx| {
                    let entity = duplicate_entity.clone();
                    entity.update(cx, |this, cx| this.duplicate_profile_tab(index, window, cx));
                },
            ),
        )
        .item(
            PopupMenuItem::new(i18n::string("chrome.menu.open_sftp_tab")).on_click(
                move |_, window, cx| {
                    let entity = sftp_entity.clone();
                    entity.update(cx, |this, cx| {
                        this.open_sftp_tab_for_session(Some(index), window, cx)
                    });
                },
            ),
        )
    } else {
        menu
    };

    menu.item(PopupMenuItem::separator())
        .item(
            PopupMenuItem::new(i18n::string("chrome.menu.close_other_tabs")).on_click(
                move |_, window, cx| {
                    let entity = close_others_entity.clone();
                    entity.update(cx, |this, cx| this.close_other_tabs(index, window, cx));
                },
            ),
        )
        .item(
            PopupMenuItem::new(i18n::string("chrome.menu.close_tab")).on_click(
                move |_, window, cx| {
                    let entity = close_entity.clone();
                    entity.update(cx, |this, cx| this.close_tab(index, window, cx));
                },
            ),
        )
}

fn close_topbar_tab(entity: &Entity<AppView>, index: usize, window: &mut Window, cx: &mut App) {
    entity.update(cx, |this, cx| this.close_tab(index, window, cx));
}

fn topbar_pointer_up_should_be_ignored(cx: &mut App) -> bool {
    if cx.has_active_drag() {
        cx.stop_propagation();
        true
    } else {
        false
    }
}

fn window_control_button(
    id: &'static str,
    icon: AppIcon,
    control_area: WindowControlArea,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let roles = settings::current_theme().material.roles;
    let is_windows = cfg!(target_os = "windows");

    div()
        .id(SharedString::from(id))
        .size(px(36.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(16.0))
        .bg(rgb(roles.surface_container_highest))
        .text_color(rgb(roles.on_surface))
        .cursor_pointer()
        .active(|this| this.opacity(0.85))
        .occlude()
        .when(is_windows, |this| this.window_control_area(control_area))
        .when(!is_windows, |this| {
            this.on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .on_mouse_up(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
        })
        .child(Icon::from(icon).small())
        .on_click(move |_, window, cx| on_click(window, cx))
}

fn maximize_window_control_button(window: &Window) -> impl IntoElement {
    let is_zoomed = if cfg!(target_os = "macos") {
        window.is_fullscreen()
    } else {
        window.is_maximized()
    };

    let icon = if is_zoomed {
        AppIcon::Restore
    } else {
        AppIcon::Maximize
    };

    window_control_button(
        "window-maximize",
        icon,
        WindowControlArea::Max,
        |window, _| {
            if cfg!(target_os = "macos") {
                window.toggle_fullscreen();
            } else {
                window.zoom_window();
            }
        },
    )
}

fn window_controls_group(window: &Window) -> impl IntoElement {
    if cfg!(target_os = "macos") {
        return div()
            .w(px(topbar_window_controls_width(window)))
            .h_full()
            .flex_shrink_0()
            .into_any_element();
    }

    if window_controls_on_left() {
        h_flex()
            .items_center()
            .gap(px(TOPBAR_TAB_GAP))
            .child(window_control_button(
                "window-close",
                AppIcon::Close,
                WindowControlArea::Close,
                |window, _| {
                    window.remove_window();
                },
            ))
            .child(window_control_button(
                "window-minimize",
                AppIcon::Minimize,
                WindowControlArea::Min,
                |window, _| {
                    window.minimize_window();
                },
            ))
            .child(maximize_window_control_button(window))
            .into_any_element()
    } else {
        h_flex()
            .items_center()
            .gap(px(TOPBAR_TAB_GAP))
            .child(window_control_button(
                "window-minimize",
                AppIcon::Minimize,
                WindowControlArea::Min,
                |window, _| {
                    window.minimize_window();
                },
            ))
            .child(maximize_window_control_button(window))
            .child(window_control_button(
                "window-close",
                AppIcon::Close,
                WindowControlArea::Close,
                |window, _| {
                    window.remove_window();
                },
            ))
            .into_any_element()
    }
}

impl AppView {
    fn collect_visible_topbar_tabs(
        &self,
        current_active_tab_id: Option<usize>,
    ) -> Vec<VisibleTopbarTab> {
        self.workspace_state
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| !tab.hidden_from_topbar)
            .enumerate()
            .map(|(visible_index, (tab_index, tab))| {
                let has_activity = tab.as_session().is_some_and(|session| session.has_activity)
                    && current_active_tab_id != Some(tab.id);
                let kind = if tab.as_sftp().is_some() {
                    TopbarTabVisualKind::Sftp
                } else if tab.as_session().is_some() {
                    TopbarTabVisualKind::Session
                } else {
                    TopbarTabVisualKind::Hosts
                };

                VisibleTopbarTab {
                    tab_index,
                    snapshot: TopbarTabSnapshot {
                        tab_id: tab.id,
                        visible_index,
                        title: tab.title.clone(),
                        kind,
                        status_color: tab_status_indicator_color(tab, has_activity),
                    },
                }
            })
            .collect()
    }

    fn sync_topbar_tab_animation_state(
        &mut self,
        visible_tabs: &[VisibleTopbarTab],
        current_active_tab_id: Option<usize>,
        window: &mut Window,
    ) {
        let now = Instant::now();
        let duration = support::CONTAINER_TRANSITION_DURATION;
        let current_tab_ids: Vec<_> = visible_tabs.iter().map(|tab| tab.snapshot.tab_id).collect();

        for tab in visible_tabs {
            if !self
                .workspace_state
                .topbar_previous_visible_tabs
                .iter()
                .any(|previous| previous.tab_id == tab.snapshot.tab_id)
                && !self
                    .workspace_state
                    .topbar_entering_tabs
                    .iter()
                    .any(|transition| transition.tab_id == tab.snapshot.tab_id)
            {
                self.workspace_state
                    .topbar_entering_tabs
                    .push(TopbarTabEnterTransition {
                        tab_id: tab.snapshot.tab_id,
                        started_at: now,
                        duration,
                    });
            }
        }

        for previous in &self.workspace_state.topbar_previous_visible_tabs {
            if !current_tab_ids.contains(&previous.tab_id)
                && !self
                    .workspace_state
                    .topbar_exiting_tabs
                    .iter()
                    .any(|transition| transition.snapshot.tab_id == previous.tab_id)
            {
                self.workspace_state
                    .topbar_exiting_tabs
                    .push(TopbarTabExitTransition {
                        snapshot: previous.clone(),
                        started_at: now,
                        duration,
                    });
            }
        }

        self.workspace_state.topbar_previous_visible_tabs = visible_tabs
            .iter()
            .map(|tab| tab.snapshot.clone())
            .collect();

        self.workspace_state
            .topbar_entering_tabs
            .retain(|transition| {
                current_tab_ids.contains(&transition.tab_id)
                    && topbar_transition_raw_progress(transition.started_at, transition.duration)
                        < 1.0
            });
        self.workspace_state
            .topbar_exiting_tabs
            .retain(|transition| {
                topbar_transition_raw_progress(transition.started_at, transition.duration) < 1.0
            });

        if self.workspace_state.topbar_visible_active_tab_id != current_active_tab_id {
            self.workspace_state.topbar_active_transition = Some(TopbarActiveTabTransition {
                from_tab_id: self.workspace_state.topbar_visible_active_tab_id,
                to_tab_id: current_active_tab_id,
                started_at: now,
                duration,
            });
            self.workspace_state.topbar_visible_active_tab_id = current_active_tab_id;
        }

        if let Some(transition) = self.workspace_state.topbar_active_transition
            && topbar_transition_raw_progress(transition.started_at, transition.duration) >= 1.0
        {
            self.workspace_state.topbar_active_transition = None;
        }

        if !self.workspace_state.topbar_entering_tabs.is_empty()
            || !self.workspace_state.topbar_exiting_tabs.is_empty()
            || self.workspace_state.topbar_active_transition.is_some()
        {
            window.request_animation_frame();
        }
    }

    fn topbar_tab_visibility(&self, tab_id: usize) -> f32 {
        self.workspace_state
            .topbar_entering_tabs
            .iter()
            .find(|transition| transition.tab_id == tab_id)
            .map(|transition| {
                topbar_transition_progress(transition.started_at, transition.duration)
            })
            .unwrap_or(1.0)
    }

    fn topbar_active_strength(&self, tab_id: usize, current_active_tab_id: Option<usize>) -> f32 {
        let Some(transition) = self.workspace_state.topbar_active_transition else {
            return if current_active_tab_id == Some(tab_id) {
                1.0
            } else {
                0.0
            };
        };

        let progress = topbar_transition_progress(transition.started_at, transition.duration);
        if transition.from_tab_id == Some(tab_id) && transition.to_tab_id == Some(tab_id) {
            1.0
        } else if transition.from_tab_id == Some(tab_id) {
            1.0 - progress
        } else if transition.to_tab_id == Some(tab_id) {
            progress
        } else if current_active_tab_id == Some(tab_id) {
            1.0
        } else {
            0.0
        }
    }

    fn topbar_exiting_tabs_render_state(
        &self,
        current_active_tab_id: Option<usize>,
    ) -> Vec<ExitingTopbarTabRenderState> {
        self.workspace_state
            .topbar_exiting_tabs
            .iter()
            .map(|transition| ExitingTopbarTabRenderState {
                snapshot: transition.snapshot.clone(),
                visibility: 1.0
                    - topbar_transition_progress(transition.started_at, transition.duration),
                active_strength: self
                    .topbar_active_strength(transition.snapshot.tab_id, current_active_tab_id),
            })
            .collect()
    }

    pub(in crate::ui::shell) fn render_top_bar(
        &mut self,
        entity: Entity<Self>,
        window: &mut Window,
    ) -> impl IntoElement {
        let roles = settings::current_theme().material.roles;
        let topbar_scroll_handle = self.workspace_state.topbar_tab_scroll_handle.clone();
        let topbar_scroll_handle_for_tabs = topbar_scroll_handle.clone();
        let current_active_tab_id = self
            .workspace_state
            .active_topbar_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .filter(|tab| !tab.hidden_from_topbar)
            .map(|tab| tab.id);
        let visible_tabs = self.collect_visible_topbar_tabs(current_active_tab_id);
        self.sync_topbar_tab_animation_state(&visible_tabs, current_active_tab_id, window);
        let exiting_tabs = self.topbar_exiting_tabs_render_state(current_active_tab_id);
        let topbar_tab_count = visible_tabs.len();
        let tabs_inline_width = topbar_tab_count as f32 * TOPBAR_TAB_WIDTH
            + topbar_tab_count.saturating_sub(1) as f32 * TOPBAR_TAB_GAP;
        let inline_add_button_width = if topbar_tab_count > 0 {
            TOPBAR_TAB_GAP + TOPBAR_ACTION_BUTTON_WIDTH
        } else {
            TOPBAR_ACTION_BUTTON_WIDTH
        };
        let window_controls_width = topbar_window_controls_width(window);
        let window_controls_gap = if window_controls_width > 0.0 {
            TOPBAR_SECTION_GAP
        } else {
            0.0
        };
        let topbar_left_section_width = (f32::from(window.bounds().size.width)
            - TOPBAR_HORIZONTAL_PADDING
            - window_controls_gap
            - window_controls_width)
            .max(0.0);
        let pin_add_button =
            tabs_inline_width + inline_add_button_width + TOPBAR_TAB_STRIP_INNER_PADDING
                > topbar_left_section_width;

        div()
            .relative()
            .h(px(TOP_BAR_HEIGHT))
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .bg(rgb(roles.surface_container))
            .px_2()
            .when(cfg!(target_os = "macos"), |this| {
                this.on_mouse_down(MouseButton::Left, |event: &MouseDownEvent, window, cx| {
                    if event.click_count == 1 {
                        window.start_window_move();
                    }
                    cx.stop_propagation();
                })
                .on_mouse_up(MouseButton::Left, |event: &MouseUpEvent, window, cx| {
                    if event.click_count == 2 {
                        window.titlebar_double_click();
                    }
                    cx.stop_propagation();
                })
            })
            .when(
                cfg!(any(target_os = "linux", target_os = "freebsd")),
                |this| {
                    this.on_mouse_down(MouseButton::Left, |event: &MouseDownEvent, window, cx| {
                        if event.click_count == 1 {
                            window.start_window_move();
                        }
                        cx.stop_propagation();
                    })
                },
            )
            .when(!cfg!(target_os = "macos"), |this| this.child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .left_0()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_up(MouseButton::Left, |event: &MouseUpEvent, window, _| {
                        if event.click_count == 2 {
                            if cfg!(target_os = "macos") {
                                window.titlebar_double_click();
                            } else {
                                window.zoom_window();
                            }
                        }
                    }),
            ))
            .child(
                h_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap(px(TOPBAR_SECTION_GAP))
                    .when(show_macos_traffic_light_space(window), |this| {
                        this.child(window_controls_group(window))
                    })
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w(px(0.0))
                            .items_center()
                            .gap(px(TOPBAR_TAB_GAP))
                            .on_scroll_wheel({
                                let scroll_handle = topbar_scroll_handle.clone();
                                let entity = entity.clone();
                                move |event: &ScrollWheelEvent, _, cx| {
                                    if scroll_topbar_tabs(&scroll_handle, event) {
                                        entity.update(cx, |_, cx| cx.notify());
                                        cx.stop_propagation();
                                    }
                                }
                            })
                            .child(
                                div()
                                    .relative()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .h(px(44.0))
                                    .px_1()
                                    .py(px(4.0))
                                    .rounded(px(24.0))
                                    .overflow_x_hidden()
                                    .child(
                                        h_flex()
                                            .id("top-tabs-strip")
                                            .relative()
                                            .w_full()
                                            .min_w(px(0.0))
                                            .h_full()
                                            .items_center()
                                            .gap(px(TOPBAR_TAB_GAP))
                                            .overflow_x_scroll()
                                            .track_scroll(&topbar_scroll_handle)
                                            .children(visible_tabs.iter().map({
                                                let entity = entity.clone();
                                                let topbar_scroll_handle = topbar_scroll_handle_for_tabs.clone();
                                                move |visible_tab| {
                                                    let index = visible_tab.tab_index;
                                                    let tab = &self.workspace_state.tabs[index];
                                                    let snapshot = &visible_tab.snapshot;
                                                    let activate_entity = entity.clone();
                                                    let middle_close_entity = entity.clone();
                                                    let content_middle_close_entity = entity.clone();
                                                    let close_entity = entity.clone();
                                                    let close_middle_entity = entity.clone();
                                                    let menu_entity = entity.clone();
                                                    let drop_entity = entity.clone();
                                                    let scroll_entity = entity.clone();
                                                    let rename_scroll_entity = entity.clone();
                                                    let close_scroll_entity = entity.clone();
                                                    let tab_scroll_handle = topbar_scroll_handle.clone();
                                                    let rename_scroll_handle = topbar_scroll_handle.clone();
                                                    let close_scroll_handle = topbar_scroll_handle.clone();
                                                    let tab_id = snapshot.tab_id;
                                                    let tab_visibility = self.topbar_tab_visibility(tab_id);
                                                    let active_strength = self.topbar_active_strength(
                                                        tab_id,
                                                        current_active_tab_id,
                                                    );
                                                    let is_active = current_active_tab_id == Some(tab_id);
                                                    let is_session = snapshot.kind == TopbarTabVisualKind::Session;
                                                    let tab_kind_icon = topbar_tab_icon(snapshot.kind);
                                                    let is_renaming = self.workspace_state.renaming_tab == Some(index);
                                                    let status_color = snapshot.status_color;
                                                    let tab_foreground_color = blend_rgb(
                                                        roles.on_surface_variant,
                                                        roles.on_secondary_container,
                                                        active_strength,
                                                    );
                                                    let display_title = truncate_with_ellipsis(
                                                        &tab.title,
                                                        TOPBAR_TAB_TITLE_CHARS,
                                                    );
                                                    let tab_background = blend_rgb(
                                                        roles.surface_container_low,
                                                        roles.secondary_container,
                                                        active_strength,
                                                    );
                                                    let tab_hover_background = blend_rgb(
                                                        roles.surface_container_high,
                                                        roles.secondary_container,
                                                        active_strength,
                                                    );
                                                    let drag_payload = DraggedTab {
                                                        source_tab_id: tab_id,
                                                        source_index: index,
                                                        source_pane_id: self.active_pane_id(),
                                                        is_active,
                                                        title: display_title.clone(),
                                                        status_color,
                                                    };

                                                    let title_child: gpui::AnyElement = if is_renaming {
                                                        div()
                                                            .flex_1()
                                                            .min_w(px(0.0))
                                                            .overflow_hidden()
                                                            .on_scroll_wheel(move |event: &ScrollWheelEvent, _, cx| {
                                                                if scroll_topbar_tabs(&rename_scroll_handle, event) {
                                                                    rename_scroll_entity.update(cx, |_, cx| cx.notify());
                                                                    cx.stop_propagation();
                                                                }
                                                            })
                                                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                                cx.stop_propagation();
                                                            })
                                                            .child(
                                                                Input::new(&self.workspace_forms.rename_input)
                                                                    .appearance(false)
                                                                    .border_1()
                                                                    .border_color(rgb(roles.primary))
                                                                    .xsmall()
                                                                    .w_full(),
                                                            )
                                                            .into_any_element()
                                                    } else {
                                                        div()
                                                            .id(SharedString::from(format!(
                                                                "top-tab-title-{tab_id}"
                                                            )))
                                                            .flex_1()
                                                            .min_w(px(0.0))
                                                            .h(px(14.0))
                                                            .overflow_hidden()
                                                            .text_size(settings::scaled_font_size(11.0))
                                                            .line_height(settings::scaled_line_height(14.0))
                                                            .text_color(rgb(tab_foreground_color))
                                                            .child(display_title)
                                                            .into_any_element()
                                                    };
                                                    let animated_tab = div()
                                                        .id(SharedString::from(format!("top-tab-{tab_id}")))
                                                        .w_full()
                                                        .h_full()
                                                        .p_1()
                                                        .rounded(px(20.0))
                                                        .bg(rgb(tab_background))
                                                        .occlude()
                                                        .cursor_pointer()
                                                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                            cx.stop_propagation();
                                                        })
                                                        .on_mouse_up(MouseButton::Left, |_, _, cx| {
                                                            cx.stop_propagation();
                                                        })
                                                        .active(|this| this.opacity(0.92))
                                                        .on_scroll_wheel(move |event: &ScrollWheelEvent, _, cx| {
                                                            if scroll_topbar_tabs(&tab_scroll_handle, event) {
                                                                scroll_entity.update(cx, |_, cx| cx.notify());
                                                                cx.stop_propagation();
                                                            }
                                                        })
                                                        .on_mouse_down(MouseButton::Middle, |_, _, cx| {
                                                            cx.stop_propagation();
                                                        })
                                                        .on_mouse_up(
                                                            MouseButton::Middle,
                                                            move |_, window, cx| {
                                                                if topbar_pointer_up_should_be_ignored(cx) {
                                                                    return;
                                                                }
                                                                close_topbar_tab(
                                                                    &middle_close_entity,
                                                                    index,
                                                                    window,
                                                                    cx,
                                                                );
                                                                cx.stop_propagation();
                                                            },
                                                        )
                                                        .hover(move |this| {
                                                            this.bg(rgb(tab_hover_background))
                                                        })
                                                        .when(!is_renaming, |this| {
                                                            this.on_drag(
                                                                drag_payload,
                                                                |drag, _, _, cx| {
                                                                    cx.new(|_| drag.clone())
                                                                },
                                                            )
                                                        })
                                                        .drag_over::<DraggedTab>(
                                                            move |style: StyleRefinement, payload: &DraggedTab, _, _| {
                                                                if payload.source_tab_id == tab_id {
                                                                    return style;
                                                                }

                                                                let mut refined = style;
                                                                refined.background = Some(color_with_alpha(roles.primary, 0x18).into());
                                                                if payload.source_index < index {
                                                                    refined.border_widths.right = Some(px(3.0).into());
                                                                } else if payload.source_index > index {
                                                                    refined.border_widths.left = Some(px(3.0).into());
                                                                }
                                                                refined.border_color = Some(rgb(roles.primary).into());
                                                                refined
                                                            },
                                                        )
                                                        .on_drop::<DraggedTab>(
                                                            move |payload: &DraggedTab, _, cx| {
                                                                let entity = drop_entity.clone();
                                                                let source_id = payload.source_tab_id;
                                                                entity.update(cx, |this, cx| {
                                                                    let from = match this
                                                                        .workspace_state.tabs
                                                                        .iter()
                                                                        .position(|t| t.id == source_id)
                                                                    {
                                                                        Some(idx) => idx,
                                                                        None => return,
                                                                    };
                                                                    let target = this
                                                                        .workspace_state.tabs
                                                                        .iter()
                                                                        .position(|t| t.id == tab_id)
                                                                        .unwrap_or(index);
                                                                    this.reorder_tab(from, target, cx);
                                                                });
                                                            },
                                                        )
                                                        .context_menu(move |menu, _window, _cx| {
                                                            build_tab_context_menu(
                                                                menu,
                                                                menu_entity.clone(),
                                                                index,
                                                                is_session,
                                                            )
                                                        })
                                                        .child(
                                                            h_flex()
                                                                .w_full()
                                                                .h_full()
                                                                .items_center()
                                                                .justify_between()
                                                                .gap_1()
                                                                .child(
                                                                    h_flex()
                                                                        .flex_1()
                                                                        .min_w(px(0.0))
                                                                        .h_full()
                                                                        .px_3()
                                                                        .items_center()
                                                                        .gap_2()
                                                                        .text_color(rgb(tab_foreground_color))
                                                                        .cursor_pointer()
                                                                        .on_mouse_up(MouseButton::Left, move |_, window, cx| {
                                                                            if topbar_pointer_up_should_be_ignored(cx) {
                                                                                return;
                                                                            }
                                                                            activate_entity.update(
                                                                                cx,
                                                                                |this, cx| {
                                                                                    this.activate_tab(index, window, cx)
                                                                                },
                                                                            );
                                                                        })
                                                                        .when(!is_renaming, |this| {
                                                                            this.on_mouse_down(MouseButton::Middle, |_, _, cx| {
                                                                                cx.stop_propagation();
                                                                            })
                                                                            .on_mouse_up(
                                                                                MouseButton::Middle,
                                                                                move |_, window, cx| {
                                                                                    if topbar_pointer_up_should_be_ignored(cx) {
                                                                                        return;
                                                                                    }
                                                                                    close_topbar_tab(
                                                                                        &content_middle_close_entity,
                                                                                        index,
                                                                                        window,
                                                                                        cx,
                                                                                    );
                                                                                    cx.stop_propagation();
                                                                                },
                                                                            )
                                                                        })
                                                                        .when_some(status_color, |this, color| {
                                                                            this.child(status_indicator(color, 7.0))
                                                                        })
                                                                        .when_some(tab_kind_icon, |this, icon| {
                                                                            this.child(
                                                                                div()
                                                                                    .flex_shrink_0()
                                                                                    .child(Icon::new(icon).small()),
                                                                            )
                                                                        })
                                                                        .child(title_child),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .flex_shrink_0()
                                                                        .size(px(24.0))
                                                                        .rounded(px(999.0))
                                                                        .flex()
                                                                        .items_center()
                                                                        .justify_center()
                                                                        .cursor_pointer()
                                                                        .text_size(settings::scaled_font_size(10.0))
                                                                        .text_color(rgb(tab_foreground_color))
                                                                        .occlude()
                                                                        .on_scroll_wheel(move |event: &ScrollWheelEvent, _, cx| {
                                                                            if scroll_topbar_tabs(&close_scroll_handle, event) {
                                                                                close_scroll_entity.update(cx, |_, cx| cx.notify());
                                                                                cx.stop_propagation();
                                                                            }
                                                                        })
                                                                        .hover(move |this| {
                                                                            this.bg(color_with_alpha(
                                                                                tab_foreground_color,
                                                                                if is_active { 0x18 } else { 0x12 },
                                                                            ))
                                                                        })
                                                                        .child(Icon::from(AppIcon::Close).small())
                                                                        .on_mouse_down(MouseButton::Middle, |_, _, cx| {
                                                                            cx.stop_propagation();
                                                                        })
                                                                        .on_mouse_up(MouseButton::Middle, move |_, window, cx| {
                                                                            if topbar_pointer_up_should_be_ignored(cx) {
                                                                                return;
                                                                            }
                                                                            close_topbar_tab(
                                                                                &close_middle_entity,
                                                                                index,
                                                                                window,
                                                                                cx,
                                                                            );
                                                                            cx.stop_propagation();
                                                                        })
                                                                        .on_mouse_up(MouseButton::Left, move |_, window, cx| {
                                                                            if topbar_pointer_up_should_be_ignored(cx) {
                                                                                return;
                                                                            }
                                                                            close_topbar_tab(
                                                                                &close_entity,
                                                                                index,
                                                                                window,
                                                                                cx,
                                                                            );
                                                                            cx.stop_propagation();
                                                                        }),
                                                                ),
                                                        );

                                                    let bubble_side_inset =
                                                        (1.0 - tab_visibility) * 14.0;
                                                    let bubble_vertical_offset =
                                                        (1.0 - tab_visibility) * 18.0;
                                                    let bubble_opacity = 0.12 + tab_visibility * 0.88;

                                                    div()
                                                        .relative()
                                                        .flex_shrink_0()
                                                        .w(px(TOPBAR_TAB_WIDTH))
                                                        .h_full()
                                                        .overflow_hidden()
                                                        .child(
                                                            div()
                                                                .absolute()
                                                                .top(px(bubble_vertical_offset))
                                                                .right(px(bubble_side_inset))
                                                                .bottom(px(-bubble_vertical_offset))
                                                                .left(px(bubble_side_inset))
                                                                .opacity(bubble_opacity)
                                                                .child(animated_tab),
                                                        )
                                                }
                                            }))
                                            .children(exiting_tabs.iter().map(|tab| {
                                                let snapshot = &tab.snapshot;
                                                let display_title = truncate_with_ellipsis(
                                                    &snapshot.title,
                                                    TOPBAR_TAB_TITLE_CHARS,
                                                );
                                                let tab_foreground_color = blend_rgb(
                                                    roles.on_surface_variant,
                                                    roles.on_secondary_container,
                                                    tab.active_strength,
                                                );
                                                let tab_background = blend_rgb(
                                                    roles.surface_container_low,
                                                    roles.secondary_container,
                                                    tab.active_strength,
                                                );
                                                let tab_kind_icon = topbar_tab_icon(snapshot.kind);

                                                div()
                                                    .absolute()
                                                    .left(px(
                                                        snapshot.visible_index as f32
                                                            * (TOPBAR_TAB_WIDTH + TOPBAR_TAB_GAP),
                                                    ))
                                                    .top(px(0.0))
                                                    .bottom(px(0.0))
                                                    .w(px(TOPBAR_TAB_WIDTH * tab.visibility))
                                                    .min_w(px(0.0))
                                                    .overflow_hidden()
                                                    .child(
                                                        div()
                                                            .absolute()
                                                            .top(px((1.0 - tab.visibility) * 4.0))
                                                            .left(px(0.0))
                                                            .bottom(px(0.0))
                                                            .w(px(TOPBAR_TAB_WIDTH))
                                                            .opacity(0.18 + tab.visibility * 0.82)
                                                            .child(
                                                                div()
                                                                    .w(px(TOPBAR_TAB_WIDTH))
                                                                    .h_full()
                                                                    .p_1()
                                                                    .rounded(px(20.0))
                                                                    .bg(rgb(tab_background))
                                                                    .child(
                                                                        h_flex()
                                                                            .w_full()
                                                                            .h_full()
                                                                            .items_center()
                                                                            .justify_between()
                                                                            .gap_1()
                                                                            .child(
                                                                                h_flex()
                                                                                    .flex_1()
                                                                                    .min_w(px(0.0))
                                                                                    .h_full()
                                                                                    .px_3()
                                                                                    .items_center()
                                                                                    .gap_2()
                                                                                    .text_color(rgb(tab_foreground_color))
                                                                                    .when_some(
                                                                                        snapshot.status_color,
                                                                                        |this, color| {
                                                                                            this.child(status_indicator(color, 7.0))
                                                                                        },
                                                                                    )
                                                                                    .when_some(
                                                                                        tab_kind_icon,
                                                                                        |this, icon| {
                                                                                            this.child(
                                                                                                div()
                                                                                                    .flex_shrink_0()
                                                                                                    .child(Icon::new(icon).small()),
                                                                                            )
                                                                                        },
                                                                                    )
                                                                                    .child(
                                                                                        div()
                                                                                            .flex_1()
                                                                                            .min_w(px(0.0))
                                                                                            .h(px(14.0))
                                                                                            .overflow_hidden()
                                                                                            .text_size(settings::scaled_font_size(11.0))
                                                                                            .line_height(settings::scaled_line_height(14.0))
                                                                                            .text_color(rgb(tab_foreground_color))
                                                                                            .child(display_title),
                                                                                    ),
                                                                            )
                                                                            .child(
                                                                                div()
                                                                                    .flex_shrink_0()
                                                                                    .size(px(24.0))
                                                                                    .rounded(px(999.0))
                                                                                    .flex()
                                                                                    .items_center()
                                                                                    .justify_center()
                                                                                    .text_size(settings::scaled_font_size(10.0))
                                                                                    .text_color(rgb(tab_foreground_color))
                                                                                    .child(Icon::from(AppIcon::Close).small()),
                                                                            ),
                                                                    ),
                                                            ),
                                                    )
                                            }))
                                            .when(!pin_add_button, |this| {
                                                this.child(topbar_add_button(
                                                    entity.clone(),
                                                    topbar_scroll_handle.clone(),
                                                ))
                                            })
                                            .when(cfg!(target_os = "macos"), |this| {
                                                this.child(topbar_drag_region("top-tabs-trailing-drag"))
                                            })
                                    )
                            )
                            .when(pin_add_button, |this| {
                                this.child(topbar_add_button(
                                    entity.clone(),
                                    topbar_scroll_handle.clone(),
                                ))
                            })
                    )
                    .when(!window_controls_on_left(), |this| {
                        this.child(window_controls_group(window))
                    }),
            )
    }

    pub(in crate::ui::shell) fn render_status_footer(
        &self,
        entity: Entity<Self>,
    ) -> impl IntoElement {
        let material = settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let active_session = self
            .workspace_state
            .workspace
            .active_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(|tab| tab.as_session().map(|session| (tab, session)));
        let panel_session =
            active_session.filter(|(_, session)| session.purpose == SessionPurpose::Terminal);

        let active_tab = active_session.map(|(tab, _)| tab).or_else(|| {
            self.workspace_state
                .active_topbar_tab
                .and_then(|index| self.workspace_state.tabs.get(index))
                .filter(|tab| tab.as_sftp().is_some())
        });

        let (connection_label, connection_color) = active_tab
            .map(|tab| {
                let status = tab.status.as_str();
                let label = if tab_is_connected_status(tab) {
                    i18n::string("session.footer.connected")
                } else {
                    if status.is_ascii() {
                        status.to_ascii_uppercase()
                    } else {
                        status.to_string()
                    }
                };
                (label, tab_status_color(tab))
            })
            .unwrap_or_else(|| (i18n::string("session.footer.offline"), text_muted));
        let connection_target = self
            .active_profile()
            .map(|profile| {
                let username = if profile.username.trim().is_empty() {
                    self.active_username()
                } else {
                    profile.username.clone()
                };
                format!("{username}@{}", profile.host)
            })
            .unwrap_or_else(|| "--".into());

        let pty_label = active_session.map(|(_, session)| {
            format!(
                "{}x{}",
                session.terminal.columns(),
                session.terminal.screen_lines()
            )
        });
        let traffic_label = active_session.map(|(_, session)| {
            i18n::string_args(
                "session.footer.traffic",
                &[
                    ("upload", &format_bytes(session.bytes_out)),
                    ("download", &format_bytes(session.bytes_in)),
                ],
            )
        });
        let monitor_toggle_entity = entity.clone();
        let snippets_toggle_entity = entity.clone();

        div()
            .h(px(STATUS_BAR_HEIGHT))
            .w_full()
            .flex_shrink_0()
            .bg(rgb(roles.surface_container))
            .px_2()
            .child(
                h_flex()
                    .w_full()
                    .h_full()
                    .items_center()
                    .gap_3()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .size(px(6.0))
                                    .rounded(px(999.0))
                                    .bg(rgb(connection_color)),
                            )
                            .child(
                                div()
                                    .text_size(settings::scaled_font_size(10.0))
                                    .text_color(rgb(roles.on_surface_variant))
                                    .child(connection_label),
                            ),
                    )
                    .when(panel_session.is_some(), |this| {
                        this.child(
                            div().id("session-monitor-panel-toggle").child(
                                icon_button(
                                    AppIcon::Computer,
                                    24.0,
                                    8.0,
                                    Some(roles.surface_container),
                                    Some(text_muted),
                                    Some(roles.outline_variant),
                                    move |_window, cx| {
                                        monitor_toggle_entity.update(cx, |this, cx| {
                                            this.panels.session_monitor_panel_open =
                                                !this.panels.session_monitor_panel_open;
                                            cx.notify();
                                        });
                                    },
                                )
                                .id("session-snippets-panel-toggle-button")
                                .hover(move |this| {
                                    this.bg(rgb(roles.surface_container_highest))
                                        .border_color(rgb(roles.primary))
                                }),
                            ),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(settings::scaled_font_size(10.0))
                            .text_color(rgb(text_muted))
                            .child(self.status_message.clone()),
                    )
                    .when_some(pty_label, |this, label| {
                        this.child(
                            div()
                                .text_size(settings::scaled_font_size(10.0))
                                .text_color(rgb(text_muted))
                                .child(label),
                        )
                    })
                    .when_some(traffic_label, |this, label| {
                        this.child(
                            div()
                                .text_size(settings::scaled_font_size(10.0))
                                .text_color(rgb(text_muted))
                                .child(label),
                        )
                    })
                    .when(panel_session.is_some(), |this| {
                        this.child(
                            div().id("session-snippets-panel-toggle").child(
                                icon_button(
                                    AppIcon::Notebook,
                                    24.0,
                                    8.0,
                                    Some(roles.surface_container),
                                    Some(text_muted),
                                    Some(roles.outline_variant),
                                    move |_window, cx| {
                                        snippets_toggle_entity.update(cx, |this, cx| {
                                            this.panels.session_snippets_panel_open =
                                                !this.panels.session_snippets_panel_open;
                                            cx.notify();
                                        });
                                    },
                                )
                                .id("session-snippets-panel-toggle-button")
                                .hover(move |this| {
                                    this.bg(rgb(roles.surface_container_highest))
                                        .border_color(rgb(roles.primary))
                                }),
                            ),
                        )
                    })
                    .child(
                        div()
                            .text_size(settings::scaled_font_size(10.0))
                            .text_color(rgb(roles.on_surface_variant))
                            .child(connection_target),
                    ),
            )
    }

    pub(in crate::ui::shell) fn render_fab(&self, entity: Entity<Self>) -> impl IntoElement {
        fab_button(move |window, cx| {
            entity.update(cx, |this, cx| this.open_add_host_editor(window, cx));
        })
    }
}
