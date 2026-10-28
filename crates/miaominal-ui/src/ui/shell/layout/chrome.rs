use super::super::*;
use crate::ui::components::footer_tool_button;
use crate::ui::components::shell_tokens::{ShellColors, ShellMetrics, ShellTypography, space};
use crate::ui::contracts::session::SessionTabSnapshot;
use crate::ui::i18n;
use gpui_kit::{StatefulInteractiveElement, TestSupportExt as _, WindowControlArea, point};
use std::time::{Duration, Instant};

const TOPBAR_TAB_GAP: f32 = 8.0;
const TOPBAR_ACTION_BUTTON_WIDTH: f32 = 36.0;
const TOPBAR_WINDOW_CONTROLS_WIDTH: f32 = TOPBAR_ACTION_BUTTON_WIDTH * 3.0 + TOPBAR_TAB_GAP * 2.0;
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

/// File name shown for a local terminal target in the status footer.
fn local_shell_file_name(shell: &str) -> String {
    let shell = shell.trim();
    std::path::Path::new(shell)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(shell)
        .to_string()
}

#[derive(Clone)]
pub(in crate::ui::shell) struct VisibleTopbarTab {
    tab_index: usize,
    snapshot: TopbarTabSnapshot,
}

#[derive(Clone)]
pub(in crate::ui::shell) struct ExitingTopbarTabRenderState {
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

fn profile_shell_label(shell: &str) -> String {
    let name = local_shell_file_name(shell);
    if name.is_empty() {
        i18n::string("hosts.home.local")
    } else {
        name
    }
}

fn snapshot_supports_sftp(kind: TopbarTabVisualKind) -> bool {
    kind == TopbarTabVisualKind::Session
}

fn topbar_tab_icon(kind: TopbarTabVisualKind) -> Option<AppIcon> {
    match kind {
        TopbarTabVisualKind::Hosts => None,
        TopbarTabVisualKind::Session => Some(AppIcon::LaptopMinimal),
        TopbarTabVisualKind::Sftp => Some(AppIcon::FolderSymlink),
        TopbarTabVisualKind::LocalTerminal => Some(AppIcon::Computer),
    }
}

fn connection_state_is_error(session: &SessionTabSnapshot) -> bool {
    session.connection_error()
}

fn tab_is_error_status(tab: &TabDescriptor, session: Option<&SessionTabSnapshot>) -> bool {
    match tab.kind {
        TabKindTag::Session => session.is_some_and(connection_state_is_error),
        TabKindTag::Sftp => {
            let error = i18n::string("session.status.error");
            let closed = i18n::string("session.status.closed");
            tab.status == error || tab.status == closed
        }
        TabKindTag::LocalTerminal => session.is_some_and(connection_state_is_error),
        TabKindTag::Hosts => false,
    }
}

fn tab_needs_attention(session: Option<&SessionTabSnapshot>) -> bool {
    session.is_some_and(|session| session.needs_attention())
}

pub(in crate::ui::shell) fn tab_status_indicator_color(
    tab: &TabDescriptor,
    session: Option<&SessionTabSnapshot>,
    has_activity: bool,
) -> Option<u32> {
    let roles = miaominal_settings::current_theme().material.roles;

    if tab_is_error_status(tab, session) {
        Some(roles.error)
    } else if has_activity || tab_needs_attention(session) {
        Some(roles.primary)
    } else {
        None
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

struct FooterMetrics {
    dimensions: Option<String>,
    traffic: Option<String>,
}

fn footer_metrics(
    session: Option<&SessionTabSnapshot>,
    sftp_bytes: Option<(u64, u64)>,
    local: bool,
) -> FooterMetrics {
    let dimensions =
        session.map(|session| format!("{} × {}", session.columns(), session.screen_lines()));
    let bytes = session
        .filter(|_| !local)
        .map(|session| (session.bytes_out(), session.bytes_in()))
        .or(sftp_bytes);
    FooterMetrics {
        dimensions,
        traffic: bytes.map(|(uploaded, downloaded)| {
            format!(
                "↑ {} · ↓ {}",
                format_bytes(uploaded),
                format_bytes(downloaded)
            )
        }),
    }
}

fn new_host_action(
    cx: &App,
    on_create: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    crate::ui::components::dock_action_button(
        "hosts-create-fab",
        i18n::string("hosts.home.create"),
        cx,
        on_create,
    )
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

#[derive(Clone, Copy)]
enum TopbarAction {
    OpenHosts,
    Activate(TabId),
    Rename(TabId),
    Duplicate(TabId),
    OpenSftp(TabId),
    OpenInNewWindow(TabId),
    CloseOthers(TabId),
    Close(TabId),
}

fn topbar_action_index(tabs: &TabRegistry, tab_id: TabId) -> Option<usize> {
    tabs.index_of(tab_id)
}

trait TopbarHost: Render + Sized + 'static {
    fn handle_topbar_action(
        &mut self,
        action: TopbarAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
}

impl TopbarHost for AppView {
    fn handle_topbar_action(
        &mut self,
        action: TopbarAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            TopbarAction::OpenHosts => {
                self.open_hosts_tab(cx);
                self.sync_window_overlays(window, cx);
            }
            TopbarAction::Activate(tab_id) => {
                if let Some(index) = topbar_action_index(&self.workspace.tabs, tab_id) {
                    self.activate_tab(index, window, cx);
                }
            }
            TopbarAction::Rename(tab_id) => {
                if let Some(index) = topbar_action_index(&self.workspace.tabs, tab_id) {
                    self.begin_rename_tab(index, window, cx);
                }
            }
            TopbarAction::Duplicate(tab_id) => {
                if let Some(index) = topbar_action_index(&self.workspace.tabs, tab_id) {
                    self.duplicate_profile_tab(index, window, cx);
                }
            }
            TopbarAction::OpenSftp(tab_id) => {
                if self.workspace.tabs.get(tab_id).is_some() {
                    self.open_sftp_tab_for_session(Some(tab_id), window, cx);
                }
            }
            TopbarAction::OpenInNewWindow(tab_id) => {
                self.open_tab_in_new_window(tab_id, window, cx);
            }
            TopbarAction::CloseOthers(tab_id) => {
                if let Some(index) = topbar_action_index(&self.workspace.tabs, tab_id) {
                    self.close_other_tabs(index, window, cx);
                }
            }
            TopbarAction::Close(tab_id) => {
                if let Some(index) = topbar_action_index(&self.workspace.tabs, tab_id) {
                    self.close_tab(index, window, cx);
                }
            }
        }
    }
}

fn topbar_add_button<V: TopbarHost>(
    entity: Entity<V>,
    scroll_handle: ScrollHandle,
) -> impl IntoElement {
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
                .w(space(32.0))
                .h(space(32.0))
                .rounded(px(99.0))
                .accessibility_label(i18n::string("chrome.new_tab"))
                .on_scroll_wheel(move |event: &ScrollWheelEvent, _, cx| {
                    if scroll_topbar_tabs(&scroll_handle, event) {
                        scroll_entity.update(cx, |_, cx| cx.notify());
                        cx.stop_propagation();
                    }
                })
                .child(Icon::from(AppIcon::Plus).size(space(20.0)))
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.handle_topbar_action(TopbarAction::OpenHosts, window, cx);
                    });
                }),
        )
}

fn topbar_drag_surface() -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id("topbar-window-drag")
        // The complete empty titlebar surface, including padding and tab gaps,
        // is native chrome. Interactive children occlude this hitbox.
        .window_control_area(WindowControlArea::Drag)
        // Windows forwards non-client presses through GPUI before DefWindowProc.
        // Stopping them here prevents the native move/zoom loop; start_window_move
        // is a no-op on Windows. Let the OS handle its caption events instead.
        .when(!cfg!(target_os = "windows"), |element| {
            element
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
        })
}

fn build_tab_context_menu<V: TopbarHost>(
    menu: PopupMenu,
    entity: Entity<V>,
    tab_id: TabId,
    supports_sftp_tab: bool,
    can_open_in_new_window: bool,
) -> PopupMenu {
    let rename_entity = entity.clone();
    let close_others_entity = entity.clone();
    let duplicate_entity = entity.clone();
    let sftp_entity = entity.clone();
    let open_in_new_window_entity = entity.clone();
    let close_entity = entity;

    let menu = menu.item(
        PopupMenuItem::new(i18n::string("chrome.menu.rename_tab")).on_click(
            move |_, window, cx| {
                let entity = rename_entity.clone();
                entity.update(cx, |this, cx| {
                    this.handle_topbar_action(TopbarAction::Rename(tab_id), window, cx)
                });
            },
        ),
    );

    let menu = if supports_sftp_tab {
        menu.item(
            PopupMenuItem::new(i18n::string("chrome.menu.duplicate_profile")).on_click(
                move |_, window, cx| {
                    let entity = duplicate_entity.clone();
                    entity.update(cx, |this, cx| {
                        this.handle_topbar_action(TopbarAction::Duplicate(tab_id), window, cx)
                    });
                },
            ),
        )
        .item(
            PopupMenuItem::new(i18n::string("chrome.menu.open_sftp_tab")).on_click(
                move |_, window, cx| {
                    let entity = sftp_entity.clone();
                    entity.update(cx, |this, cx| {
                        this.handle_topbar_action(TopbarAction::OpenSftp(tab_id), window, cx)
                    });
                },
            ),
        )
    } else {
        menu
    };

    let menu = if can_open_in_new_window {
        menu.item(
            PopupMenuItem::new(i18n::string("chrome.menu.open_in_new_window")).on_click(
                move |_, window, cx| {
                    let entity = open_in_new_window_entity.clone();
                    entity.update(cx, |this, cx| {
                        this.handle_topbar_action(TopbarAction::OpenInNewWindow(tab_id), window, cx)
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
                    entity.update(cx, |this, cx| {
                        this.handle_topbar_action(TopbarAction::CloseOthers(tab_id), window, cx)
                    });
                },
            ),
        )
        .item(
            PopupMenuItem::new(i18n::string("chrome.menu.close_tab")).on_click(
                move |_, window, cx| {
                    let entity = close_entity.clone();
                    entity.update(cx, |this, cx| {
                        this.handle_topbar_action(TopbarAction::Close(tab_id), window, cx)
                    });
                },
            ),
        )
}

fn close_topbar_tab<V: TopbarHost>(
    entity: &Entity<V>,
    tab_id: TabId,
    window: &mut Window,
    cx: &mut App,
) {
    entity.update(cx, |this, cx| {
        this.handle_topbar_action(TopbarAction::Close(tab_id), window, cx)
    });
}

fn topbar_pointer_up_should_be_ignored(cx: &mut App) -> bool {
    cx.has_active_drag()
}

fn window_control_button(
    id: &'static str,
    icon: AppIcon,
    control_area: WindowControlArea,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let is_windows = cfg!(target_os = "windows");
    let hover_background = ShellColors::current().hover;

    div()
        .id(SharedString::from(id))
        .w(space(40.0))
        .h(space(32.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(space(8.0))
        .bg(ShellColors::current().chrome)
        .text_color(rgb(roles.on_surface_variant))
        .cursor_pointer()
        .hover(move |this| this.bg(hover_background))
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
        .child(Icon::from(icon).size(space(20.0)))
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

fn request_window_close(
    entity: &Entity<AppView>,
    role: AppWindowRole,
    window: &mut Window,
    cx: &mut App,
) {
    if role == AppWindowRole::Primary {
        crate::ui::request_main_window_close(window, cx);
        return;
    }
    entity.update(cx, |view, cx| {
        view.prepare_detached_window_close(window, cx);
    });
    window.remove_window();
}

fn window_controls_group(
    entity: Entity<AppView>,
    role: AppWindowRole,
    window: &Window,
) -> impl IntoElement {
    if cfg!(target_os = "macos") {
        return div()
            .w(px(topbar_window_controls_width(window)))
            .h_full()
            .flex_shrink_0()
            .into_any_element();
    }

    if window_controls_on_left() {
        let close_entity = entity.clone();
        h_flex()
            .items_center()
            .gap(space(4.0))
            .child(window_control_button(
                "window-close",
                AppIcon::Close,
                WindowControlArea::Close,
                move |window, cx| {
                    request_window_close(&close_entity, role, window, cx);
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
        let close_entity = entity;
        h_flex()
            .items_center()
            .gap(space(4.0))
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
                move |window, cx| {
                    request_window_close(&close_entity, role, window, cx);
                },
            ))
            .into_any_element()
    }
}

pub(in crate::ui::shell) trait ChromeAppViewExt: Sized {
    fn collect_visible_topbar_tabs(
        &self,
        current_active_tab_id: Option<TabId>,
        cx: &App,
    ) -> Vec<VisibleTopbarTab>;

    fn sync_topbar_tab_animation_state(
        &mut self,
        visible_tabs: &[VisibleTopbarTab],
        current_active_tab_id: Option<TabId>,
        window: &mut Window,
    );

    fn topbar_tab_visibility(&self, tab_id: TabId) -> f32;

    fn topbar_active_strength(&self, tab_id: TabId, current_active_tab_id: Option<TabId>) -> f32;

    fn topbar_exiting_tabs_render_state(
        &self,
        current_active_tab_id: Option<TabId>,
    ) -> Vec<ExitingTopbarTabRenderState>;

    fn render_top_bar(
        &mut self,
        entity: Entity<Self>,
        window: &mut Window,
        cx: &App,
    ) -> impl IntoElement;

    fn render_status_footer(
        &self,
        entity: Entity<Self>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement;

    /// The hosts page's create command, docked into the bottom navigation bar.
    fn render_new_host_action(&self, cx: &App) -> AnyElement;
}

impl ChromeAppViewExt for AppView {
    fn collect_visible_topbar_tabs(
        &self,
        current_active_tab_id: Option<TabId>,
        cx: &App,
    ) -> Vec<VisibleTopbarTab> {
        self.workspace
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| tab.is_top_level())
            .enumerate()
            .map(|(visible_index, (tab_index, tab))| {
                let session = self.session_tab(tab.id, cx);
                let has_activity = session
                    .as_ref()
                    .is_some_and(|session| session.has_activity())
                    && current_active_tab_id != Some(tab.id);
                let kind = match tab.kind {
                    TabKindTag::Hosts => TopbarTabVisualKind::Hosts,
                    TabKindTag::Session => TopbarTabVisualKind::Session,
                    TabKindTag::Sftp => TopbarTabVisualKind::Sftp,
                    TabKindTag::LocalTerminal => TopbarTabVisualKind::LocalTerminal,
                };

                VisibleTopbarTab {
                    tab_index,
                    snapshot: TopbarTabSnapshot {
                        tab_id: tab.id,
                        visible_index,
                        title: tab.title.clone(),
                        kind,
                        status_color: tab_status_indicator_color(
                            &tab,
                            session.as_ref(),
                            has_activity,
                        ),
                    },
                }
            })
            .collect()
    }

    fn sync_topbar_tab_animation_state(
        &mut self,
        visible_tabs: &[VisibleTopbarTab],
        current_active_tab_id: Option<TabId>,
        window: &mut Window,
    ) {
        let now = Instant::now();
        let duration = support::CONTAINER_TRANSITION_DURATION;
        let current_tab_ids: Vec<_> = visible_tabs.iter().map(|tab| tab.snapshot.tab_id).collect();

        for tab in visible_tabs {
            if !self
                .workspace
                .topbar_previous_visible_tabs
                .iter()
                .any(|previous| previous.tab_id == tab.snapshot.tab_id)
                && !self
                    .workspace
                    .topbar_entering_tabs
                    .iter()
                    .any(|transition| transition.tab_id == tab.snapshot.tab_id)
            {
                self.workspace
                    .topbar_entering_tabs
                    .push(TopbarTabEnterTransition {
                        tab_id: tab.snapshot.tab_id,
                        started_at: now,
                        duration,
                    });
            }
        }

        for previous in &self.workspace.topbar_previous_visible_tabs {
            if !current_tab_ids.contains(&previous.tab_id)
                && !self
                    .workspace
                    .topbar_exiting_tabs
                    .iter()
                    .any(|transition| transition.snapshot.tab_id == previous.tab_id)
            {
                self.workspace
                    .topbar_exiting_tabs
                    .push(TopbarTabExitTransition {
                        snapshot: previous.clone(),
                        started_at: now,
                        duration,
                    });
            }
        }

        self.workspace.topbar_previous_visible_tabs = visible_tabs
            .iter()
            .map(|tab| tab.snapshot.clone())
            .collect();

        self.workspace.topbar_entering_tabs.retain(|transition| {
            current_tab_ids.contains(&transition.tab_id)
                && topbar_transition_raw_progress(transition.started_at, transition.duration) < 1.0
        });
        self.workspace.topbar_exiting_tabs.retain(|transition| {
            topbar_transition_raw_progress(transition.started_at, transition.duration) < 1.0
        });

        if self.workspace.topbar_visible_active_tab_id != current_active_tab_id {
            self.workspace.topbar_active_transition = Some(TopbarActiveTabTransition {
                from_tab_id: self.workspace.topbar_visible_active_tab_id,
                to_tab_id: current_active_tab_id,
                started_at: now,
                duration,
            });
            self.workspace.topbar_visible_active_tab_id = current_active_tab_id;
        }

        if let Some(transition) = self.workspace.topbar_active_transition
            && topbar_transition_raw_progress(transition.started_at, transition.duration) >= 1.0
        {
            self.workspace.topbar_active_transition = None;
        }

        if !self.workspace.topbar_entering_tabs.is_empty()
            || !self.workspace.topbar_exiting_tabs.is_empty()
            || self.workspace.topbar_active_transition.is_some()
        {
            window.request_animation_frame();
        }
    }

    fn topbar_tab_visibility(&self, tab_id: TabId) -> f32 {
        self.workspace
            .topbar_entering_tabs
            .iter()
            .find(|transition| transition.tab_id == tab_id)
            .map(|transition| {
                topbar_transition_progress(transition.started_at, transition.duration)
            })
            .unwrap_or(1.0)
    }

    fn topbar_active_strength(&self, tab_id: TabId, current_active_tab_id: Option<TabId>) -> f32 {
        let Some(transition) = self.workspace.topbar_active_transition else {
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
        current_active_tab_id: Option<TabId>,
    ) -> Vec<ExitingTopbarTabRenderState> {
        self.workspace
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

    fn render_top_bar(
        &mut self,
        entity: Entity<Self>,
        window: &mut Window,
        cx: &App,
    ) -> impl IntoElement {
        let colors = ShellColors::current();
        let active = self.workspace.active_topbar_tab;
        let tabs = self.collect_visible_topbar_tabs(active, cx);
        let active_changed = self.workspace.topbar_visible_active_tab_id != active;
        let tab_layout_animating = !self.workspace.topbar_entering_tabs.is_empty()
            || !self.workspace.topbar_exiting_tabs.is_empty();
        self.sync_topbar_tab_animation_state(&tabs, active, window);
        if (active_changed
            || tab_layout_animating
            || !self.workspace.topbar_entering_tabs.is_empty())
            && let Some(index) = tabs
                .iter()
                .position(|tab| Some(tab.snapshot.tab_id) == active)
        {
            self.workspace
                .topbar_tab_scroll_handle
                .scroll_to_item(index);
        }
        self.shell
            .topbar_tab_focus
            .retain(|id, _| tabs.iter().any(|tab| tab.snapshot.tab_id == *id));
        for tab in &tabs {
            self.shell
                .topbar_tab_focus
                .entry(tab.snapshot.tab_id)
                .or_insert_with(|| cx.focus_handle());
        }
        let scale = f32::from(window.rem_size()) / 14.0;
        let width = f32::from(window.bounds().size.width) / scale;
        let tab_width = if width <= 850.0 {
            ShellMetrics::COMPACT_TAB_WIDTH
        } else {
            ShellMetrics::TAB_WIDTH
        };
        let scroll = self.workspace.topbar_tab_scroll_handle.clone();
        let controls_width = if cfg!(target_os = "macos") {
            topbar_window_controls_width(window) / scale
        } else {
            128.0
        };
        let strip_width = (tabs.len() as f32 * (tab_width + 12.0) + 4.0)
            .min((width - controls_width - 88.0).max(0.0));
        let rename_height = miaominal_settings::scaled_line_height(24.0);
        let role = self.window_role;
        let tab_elements = tabs
            .iter()
            .map(|visible| {
                let snapshot = &visible.snapshot;
                let tab_id = snapshot.tab_id;
                let is_active = active == Some(tab_id);
                let tab_focus = self.shell.topbar_tab_focus[&tab_id].clone();
                let visibility = self.topbar_tab_visibility(tab_id);
                let strength = self.topbar_active_strength(tab_id, active);
                let roles = miaominal_settings::current_theme().material.roles;
                let tab_background: gpui_kit::Hsla = rgb(blend_rgb(
                    roles.surface_container_high,
                    roles.surface,
                    strength,
                ))
                .into();
                let is_renaming = self.workspace.renaming_tab == Some(tab_id);
                let activate = entity.clone();
                let close = entity.clone();
                let middle_close = entity.clone();
                let menu = entity.clone();
                let drop_target = entity.clone();
                let keyboard = entity.clone();
                let wheel_entity = entity.clone();
                let wheel_scroll = scroll.clone();
                let can_detach = self.can_open_tab_in_new_window(tab_id, cx);
                let supports_sftp = snapshot_supports_sftp(snapshot.kind);
                let payload = DraggedTab {
                    source_tab_id: tab_id,
                    source_index: visible.tab_index,
                    source_pane_id: self.active_pane_id(),
                    is_active,
                    title: snapshot.title.clone(),
                    status_color: snapshot.status_color,
                };
                let title = if is_renaming {
                    HintedInput::new(&self.shell.workspace_forms.rename_input)
                        .appearance(false)
                        .xsmall()
                        .h(rename_height)
                        .w_full()
                        .into_any_element()
                } else {
                    Button::new(SharedString::from(format!("top-tab-label-{tab_id}")))
                        .custom(
                            gpui_kit::component::button::ButtonCustomVariant::new(cx).foreground(
                                if is_active {
                                    colors.foreground
                                } else {
                                    colors.muted
                                },
                            ),
                        )
                        .tab_stop(false)
                        .accessibility_label(snapshot.title.clone())
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .px_0()
                        .text_color(if is_active {
                            colors.foreground
                        } else {
                            colors.muted
                        })
                        .when_some(snapshot.status_color, |button, color| {
                            button.child(status_indicator(color, 6.0))
                        })
                        .child(
                            Icon::new(topbar_tab_icon(snapshot.kind).unwrap_or(AppIcon::Computer))
                                .size(space(16.0))
                                .when(is_active, |icon| icon.text_color(colors.primary)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(space(ShellTypography::BODY_SIZE))
                                .line_height(space(ShellTypography::BODY_LINE_HEIGHT))
                                .when(is_active, |label| {
                                    label.font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                })
                                .child(snapshot.title.clone()),
                        )
                        .on_click(move |_, window, cx| {
                            activate.update(cx, |this, cx| {
                                this.handle_topbar_action(
                                    TopbarAction::Activate(tab_id),
                                    window,
                                    cx,
                                )
                            })
                        })
                        .into_any_element()
                };
                let corner = |left: bool| {
                    div()
                        .absolute()
                        .bottom_0()
                        .size(space(8.0))
                        .bg(colors.canvas)
                        .when(left, |element| element.left(space(-8.0)))
                        .when(!left, |element| element.right(space(-8.0)))
                        .child(
                            div()
                                .size_full()
                                .bg(colors.chrome)
                                .when(left, |element| element.rounded_br(space(8.0)))
                                .when(!left, |element| element.rounded_bl(space(8.0))),
                        )
                };
                div()
                    .id(SharedString::from(format!("top-tab-{tab_id}")))
                    .test_support()
                    .occlude()
                    // Tabs occlude the native caption hitbox. Forward wheel input
                    // here too, since that occlusion also hides the scroll strip.
                    .on_scroll_wheel(move |event, _, cx| {
                        if scroll_topbar_tabs(&wheel_scroll, event) {
                            wheel_entity.update(cx, |_, cx| cx.notify());
                            cx.stop_propagation();
                        }
                    })
                    .role(gpui_kit::Role::Tab)
                    .aria_label(snapshot.title.clone())
                    .aria_selected(is_active)
                    .track_focus(&tab_focus)
                    .tab_stop(is_active)
                    .focus(move |style| style.border_1().border_color(colors.primary))
                    .relative()
                    .w(space(tab_width * visibility))
                    .opacity(visibility)
                    .h(space(ShellMetrics::TAB_HEIGHT))
                    .flex_shrink_0()
                    .rounded_t(space(16.0))
                    .bg(tab_background)
                    .when(!is_active, |element| {
                        element
                            .hover(|style| style.bg(colors.hover))
                            .active(|style| style.bg(colors.selected))
                    })
                    .when(is_active, |element| {
                        element.child(corner(true)).child(corner(false))
                    })
                    .on_key_down(move |event, window, cx| {
                        if is_renaming
                            || event.keystroke.modifiers.control
                            || event.keystroke.modifiers.alt
                            || event.keystroke.modifiers.platform
                            || event.keystroke.modifiers.shift
                        {
                            return;
                        }
                        let direction = event.keystroke.key.as_str();
                        if !matches!(
                            direction,
                            "left" | "right" | "home" | "end" | "enter" | "space"
                        ) {
                            return;
                        }
                        keyboard.update(cx, |this, cx| {
                            let ids: Vec<_> = this
                                .workspace
                                .tabs
                                .iter()
                                .filter(|tab| tab.is_top_level())
                                .map(|tab| tab.id)
                                .collect();
                            let Some(ix) = ids.iter().position(|id| *id == tab_id) else {
                                return;
                            };
                            let next = match direction {
                                "home" => 0,
                                "end" => ids.len() - 1,
                                "left" => (ix + ids.len() - 1) % ids.len(),
                                "enter" | "space" => ix,
                                _ => (ix + 1) % ids.len(),
                            };
                            this.handle_topbar_action(
                                TopbarAction::Activate(ids[next]),
                                window,
                                cx,
                            );
                            this.workspace.topbar_tab_scroll_handle.scroll_to_item(next);
                            if let Some(focus) = this.shell.topbar_tab_focus.get(&ids[next]) {
                                window.focus(focus, cx);
                            }
                        });
                        cx.stop_propagation();
                    })
                    .on_mouse_up(MouseButton::Middle, move |_, window, cx| {
                        if !topbar_pointer_up_should_be_ignored(cx) {
                            close_topbar_tab(&middle_close, tab_id, window, cx);
                        }
                        cx.stop_propagation();
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .capture_any_mouse_down(|event, window, _| {
                        if matches!(event.button, MouseButton::Right | MouseButton::Middle) {
                            // Keep the menu's restore target on the current view,
                            // not the nested label/close button's clipped focus ring.
                            // Middle-click closes the tab, so it must not focus the
                            // nested title button and flash its focus ring either.
                            window.prevent_default();
                        }
                    })
                    .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .when(!is_renaming, |element| {
                        element.on_drag(payload, |payload, _, _, cx| cx.new(|_| payload.clone()))
                    })
                    .drag_over::<DraggedTab>(move |mut style, payload, _, _| {
                        if payload.source_tab_id != tab_id {
                            style.background = Some(colors.selected.into());
                        }
                        style
                    })
                    .on_drop::<DraggedTab>(move |payload, _, cx| {
                        drop_target.update(cx, |this, cx| {
                            if let (Some(from), Some(to)) = (
                                this.workspace.tabs.index_of(payload.source_tab_id),
                                this.workspace.tabs.index_of(tab_id),
                            ) {
                                this.reorder_tab(from, to, cx);
                            }
                        })
                    })
                    .context_menu(move |popup, _, _| {
                        build_tab_context_menu(
                            popup,
                            menu.clone(),
                            tab_id,
                            supports_sftp,
                            can_detach,
                        )
                    })
                    .child(
                        h_flex()
                            .size_full()
                            .px(space(8.0))
                            .pl(space(16.0))
                            .gap(space(8.0))
                            .child(title)
                            .child(
                                Button::new(SharedString::from(format!("close-tab-{tab_id}")))
                                    .ghost()
                                    .accessibility_label(i18n::string("chrome.menu.close_tab"))
                                    .tab_stop(is_active)
                                    .child(Icon::new(AppIcon::Close).size(space(20.0)))
                                    .size(space(28.0))
                                    .rounded_full()
                                    .text_color(colors.muted)
                                    .on_click(move |_, window, cx| {
                                        close_topbar_tab(&close, tab_id, window, cx);
                                        cx.stop_propagation();
                                    }),
                            ),
                    )
            })
            .collect::<Vec<_>>();
        let mut tab_elements = tab_elements
            .into_iter()
            .map(IntoElement::into_any_element)
            .collect::<Vec<_>>();
        for exiting in self.topbar_exiting_tabs_render_state(active) {
            let index = exiting.snapshot.visible_index.min(tab_elements.len());
            let roles = miaominal_settings::current_theme().material.roles;
            tab_elements.insert(
                index,
                div()
                    .w(space(tab_width * exiting.visibility))
                    .h(space(ShellMetrics::TAB_HEIGHT))
                    .flex_shrink_0()
                    .opacity(exiting.visibility)
                    .rounded_t(space(16.0))
                    .bg(rgb(blend_rgb(
                        roles.surface_container_high,
                        roles.surface,
                        exiting.active_strength,
                    )))
                    .child(
                        div()
                            .px(space(16.0))
                            .py(space(12.0))
                            .truncate()
                            .child(exiting.snapshot.title),
                    )
                    .into_any_element(),
            );
        }
        let scroll_entity = entity.clone();
        let scroll_wheel = scroll.clone();
        let chrome_scroll = scroll.clone();
        let chrome_entity = entity.clone();
        topbar_drag_surface()
            .test_support()
            .on_scroll_wheel(move |event, _, cx| {
                if scroll_topbar_tabs(&chrome_scroll, event) {
                    chrome_entity.update(cx, |_, cx| cx.notify());
                    cx.stop_propagation();
                }
            })
            .h(space(ShellMetrics::TOPBAR_HEIGHT))
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_end()
            .pt(space(8.0))
            .px(space(ShellMetrics::titlebar_inset(width)))
            .gap(space(4.0))
            .bg(colors.chrome)
            .when(show_macos_traffic_light_space(window), |element| {
                element.child(window_controls_group(entity.clone(), role, window))
            })
            .child(
                h_flex()
                    .id("top-tabs-strip")
                    .w(space(strip_width))
                    .flex_shrink_0()
                    .min_w_0()
                    .h_full()
                    .items_end()
                    .gap(space(12.0))
                    .px(space(8.0))
                    .overflow_x_scroll()
                    .track_scroll(&scroll)
                    .children(tab_elements)
                    .on_scroll_wheel(move |event, _, cx| {
                        if scroll_topbar_tabs(&scroll_wheel, event) {
                            scroll_entity.update(cx, |_, cx| cx.notify());
                            cx.stop_propagation();
                        }
                    }),
            )
            .child(
                div()
                    .pb(space(8.0))
                    .child(topbar_add_button(entity.clone(), scroll)),
            )
            .child(div().flex_1().min_w_0().h_full())
            .when(!window_controls_on_left(), |element| {
                element.child(
                    // Center the 32px controls in the full topbar rather than in
                    // the tab row, which sits below the 8px top padding.
                    div()
                        .pb(space((ShellMetrics::TOPBAR_HEIGHT - 32.0) / 2.0))
                        .child(window_controls_group(entity, role, window)),
                )
            })
    }

    fn render_status_footer(
        &self,
        entity: Entity<Self>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        use gpui_kit::component::popover::Popover;
        let colors = ShellColors::current();
        let active_tab = self
            .workspace
            .active_topbar_tab
            .and_then(|id| self.workspace.tabs.get(id));
        let session = active_tab.and_then(|tab| self.session_tab(tab.id, cx));
        let sftp = active_tab.and_then(|tab| {
            self.controllers
                .sftp
                .read(cx)
                .chrome_snapshot(tab.id)
                .map(|snapshot| (tab.id, snapshot))
        });
        let workspace = session.is_some() || sftp.is_some();
        let local = active_tab.is_some_and(|tab| tab.is_local_terminal());
        let connected = session.as_ref().is_some_and(|session| session.connected())
            || sftp.as_ref().is_some_and(|(_, state)| state.connected());
        let profile = active_tab.and_then(|tab| {
            self.controllers
                .session
                .read(cx)
                .reopen_profile_for_tab(tab.id)
                .or_else(|| {
                    self.controllers
                        .sftp
                        .read(cx)
                        .reopen_profile_for_tab(tab.id, tab.owner())
                })
        });
        let label = if !workspace {
            i18n::string("hosts.home.ready")
        } else if local && connected {
            profile_shell_label(
                profile
                    .as_ref()
                    .map(|profile| profile.local_shell.as_str())
                    .unwrap_or_default(),
            )
        } else if connected {
            i18n::string("session.footer.connected")
        } else {
            active_tab.map(|tab| tab.status.clone()).unwrap_or_default()
        };
        let name = active_tab.map(|tab| tab.title.clone()).unwrap_or_default();
        let endpoint = profile
            .as_ref()
            .map(|profile| {
                if profile.is_local() {
                    profile.local_shell.clone()
                } else {
                    format!("{}@{}:{}", profile.username, profile.host, profile.port)
                }
            })
            .unwrap_or_default();
        let FooterMetrics {
            dimensions,
            traffic,
        } = footer_metrics(
            session.as_ref(),
            sftp.as_ref()
                .map(|(_, state)| (state.uploaded(), state.downloaded())),
            local,
        );
        let profiles = self.controllers.hosts.read(cx).profiles();
        let context = if workspace {
            if self.shell.status_message.is_empty() {
                name.clone()
            } else {
                self.shell.status_message.clone()
            }
        } else if self.shell.shell_state.sidebar_section == SidebarSection::Hosts {
            i18n::string_args(
                "hosts.home.summary",
                &[
                    (
                        "remote",
                        &profiles
                            .iter()
                            .filter(|profile| !profile.is_local())
                            .count()
                            .to_string(),
                    ),
                    (
                        "local",
                        &profiles
                            .iter()
                            .filter(|profile| profile.is_local())
                            .count()
                            .to_string(),
                    ),
                ],
            )
        } else {
            self.shell.shell_state.sidebar_section.title()
        };
        drop(profiles);
        let wide =
            f32::from(window.bounds().size.width) / (f32::from(window.rem_size()) / 14.0) > 1100.0;
        let detail_name = name.clone();
        let detail_dimensions = dimensions.clone();
        let detail_traffic = traffic.clone();
        let session_info = Popover::new("footer-session-info")
            .anchor(gpui_kit::Anchor::BottomRight)
            .trigger(
                Button::new("footer-session-info-trigger")
                    .ghost()
                    .accessibility_label(i18n::string("hosts.home.session_info"))
                    .h(space(32.0))
                    .max_w(space(if wide { 224.0 } else { 144.0 }))
                    .text_color(colors.muted)
                    .child(
                        Icon::new(if local {
                            AppIcon::Computer
                        } else {
                            AppIcon::LaptopMinimal
                        })
                        .small(),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(space(12.0))
                            .child(name),
                    )
                    .child(Icon::new(AppIcon::ChevronUp).small()),
            )
            .content(move |_, _, _| {
                v_flex()
                    .w(space(320.0))
                    .p(space(16.0))
                    .gap(space(12.0))
                    .child(
                        div()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(i18n::string("hosts.home.session_info")),
                    )
                    .child(div().child(detail_name.clone()))
                    .child(div().child(endpoint.clone()))
                    .when_some(detail_dimensions.clone(), |column, value| {
                        column.child(div().child(value))
                    })
                    .when_some(detail_traffic.clone(), |column, value| {
                        column.child(div().child(value))
                    })
            });
        let monitor = entity.clone();
        let agent = entity.clone();
        let session_id = self
            .active_terminal_session_index(cx)
            .and_then(|index| self.workspace.tabs.id_at(index));
        let session_progress = self.controllers.session.clone();
        let progress = self.controllers.sftp.clone();
        let favorites = self.controllers.sftp.clone();
        let favorite_toggle = sftp
            .as_ref()
            .map(|(id, state)| (*id, state.favorites_panel_visible()));
        let progress_open = sftp
            .as_ref()
            .map(|(_, state)| state.progress_center_visible())
            .unwrap_or_else(|| {
                session_id
                    .and_then(|id| self.session_tab(id, cx))
                    .is_some_and(|state| state.sftp_progress_visible())
            });
        let sftp_id = sftp.as_ref().map(|(id, _)| *id);
        let sftp_tooltip = if progress_open {
            i18n::string("sftp.tooltips.hide_progress_center")
        } else {
            i18n::string("sftp.tooltips.show_progress_center")
        };
        let tool = |id, icon, label: String, selected: bool| {
            footer_tool_button(id, selected, cx)
                .icon(icon)
                .tooltip(label.clone())
                .accessibility_label(label)
        };
        div()
            .w_full()
            .flex_shrink_0()
            .px(space(8.0))
            .pb(space(8.0))
            .bg(colors.chrome)
            .child(
                h_flex()
                    .w_full()
                    .min_h(space(36.0))
                    .px(space(16.0))
                    .gap(space(if wide { 16.0 } else { 8.0 }))
                    .rounded_b(space(16.0))
                    .bg(colors.footer)
                    .text_size(space(ShellTypography::SUPPORTING_SIZE))
                    .line_height(space(ShellTypography::SUPPORTING_LINE_HEIGHT))
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .min_w_0()
                            .max_w(space(if wide { 240.0 } else { 144.0 }))
                            .gap(space(8.0))
                            .child(div().flex_shrink_0().size(space(6.0)).rounded_full().bg(
                                if connected {
                                    colors.primary
                                } else {
                                    colors.muted
                                },
                            ))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(if connected {
                                        colors.primary
                                    } else {
                                        colors.foreground
                                    })
                                    .child(label),
                            ),
                    )
                    .child(
                        div()
                            .id("status-footer-context")
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(colors.muted)
                            .child(context),
                    )
                    .when(workspace, |row| {
                        row.when(wide, |row| {
                            row.when_some(dimensions, |row, value| {
                                row.child(div().text_color(colors.muted).child(value))
                            })
                            .when_some(traffic, |row, value| {
                                row.child(div().text_color(colors.muted).child(value))
                            })
                        })
                        .child(session_info)
                    })
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .gap(space(4.0))
                            .when(workspace, |row| {
                                row.when_some(favorite_toggle, |row, (id, open)| {
                                    row.child(
                                        tool(
                                            "sftp-favorites-panel-toggle",
                                            AppIcon::Folder,
                                            i18n::string("sftp.tooltips.show_remote_favorites"),
                                            open,
                                        )
                                        .on_click(
                                            move |_, _, cx| {
                                                favorites.update(cx, |controller, cx| {
                                                    controller.toggle_favorites_panel(id, cx)
                                                })
                                            },
                                        ),
                                    )
                                })
                                .when(session.is_some() && !local, |row| {
                                    row.child(
                                        tool(
                                            "session-monitor-panel-toggle",
                                            AppIcon::Computer,
                                            i18n::string("session.footer.tooltips.monitor_panel"),
                                            self.controllers.session.read(cx).side_panel_open()
                                                && self
                                                    .controllers
                                                    .session
                                                    .read(cx)
                                                    .side_panel_view()
                                                    == SessionSidePanelView::Monitor,
                                        )
                                        .on_click(
                                            move |_, _, cx| {
                                                monitor.update(cx, |this, cx| {
                                                    this.toggle_session_side_panel(cx);
                                                    cx.notify();
                                                })
                                            },
                                        ),
                                    )
                                    .child(
                                        tool(
                                            "session-agent-panel-toggle",
                                            AppIcon::Sparkles,
                                            i18n::string("session.footer.tooltips.agent_panel"),
                                            self.controllers.agent.read(cx).panel_open(),
                                        )
                                        .on_click(
                                            move |_, _, cx| {
                                                agent.update(cx, |this, cx| {
                                                    this.toggle_session_agent_panel(cx);
                                                    cx.notify();
                                                })
                                            },
                                        ),
                                    )
                                })
                                .when(!local, |row| {
                                    row.child(
                                        tool(
                                            "sftp-progress-center-toggle",
                                            AppIcon::ArrowUpDown,
                                            sftp_tooltip,
                                            progress_open,
                                        )
                                        .on_click(
                                            move |_, _, cx| {
                                                if let Some(id) = sftp_id {
                                                    progress.update(cx, |controller, cx| {
                                                        controller.set_tab_progress_visible(
                                                            id,
                                                            !progress_open,
                                                        );
                                                        cx.notify();
                                                    });
                                                } else if let Some(id) = session_id {
                                                    session_progress.update(
                                                        cx,
                                                        |controller, cx| {
                                                            controller.set_sftp_progress_visible(
                                                                id,
                                                                !progress_open,
                                                            );
                                                            cx.notify();
                                                        },
                                                    );
                                                }
                                            },
                                        ),
                                    )
                                })
                            })
                            .child(render_notification_center_popover(window, cx)),
                    ),
            )
    }

    fn render_new_host_action(&self, cx: &App) -> AnyElement {
        let controller = self.controllers.hosts.clone();
        new_host_action(cx, move |_, cx| {
            controller.update(cx, |controller, cx| {
                controller.request_new_profile_editor(cx)
            })
        })
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use gpui_kit::{Render, TestAppContext};
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn local_terminal_footer_targets_use_the_shell_file_name() {
        assert_eq!(local_shell_file_name("/bin/zsh"), "zsh");
        assert_eq!(local_shell_file_name("  /usr/local/bin/fish  "), "fish");
        #[cfg(windows)]
        assert_eq!(
            local_shell_file_name(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            "pwsh.exe"
        );
    }

    #[test]
    fn footer_projects_ssh_local_and_standalone_sftp_metrics() {
        let session = SessionTabSnapshot::new("fixture".into(), None, SessionPurpose::Terminal)
            .with_connection(true, false, false)
            .with_terminal(120, 32, 2048, 1024);
        let ssh = footer_metrics(Some(&session), None, false);
        assert_eq!(ssh.dimensions.as_deref(), Some("120 × 32"));
        assert_eq!(ssh.traffic.as_deref(), Some("↑ 1.0 KB · ↓ 2.0 KB"));

        let local = footer_metrics(Some(&session), None, true);
        assert_eq!(local.dimensions.as_deref(), Some("120 × 32"));
        assert!(local.traffic.is_none());

        let sftp = footer_metrics(None, Some((512, 4096)), false);
        assert!(sftp.dimensions.is_none());
        assert_eq!(sftp.traffic.as_deref(), Some("↑ 512 B · ↓ 4.0 KB"));
    }

    #[test]
    fn delayed_topbar_action_resolves_reordered_tab_by_id() {
        let target = TabId::new(20);
        let mut tabs = TabRegistry::from_tabs([
            TabState::new_hosts(TabId::new(10)),
            TabState::new_hosts(target),
            TabState::new_hosts(TabId::new(30)),
        ]);

        tabs.move_to(2, 0);

        assert_eq!(topbar_action_index(&tabs, target), Some(2));
    }

    struct AddButtonHost {
        opens: Rc<Cell<usize>>,
    }

    impl Render for AddButtonHost {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            // Keep the add command independently operable inside native chrome.
            topbar_drag_surface()
                .test_support()
                .w_full()
                .h(px(56.0))
                .relative()
                .child(
                    div()
                        .absolute()
                        .top(px(10.0))
                        .left(px(10.0))
                        .child(topbar_add_button(cx.entity(), ScrollHandle::new())),
                )
        }
    }

    impl TopbarHost for AddButtonHost {
        fn handle_topbar_action(
            &mut self,
            action: TopbarAction,
            _window: &mut Window,
            _cx: &mut Context<Self>,
        ) {
            if matches!(action, TopbarAction::OpenHosts) {
                self.opens.set(self.opens.get() + 1);
            }
        }
    }

    struct FabHost {
        creates: Rc<Cell<usize>>,
        connections: Rc<Cell<usize>>,
    }

    impl Render for FabHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let connections = self.connections.clone();
            let creates = self.creates.clone();
            div()
                .size_full()
                .relative()
                .child(
                    Button::new("underlying-host")
                        .size_full()
                        .label("Connect")
                        .on_click(move |_, _, _| connections.set(connections.get() + 1)),
                )
                .child(
                    div()
                        .absolute()
                        .right_4()
                        .bottom_4()
                        .child(new_host_action(cx, move |_, _| {
                            creates.set(creates.get() + 1)
                        })),
                )
        }
    }

    #[gpui_kit::test]
    fn floating_create_does_not_connect_the_host_beneath_it(cx: &mut TestAppContext) {
        use gpui_kit::test::TestWindowExt as _;
        cx.update(gpui_kit::init);
        let creates = Rc::new(Cell::new(0));
        let connections = Rc::new(Cell::new(0));
        let created = creates.clone();
        let connected = connections.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|_| FabHost {
                creates: created,
                connections: connected,
            });
            Root::new(view, window, cx)
        });
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click("hosts-create-fab", cx);
            assert_eq!(creates.get(), 1);
            assert_eq!(connections.get(), 0);
            window.click("underlying-host", cx);
            assert_eq!(connections.get(), 1);
        });
    }

    #[gpui_kit::test]
    #[cfg(target_os = "windows")]
    fn caption_presses_reach_the_native_window_handler(cx: &mut TestAppContext) {
        use gpui_kit::{InputEvent as _, test::TestWindowExt as _};
        cx.update(gpui_kit::init);
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|_| AddButtonHost {
                opens: Rc::new(Cell::new(0)),
            });
            Root::new(view, window, cx)
        });
        cx.update(|window, cx| {
            window.render_frame(cx);
            let bounds = window.find("topbar-window-drag").bounds();
            assert_eq!(bounds.origin, point(px(0.0), px(0.0)));
            assert_eq!(bounds.size.width, window.viewport_size().width);
            assert_eq!(bounds.size.height, px(56.0));
            for click_count in [1, 2] {
                let down = window.dispatch_event(
                    MouseDownEvent {
                        position: point(px(300.0), px(24.0)),
                        button: MouseButton::Left,
                        click_count,
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                );
                assert!(down.propagate, "caption press must reach DefWindowProc");
                let up = window.dispatch_event(
                    MouseUpEvent {
                        position: point(px(300.0), px(24.0)),
                        button: MouseButton::Left,
                        click_count,
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                );
                assert!(up.propagate, "caption release must reach DefWindowProc");
            }
        });
    }

    #[gpui_kit::test]
    fn the_add_button_opens_a_new_connection(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let opens = Rc::new(Cell::new(0usize));
        let opens_for_view = opens.clone();
        let (_root, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|_| AddButtonHost {
                opens: opens_for_view,
            });
            Root::new(view, window, cx)
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        cx.simulate_click(point(px(24.0), px(24.0)), Default::default());
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        assert_eq!(opens.get(), 1);
    }
}
