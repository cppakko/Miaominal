use crate::ui::components::md3_spinner;
use crate::ui::shell::state::SessionSidePanelView;
use crate::ui::shell::state::SessionFailureStatus;
use crate::ui::{
    components::{editor_button, fab_icon_button},
    i18n,
};
use gpui::{StatefulInteractiveElement, linear_color_stop, linear_gradient};
use gpui_component::{
    ActiveTheme, Disableable,
    plot::{
        Grid, IntoPlot, Plot, StrokeStyle,
        scale::{Scale, ScaleLinear, ScalePoint},
        shape::Area,
    },
    tab::{Tab, TabBar},
};
use miaominal_settings::TerminalRightClickBehavior;

use super::super::metrics::TERMINAL_PANEL_BORDER;
use super::super::pages::shell_empty_state;
use super::super::workspace::PaneLayout;
use super::super::*;
use super::chrome::{status_indicator, tab_status_indicator_color};

struct SessionFailureView {
    title: String,
    summary: String,
    error: String,
    failure_status: Option<SessionFailureStatus>,
    profile_id: String,
    purpose: SessionPurpose,
    tab_id: usize,
}

struct MonitorChartCardConfig<'a> {
    title: String,
    value: String,
    detail: Option<String>,
    history: &'a [MonitorChartPoint],
    y_max: f64,
    y_ticks: Vec<f64>,
    y_tick_labels: [String; 3],
    palette_index: usize,
    mode: MonitorChartCardMode,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MonitorChartCardMode {
    Full,
    Compact,
}

fn pane_drop_zone_style(
    style: gpui::StyleRefinement,
    zone: super::super::panes::PaneTabDropZone,
) -> gpui::StyleRefinement {
    let roles = miaominal_settings::current_theme().material.roles;
    let mut refined = style;
    refined.background = Some(color_with_alpha(roles.primary, 0x33).into());
    refined.border_color = Some(rgb(roles.primary).into());
    match zone {
        super::super::panes::PaneTabDropZone::Center => {
            refined.border_widths.top = Some(px(2.0).into());
            refined.border_widths.right = Some(px(2.0).into());
            refined.border_widths.bottom = Some(px(2.0).into());
            refined.border_widths.left = Some(px(2.0).into());
        }
        super::super::panes::PaneTabDropZone::Up => {
            refined.border_widths.bottom = Some(px(2.0).into());
        }
        super::super::panes::PaneTabDropZone::Down => {
            refined.border_widths.top = Some(px(2.0).into());
        }
        super::super::panes::PaneTabDropZone::Left => {
            refined.border_widths.right = Some(px(2.0).into());
        }
        super::super::panes::PaneTabDropZone::Right => {
            refined.border_widths.left = Some(px(2.0).into());
        }
    }
    refined
}

fn session_summary(tab: &TabState, sessions: &[SessionProfile]) -> String {
    let Some(session) = tab.as_session() else {
        return String::new();
    };

    if let Some(profile) = sessions
        .iter()
        .find(|profile| profile.id == session.profile_id)
    {
        return format!("{}@{}:{}", profile.username, profile.host, profile.port);
    }

    if let Some(profile) = session.pending_profile.as_ref() {
        return format!("{}@{}:{}", profile.username, profile.host, profile.port);
    }

    tab.title.clone()
}

fn terminal_pane_surface_id(pane_id: super::super::panes::PaneId) -> SharedString {
    SharedString::from(format!("terminal-pane-surface-{}", pane_id.raw()))
}

fn activate_terminal_menu_target(
    this: &mut AppView,
    target_pane_id: Option<super::super::panes::PaneId>,
    window: &mut Window,
    cx: &mut Context<AppView>,
) {
    if let Some(pane_id) = target_pane_id {
        this.set_active_pane(pane_id, window, cx);
    }
}

fn build_terminal_context_menu(
    menu: PopupMenu,
    entity: Entity<AppView>,
    has_selection: bool,
    target_pane_id: Option<super::super::panes::PaneId>,
    _window: &mut Window,
    _cx: &mut App,
) -> PopupMenu {
    let copy = entity.clone();
    let paste = entity.clone();
    let split_right = entity.clone();
    let split_down = entity.clone();
    let split_left = entity.clone();
    let split_up = entity.clone();
    let sftp_entry = entity.clone();
    let close = entity.clone();

    menu.item(
        PopupMenuItem::new(i18n::string("workspace.menu.copy"))
            .disabled(!has_selection)
            .on_click(move |_, window, cx| {
                let entity = copy.clone();
                entity.update(cx, |this, cx| {
                    activate_terminal_menu_target(this, target_pane_id, window, cx);
                    this.copy_terminal_selection(cx);
                    window.focus(
                        &this.workspace_state.workspace.active_pane.terminal_focus,
                        cx,
                    );
                    this.sync_terminal_focus_reporting(window, cx);
                });
            }),
    )
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.paste")).on_click(move |_, window, cx| {
            let entity = paste.clone();
            entity.update(cx, |this, cx| {
                activate_terminal_menu_target(this, target_pane_id, window, cx);
                this.paste_into_terminal(cx);
                window.focus(
                    &this.workspace_state.workspace.active_pane.terminal_focus,
                    cx,
                );
                this.sync_terminal_focus_reporting(window, cx);
            });
        }),
    )
    .item(PopupMenuItem::separator())
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.split_right")).on_click(
            move |_, window, cx| {
                let entity = split_right.clone();
                entity.update(cx, |this, cx| {
                    activate_terminal_menu_target(this, target_pane_id, window, cx);
                    this.split_active_pane(
                        super::super::workspace::SplitDirection::Right,
                        window,
                        cx,
                    )
                });
            },
        ),
    )
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.split_down")).on_click(
            move |_, window, cx| {
                let entity = split_down.clone();
                entity.update(cx, |this, cx| {
                    activate_terminal_menu_target(this, target_pane_id, window, cx);
                    this.split_active_pane(
                        super::super::workspace::SplitDirection::Down,
                        window,
                        cx,
                    )
                });
            },
        ),
    )
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.split_left")).on_click(
            move |_, window, cx| {
                let entity = split_left.clone();
                entity.update(cx, |this, cx| {
                    activate_terminal_menu_target(this, target_pane_id, window, cx);
                    this.split_active_pane(
                        super::super::workspace::SplitDirection::Left,
                        window,
                        cx,
                    )
                });
            },
        ),
    )
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.split_up")).on_click(
            move |_, window, cx| {
                let entity = split_up.clone();
                entity.update(cx, |this, cx| {
                    activate_terminal_menu_target(this, target_pane_id, window, cx);
                    this.split_active_pane(super::super::workspace::SplitDirection::Up, window, cx)
                });
            },
        ),
    )
    .item(PopupMenuItem::separator())
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.open_sftp_tab")).on_click(
            move |_, window, cx| {
                let entity = sftp_entry.clone();
                entity.update(cx, |this, cx| {
                    activate_terminal_menu_target(this, target_pane_id, window, cx);
                    this.open_sftp_tab_for_session(
                        this.workspace_state.workspace.active_tab,
                        window,
                        cx,
                    )
                });
            },
        ),
    )
    .item(PopupMenuItem::separator())
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.close_pane")).on_click(
            move |_, window, cx| {
                let entity = close.clone();
                entity.update(cx, |this, cx| {
                    activate_terminal_menu_target(this, target_pane_id, window, cx);
                    this.close_active_pane(window, cx)
                });
            },
        ),
    )
}

const SESSION_MONITOR_PANEL_WIDTH: f32 = 340.0;
const SESSION_MONITOR_CHART_HEIGHT: f32 = 60.0;
const SESSION_MONITOR_SAMPLE_INTERVAL_SECS: usize = 2;
const SESSION_MONITOR_PERCENT_MIN_Y_MAX: f64 = 2.0;
const WORKSPACE_SIDE_PANEL_GAP: f32 = 12.0;
const WORKSPACE_SIDE_PANEL_SLIDE_OFFSET: f32 = 28.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkspaceSidePanelDock {
    Left,
    Right,
}

fn workspace_side_panel_render_state(
    desired_visible: bool,
    visible: &mut bool,
    transition: &mut Option<WorkspaceSidePanelTransition>,
    window: &mut Window,
) -> Option<f32> {
    let now = std::time::Instant::now();
    let duration = OVERLAY_ENTER_DURATION;

    match desired_visible {
        true => match transition {
            Some(current) if current.phase == WorkspaceSidePanelTransitionPhase::Exiting => {
                *transition = Some(WorkspaceSidePanelTransition {
                    phase: WorkspaceSidePanelTransitionPhase::Entering,
                    started_at: now,
                    ..*current
                });
            }
            Some(_) => {}
            None => {
                if !*visible {
                    *visible = true;
                    *transition = Some(WorkspaceSidePanelTransition {
                        phase: WorkspaceSidePanelTransitionPhase::Entering,
                        started_at: now,
                        duration,
                    });
                }
            }
        },
        false => {
            if *visible {
                match transition {
                    Some(current)
                        if current.phase == WorkspaceSidePanelTransitionPhase::Entering =>
                    {
                        *transition = Some(WorkspaceSidePanelTransition {
                            phase: WorkspaceSidePanelTransitionPhase::Exiting,
                            started_at: now,
                            ..*current
                        });
                    }
                    None => {
                        *transition = Some(WorkspaceSidePanelTransition {
                            phase: WorkspaceSidePanelTransitionPhase::Exiting,
                            started_at: now,
                            duration,
                        });
                    }
                    _ => {}
                }
            }
        }
    }

    if let Some(current) = *transition {
        let duration_seconds = current.duration.as_secs_f32();
        if duration_seconds <= f32::EPSILON {
            *transition = None;
            *visible = current.phase == WorkspaceSidePanelTransitionPhase::Entering;
            return visible.then_some(1.0);
        }

        let elapsed = now.saturating_duration_since(current.started_at);
        let progress = (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0);
        let eased = progress * progress * (3.0 - 2.0 * progress);

        if progress >= 1.0 {
            *transition = None;
            *visible = current.phase == WorkspaceSidePanelTransitionPhase::Entering;
            return visible.then_some(1.0);
        }

        window.request_animation_frame();

        return Some(match current.phase {
            WorkspaceSidePanelTransitionPhase::Entering => eased,
            WorkspaceSidePanelTransitionPhase::Exiting => 1.0 - eased,
        });
    }

    *visible = desired_visible;
    desired_visible.then_some(1.0)
}

fn render_workspace_side_panel(
    panel: gpui::AnyElement,
    panel_width: f32,
    visibility: f32,
    dock: WorkspaceSidePanelDock,
) -> gpui::AnyElement {
    let wrapper_width = (panel_width + WORKSPACE_SIDE_PANEL_GAP) * visibility;
    let slide_offset = (1.0 - visibility) * WORKSPACE_SIDE_PANEL_SLIDE_OFFSET;
    let opacity = 0.24 + visibility * 0.76;

    let anchored_panel = match dock {
        WorkspaceSidePanelDock::Left => div()
            .absolute()
            .top(px(0.0))
            .left(px(-slide_offset))
            .bottom(px(0.0))
            .w(px(panel_width))
            .opacity(opacity)
            .child(panel),
        WorkspaceSidePanelDock::Right => div()
            .absolute()
            .top(px(0.0))
            .right(px(-slide_offset))
            .bottom(px(0.0))
            .w(px(panel_width))
            .opacity(opacity)
            .child(panel),
    };

    div()
        .relative()
        .h_full()
        .w(px(wrapper_width))
        .min_w(px(0.0))
        .flex_shrink_0()
        .overflow_hidden()
        .child(anchored_panel)
        .into_any_element()
}

fn tail_history(history: &[MonitorChartPoint], limit: usize) -> Vec<MonitorChartPoint> {
    let start = history.len().saturating_sub(limit);
    history[start..].to_vec()
}

fn session_snippet_package_card(
    title: String,
    snippet_count: usize,
    is_selected: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let palette = group_accent_palette(&title, &material);
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );
    let count = snippet_count.to_string();
    let count_label = if snippet_count == 1 {
        i18n::string_args("snippets.package_card.count_one", &[("count", &count)])
    } else {
        i18n::string_args("snippets.package_card.count_other", &[("count", &count)])
    };
    let icon = miaominal_core::snippet::package_initials(&title)
        .unwrap_or_else(|| i18n::string("snippets.package_card.fallback_icon"));

    card_surface(
        if is_selected {
            palette.accent_container
        } else {
            roles.surface_container_high
        },
        16.0,
    )
    .w_full()
    .cursor_pointer()
    .p_3()
    .on_mouse_down(MouseButton::Left, move |_, window: &mut Window, cx| {
        on_click(window, cx);
    })
    .child(
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                h_flex()
                    .min_w(px(0.0))
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .size(px(36.0))
                            .rounded(px(12.0))
                            .bg(rgb(if is_selected {
                                palette.accent
                            } else {
                                roles.surface_container_low
                            }))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(miaominal_settings::FontSize::Body.scaled())
                            .text_color(rgb(if is_selected {
                                palette.on_accent
                            } else {
                                palette.accent
                            }))
                            .child(icon),
                    )
                    .child(
                        v_flex()
                            .min_w(px(0.0))
                            .gap_1()
                            .child(
                                div()
                                    .text_size(miaominal_settings::FontSize::Input.scaled())
                                    .text_color(rgb(roles.on_surface))
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_size(miaominal_settings::FontSize::Body.scaled())
                                    .text_color(rgb(text_muted))
                                    .child(count_label),
                            ),
                    ),
            ),
    )
}

#[derive(Clone, IntoPlot)]
struct MonitorAreaChart {
    data: Vec<MonitorChartPoint>,
    y_max: f64,
    y_ticks: Vec<f64>,
    palette_index: usize,
}

impl MonitorAreaChart {
    fn new(
        data: Vec<MonitorChartPoint>,
        y_max: f64,
        y_ticks: Vec<f64>,
        palette_index: usize,
    ) -> Self {
        Self {
            data,
            y_max,
            y_ticks,
            palette_index,
        }
    }
}

impl Plot for MonitorAreaChart {
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        if self.data.is_empty() {
            return;
        }

        let width = bounds.size.width.as_f32();
        let height = bounds.size.height.as_f32();
        let y_max = self.y_max.max(1.0);
        let x = ScalePoint::new(
            self.data
                .iter()
                .map(|point| point.label.clone())
                .collect::<Vec<_>>(),
            vec![0.0, width],
        );
        let y = ScaleLinear::new(vec![0.0, y_max], vec![height - 3.0, 3.0]);
        let grid_ticks = self
            .y_ticks
            .iter()
            .filter_map(|tick| y.tick(tick))
            .collect::<Vec<_>>();
        let stroke = match self.palette_index % 5 {
            0 => cx.theme().chart_1,
            1 => cx.theme().chart_2,
            2 => cx.theme().chart_3,
            3 => cx.theme().chart_4,
            _ => cx.theme().chart_5,
        };
        let fill = linear_gradient(
            0.0,
            linear_color_stop(stroke.opacity(0.34), 1.0),
            linear_color_stop(cx.theme().background.opacity(0.04), 0.0),
        );

        if !grid_ticks.is_empty() {
            Grid::new()
                .y(grid_ticks)
                .stroke(cx.theme().border.opacity(0.72))
                .dash_array(&[px(4.0), px(2.0)])
                .paint(&bounds, window);
        }

        Area::new()
            .data(self.data.clone())
            .x(move |point| x.tick(&point.label))
            .y0(height - 3.0)
            .y1(move |point| y.tick(&point.value))
            .fill(fill)
            .stroke(stroke)
            .stroke_style(StrokeStyle::Linear)
            .paint(&bounds, window);
    }
}

fn format_percentage(value: f64) -> String {
    format!("{:.0}%", value.max(0.0))
}

fn format_rate_label(kib_per_second: f64) -> String {
    if kib_per_second >= 1024.0 {
        format!("{:.1} MB/s", kib_per_second / 1024.0)
    } else {
        format!("{:.0} KB/s", kib_per_second.max(0.0))
    }
}

fn format_load_label(value: f64) -> String {
    format!("{:.2}", value.max(0.0))
}

fn nice_chart_max(peak: f64, minimum: f64) -> f64 {
    let peak = peak.max(minimum).max(1.0);
    let exponent = peak.log10().floor();
    let magnitude = 10f64.powf(exponent);
    let normalized = peak / magnitude;
    let nice = if normalized <= 1.0 {
        1.0
    } else if normalized <= 2.0 {
        2.0
    } else if normalized <= 5.0 {
        5.0
    } else {
        10.0
    };

    nice * magnitude
}

fn build_chart_ticks(max: f64) -> Vec<f64> {
    vec![0.0, max / 2.0, max]
}

fn build_chart_tick_labels<F>(ticks: &[f64], formatter: F) -> [String; 3]
where
    F: Fn(f64) -> String,
{
    let bottom = ticks.first().copied().unwrap_or(0.0);
    let middle = ticks.get(1).copied().unwrap_or(bottom);
    let top = ticks.last().copied().unwrap_or(middle);

    [formatter(top), formatter(middle), formatter(bottom)]
}

fn estimate_monitor_axis_label_width(labels: &[String; 3], font_size: f32) -> f32 {
    let max_chars = labels
        .iter()
        .map(|label| label.chars().count())
        .max()
        .unwrap_or(0) as f32;

    (max_chars * font_size * 0.72 + font_size).clamp(20.0, 44.0)
}

fn format_monitor_time_axis_label(total_seconds: usize) -> String {
    if total_seconds == 0 {
        return i18n::string("workspace.panel.monitor.axis_now");
    }

    if total_seconds < 60 {
        return i18n::string_args(
            "workspace.panel.monitor.axis_seconds_ago",
            &[("seconds", &total_seconds.to_string())],
        );
    }

    let minutes = ((total_seconds + 30) / 60).max(1);
    i18n::string_args(
        "workspace.panel.monitor.axis_minutes_ago",
        &[("minutes", &minutes.to_string())],
    )
}

fn build_monitor_time_axis_labels(point_count: usize) -> [String; 3] {
    let span_seconds = point_count
        .saturating_sub(1)
        .saturating_mul(SESSION_MONITOR_SAMPLE_INTERVAL_SECS);
    let midpoint_seconds = span_seconds / 2;

    [
        if span_seconds > 0 {
            format_monitor_time_axis_label(span_seconds)
        } else {
            String::new()
        },
        if midpoint_seconds > 0 && midpoint_seconds < span_seconds {
            format_monitor_time_axis_label(midpoint_seconds)
        } else {
            String::new()
        },
        format_monitor_time_axis_label(0),
    ]
}

fn format_monitor_history_window(history_limit: usize) -> String {
    let span_seconds = history_limit.saturating_mul(SESSION_MONITOR_SAMPLE_INTERVAL_SECS);
    let label = if span_seconds >= 60 {
        i18n::string_args(
            "workspace.panel.monitor.window_minutes",
            &[("minutes", &(span_seconds / 60).max(1).to_string())],
        )
    } else {
        i18n::string_args(
            "workspace.panel.monitor.window_seconds",
            &[("seconds", &span_seconds.max(1).to_string())],
        )
    };

    i18n::string_args(
        "workspace.panel.monitor.history_window",
        &[("window", &label)],
    )
}

impl AppView {
    fn hide_preserved_history_popup(&mut self, tab_id: usize, cx: &mut Context<Self>) {
        let Some(tab_index) = self
            .workspace_state
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
        else {
            return;
        };
        let Some(session) = self.workspace_state.tabs[tab_index].as_session_mut() else {
            return;
        };

        session.hide_preserved_history_popup();
        cx.notify();
    }

    fn reconnect_session_tab(
        &mut self,
        tab_id: usize,
        profile_id: &str,
        write_marker: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_index) = self
            .workspace_state
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
        else {
            return;
        };
        let profile = self
            .data
            .sessions
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned();

        if let Some(session) = self.workspace_state.tabs[tab_index].as_session_mut()
            && let Some(profile) = profile
        {
            session.commands = None;
            session.pending_profile = Some(profile);
            session.set_connection_state(SessionConnectionState::Connecting);
            session.reconnect_attempt = 0;
            if write_marker {
                session.terminal.push_text(&format!(
                    "{}\r\n",
                    i18n::string("session.terminal.reconnecting_marker")
                ));
            }
        }

        cx.notify();
    }

    fn render_session_placeholder(
        &self,
        tab: &TabState,
        rounded: bool,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let session = tab.as_session()?;
        if !session.uses_blocking_placeholder() {
            return None;
        }

        let summary = session_summary(tab, &self.data.sessions);

        match &session.connection_state {
            SessionConnectionState::Connecting => Some(self.render_session_connecting_surface(
                if session.purpose == SessionPurpose::PortForwarding {
                    i18n::string("session.workspace.connecting_forwarding_rule")
                } else {
                    i18n::string("session.workspace.connecting_to_host")
                },
                tab.status.clone(),
                summary,
                rounded,
            )),
            SessionConnectionState::Failed { error, status } => {
                Some(self.render_session_failure_surface(
                    SessionFailureView {
                        title: if session.purpose == SessionPurpose::PortForwarding {
                            i18n::string("session.workspace.forwarding_connection_failed")
                        } else {
                            i18n::string("session.workspace.connection_failed")
                        },
                        summary,
                        error: error.clone(),
                        failure_status: *status,
                        profile_id: session.profile_id.clone(),
                        purpose: session.purpose,
                        tab_id: tab.id,
                    },
                    rounded,
                    cx,
                ))
            }
            SessionConnectionState::Reconnecting { error, attempt } => {
                Some(self.render_session_reconnecting_surface(
                    summary,
                    error.clone(),
                    *attempt,
                    tab.id,
                    rounded,
                    cx,
                ))
            }
            SessionConnectionState::Ready => None,
            SessionConnectionState::Disconnected => Some(self.render_session_disconnected_surface(
                summary,
                session.profile_id.clone(),
                session.purpose,
                tab.id,
                rounded,
                cx,
            )),
        }
    }

    fn render_session_history_banner(
        &self,
        tab: &TabState,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let session = tab.as_session()?;
        if !session.preserves_terminal_history() {
            return None;
        }

        let summary = session_summary(tab, &self.data.sessions);
        let popup_hidden = session.preserved_history_popup_hidden();

        match &session.connection_state {
            SessionConnectionState::Failed { error, status } => Some(if popup_hidden {
                self.render_session_reconnect_fab(session.profile_id.clone(), true, tab.id, cx)
            } else {
                self.render_session_failure_banner(
                    SessionFailureView {
                        title: i18n::string("session.workspace.connection_failed"),
                        summary,
                        error: error.clone(),
                        failure_status: *status,
                        profile_id: session.profile_id.clone(),
                        purpose: session.purpose,
                        tab_id: tab.id,
                    },
                    cx,
                )
            }),
            SessionConnectionState::Disconnected => Some(if popup_hidden {
                self.render_session_reconnect_fab(session.profile_id.clone(), false, tab.id, cx)
            } else {
                self.render_session_disconnected_banner(
                    summary,
                    session.profile_id.clone(),
                    session.purpose,
                    tab.id,
                    cx,
                )
            }),
            SessionConnectionState::Connecting
            | SessionConnectionState::Ready
            | SessionConnectionState::Reconnecting { .. } => None,
        }
    }

    fn render_session_disconnected_surface(
        &self,
        summary: String,
        profile_id: String,
        purpose: SessionPurpose,
        tab_id: usize,
        rounded: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let weak = cx.entity().downgrade();
        let is_port_forward = purpose == SessionPurpose::PortForwarding;
        let profile_exists = self.data.sessions.iter().any(|p| p.id == profile_id);

        div()
            .size_full()
            .when(rounded, |this| this.rounded(px(16.0)))
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(roles.background))
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(560.0))
                    .items_center()
                    .gap_4()
                    .px_6()
                    .py_8()
                    .child(
                        div()
                            .size(px(56.0))
                            .rounded(px(18.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(color_with_alpha(text_muted, 0x18))
                            .child(
                                Icon::new(IconName::Minus)
                                    .large()
                                    .text_color(rgb(roles.on_surface_variant)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(miaominal_settings::FontSize::Display.scaled())
                            .text_color(rgb(roles.on_surface))
                            .child(i18n::string("session.workspace.session_closed")),
                    )
                    .when(!summary.is_empty(), |this| {
                        this.child(
                            div()
                                .text_size(miaominal_settings::FontSize::Input.scaled())
                                .text_color(rgb(roles.on_surface_variant))
                                .child(summary),
                        )
                    })
                    .child(
                        h_flex()
                            .gap_3()
                            .when(!is_port_forward && profile_exists, |this| {
                                this.child(icon_button(
                                    AppIcon::Rotate,
                                    36.0,
                                    12.0,
                                    Some(roles.primary),
                                    Some(roles.on_primary),
                                    None,
                                    {
                                        let weak = weak.clone();
                                        let profile_id = profile_id.clone();
                                        move |_window, cx| {
                                            weak.update(cx, |this, cx| {
                                                let Some(tab_index) = this
                                                    .workspace_state
                                                    .tabs
                                                    .iter()
                                                    .position(|t| t.id == tab_id)
                                                else {
                                                    return;
                                                };
                                                let profile = this
                                                    .data
                                                    .sessions
                                                    .iter()
                                                    .find(|p| p.id == profile_id)
                                                    .cloned();
                                                if let Some(session) = this.workspace_state.tabs
                                                    [tab_index]
                                                    .as_session_mut()
                                                    && let Some(profile) = profile
                                                {
                                                    session.commands = None;
                                                    session.pending_profile = Some(profile);
                                                    session.set_connection_state(
                                                        SessionConnectionState::Connecting,
                                                    );
                                                    session.reconnect_attempt = 0;
                                                }
                                                cx.notify();
                                            })
                                            .ok();
                                        }
                                    },
                                ))
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_session_connecting_surface(
        &self,
        title: String,
        status: String,
        summary: String,
        rounded: bool,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );

        div()
            .size_full()
            .when(rounded, |this| this.rounded(px(16.0)))
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(roles.background))
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(560.0))
                    .items_center()
                    .gap_4()
                    .px_6()
                    .py_8()
                    .child(md3_spinner(64.0))
                    .child(
                        div()
                            .text_size(miaominal_settings::FontSize::Display.scaled())
                            .text_color(rgb(roles.on_surface))
                            .child(title),
                    )
                    .when(!summary.is_empty(), |this| {
                        this.child(
                            div()
                                .text_size(miaominal_settings::FontSize::Input.scaled())
                                .text_color(rgb(roles.on_surface_variant))
                                .child(summary),
                        )
                    })
                    .when(!status.is_empty(), |this| {
                        this.child(
                            div()
                                .w_full()
                                .px_4()
                                .py_3()
                                .rounded(px(16.0))
                                .bg(color_with_alpha(text_muted, 0x10))
                                .child(
                                    div()
                                        .text_size(miaominal_settings::FontSize::Input.scaled())
                                        .text_color(rgb(roles.on_surface_variant))
                                        .child(status),
                                ),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_session_reconnecting_surface(
        &self,
        summary: String,
        error: String,
        attempt: u32,
        tab_id: usize,
        rounded: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        const MAX_RECONNECT_ATTEMPTS: u32 = 10;
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let weak = cx.entity().downgrade();
        let error_for_cancel = error.clone();
        div()
            .size_full()
            .when(rounded, |this| this.rounded(px(16.0)))
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(roles.background))
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(560.0))
                    .items_center()
                    .gap_4()
                    .px_6()
                    .py_8()
                    .child(md3_spinner(64.0))
                    .child(
                        div()
                            .text_size(miaominal_settings::FontSize::Display.scaled())
                            .text_color(rgb(roles.on_surface))
                            .child(i18n::string("session.workspace.reconnecting")),
                    )
                    .when(!summary.is_empty(), |this| {
                        this.child(
                            div()
                                .text_size(miaominal_settings::FontSize::Input.scaled())
                                .text_color(rgb(roles.on_surface_variant))
                                .child(summary),
                        )
                    })
                    .child(
                        div()
                            .text_size(miaominal_settings::FontSize::Input.scaled())
                            .text_color(rgb(text_muted))
                            .child(i18n::string_args(
                                "session.workspace.reconnect_attempt",
                                &[
                                    ("attempt", &attempt.to_string()),
                                    ("max", &MAX_RECONNECT_ATTEMPTS.to_string()),
                                ],
                            )),
                    )
                    .child(
                        div().w_full().p_4().child(
                            div()
                                .text_size(miaominal_settings::FontSize::Subheading.scaled())
                                .text_color(rgb(roles.on_surface))
                                .child(error.clone()),
                        ),
                    )
                    .child(icon_button(
                        AppIcon::Close,
                        36.0,
                        12.0,
                        None,
                        None,
                        None,
                        move |_window, cx| {
                            weak.update(cx, |this, cx| {
                                let Some(tab_index) = this
                                    .workspace_state
                                    .tabs
                                    .iter()
                                    .position(|t| t.id == tab_id)
                                else {
                                    return;
                                };
                                if let Some(session) =
                                    this.workspace_state.tabs[tab_index].as_session_mut()
                                {
                                    session.reconnect_task = None;
                                    session.reconnect_attempt = 0;
                                    session.set_connection_state(SessionConnectionState::Failed {
                                        error: error_for_cancel.clone(),
                                        status: None,
                                    });
                                }
                                cx.notify();
                            })
                            .ok();
                        },
                    )),
            )
            .into_any_element()
    }

    fn render_session_failure_surface(
        &self,
        failure: SessionFailureView,
        rounded: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let SessionFailureView {
            title,
            summary,
            error,
            failure_status,
            profile_id,
            purpose,
            tab_id,
        } = failure;

        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let weak = cx.entity().downgrade();
        let profile_id_retry = profile_id.clone();
        let profile_id_edit = profile_id.clone();
        let is_port_forward = purpose == SessionPurpose::PortForwarding;
        let profile_exists = self.data.sessions.iter().any(|p| p.id == profile_id);

        div()
            .size_full()
            .when(rounded, |this| this.rounded(px(16.0)))
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(roles.background))
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(620.0))
                    .items_center()
                    .gap_4()
                    .px_6()
                    .py_8()
                    .child(
                        div()
                            .size(px(56.0))
                            .rounded(px(18.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(color_with_alpha(roles.error, 0x18))
                            .child(
                                Icon::new(IconName::CircleX)
                                    .large()
                                    .text_color(rgb(roles.error)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(miaominal_settings::FontSize::Display.scaled())
                            .text_color(rgb(roles.on_surface))
                            .child(title),
                    )
                    .when(!summary.is_empty(), |this| {
                        this.child(
                            div()
                                .text_size(miaominal_settings::FontSize::Input.scaled())
                                .text_color(rgb(roles.on_surface_variant))
                                .child(summary),
                        )
                    })
                    .when_some(failure_status, |this, failure_status| {
                        let status = match failure_status {
                            SessionFailureStatus::Closed => i18n::string("session.status.closed"),
                        };

                        this.child(
                            div()
                                .text_size(miaominal_settings::FontSize::Input.scaled())
                                .text_color(rgb(text_muted))
                                .child(status),
                        )
                    })
                    .child(
                        div().w_full().p_4().child(
                            div()
                                .text_size(miaominal_settings::FontSize::Subheading.scaled())
                                .text_color(rgb(roles.on_surface))
                                .child(error),
                        ),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .when(profile_exists, |this| {
                                this.child(icon_button(
                                    AppIcon::Rotate,
                                    36.0,
                                    12.0,
                                    Some(roles.primary),
                                    Some(roles.on_primary),
                                    None,
                                    {
                                        let weak = weak.clone();
                                        move |_window, cx| {
                                            weak.update(cx, |this, cx| {
                                                let Some(tab_index) = this
                                                    .workspace_state
                                                    .tabs
                                                    .iter()
                                                    .position(|t| t.id == tab_id)
                                                else {
                                                    return;
                                                };
                                                let profile = this
                                                    .data
                                                    .sessions
                                                    .iter()
                                                    .find(|p| p.id == profile_id_retry)
                                                    .cloned();
                                                if let Some(session) = this.workspace_state.tabs
                                                    [tab_index]
                                                    .as_session_mut()
                                                    && let Some(profile) = profile
                                                {
                                                    session.commands = None;
                                                    session.pending_profile = Some(profile);
                                                    session.set_connection_state(
                                                        SessionConnectionState::Connecting,
                                                    );
                                                    session.reconnect_attempt = 0;
                                                    session.terminal.push_text(&format!(
                                                        "{}\r\n",
                                                        i18n::string(
                                                            "session.terminal.reconnecting_marker"
                                                        )
                                                    ));
                                                }
                                                cx.notify();
                                            })
                                            .ok();
                                        }
                                    },
                                ))
                            })
                            .when(!is_port_forward && profile_exists, |this| {
                                this.child(icon_button(
                                    AppIcon::Edit,
                                    36.0,
                                    12.0,
                                    None,
                                    None,
                                    None,
                                    {
                                        let weak = weak.clone();
                                        move |window, cx| {
                                            weak.update(cx, |this, cx| {
                                                if let Some(index) = this
                                                    .data
                                                    .sessions
                                                    .iter()
                                                    .position(|p| p.id == profile_id_edit)
                                                {
                                                    this.open_hosts_tab(cx);
                                                    this.open_host_editor(index, window, cx);
                                                }
                                            })
                                            .ok();
                                        }
                                    },
                                ))
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_session_disconnected_banner(
        &self,
        summary: String,
        profile_id: String,
        purpose: SessionPurpose,
        tab_id: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let weak = cx.entity().downgrade();
        let is_port_forward = purpose == SessionPurpose::PortForwarding;
        let profile_exists = self.data.sessions.iter().any(|p| p.id == profile_id);
        let supporting_text = (!summary.is_empty()).then_some(summary);

        let hide_action = {
            let weak = weak.clone();
            basic_dialog_action_button(
                SharedString::from(format!("session-hide-{tab_id}")),
                i18n::string("session.workspace.hide_action"),
                BasicDialogActionTone::Default,
            )
            .on_click(move |_, _, cx| {
                weak.update(cx, |this, cx| {
                    this.hide_preserved_history_popup(tab_id, cx);
                })
                .ok();
            })
        };

        let reconnect_action = {
            let mut button = basic_dialog_action_button(
                SharedString::from(format!("session-reconnect-{tab_id}")),
                i18n::string("session.workspace.reconnect_action"),
                BasicDialogActionTone::Default,
            );
            button = button.disabled(is_port_forward || !profile_exists);
            if is_port_forward || !profile_exists {
                button = button.opacity(0.48);
            }

            let weak = weak.clone();
            button.on_click(move |_, _, cx| {
                weak.update(cx, |this, cx| {
                    this.reconnect_session_tab(tab_id, &profile_id, false, cx);
                })
                .ok();
            })
        };

        let body = v_flex()
            .w_full()
            .min_w(px(0.0))
            .gap_3()
            .child(
                div()
                    .w_full()
                    .text_center()
                    .text_size(miaominal_settings::FontSize::Heading.scaled())
                    .line_height(miaominal_settings::scaled_line_height(20.0))
                    .text_color(rgb(roles.on_surface_variant))
                    .child(i18n::string("session.terminal_messages.read_only_history")),
            )
            .into_any_element();

        let actions = h_flex()
            .w_full()
            .gap_2()
            .justify_end()
            .child(hide_action)
            .child(reconnect_action)
            .into_any_element();

        render_basic_dialog_with_config(
            format!("session-disconnected-{tab_id}"),
            crate::ui::shell::support::BasicDialogConfig {
                title: i18n::string("session.workspace.session_closed"),
                supporting_text,
                body: Some(body),
                actions,
                icon: Some(BasicDialogIcon {
                    icon: AppIcon::Minimize,
                    tint: roles.on_surface_variant,
                }),
                header_alignment: BasicDialogHeaderAlignment::Center,
                exit_progress: None,
            },
        )
    }

    fn render_session_failure_banner(
        &self,
        failure: SessionFailureView,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let SessionFailureView {
            title,
            summary,
            error,
            failure_status,
            profile_id,
            purpose,
            tab_id,
        } = failure;

        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let weak = cx.entity().downgrade();
        let profile_id_retry = profile_id.clone();
        let is_port_forward = purpose == SessionPurpose::PortForwarding;
        let profile_exists = self.data.sessions.iter().any(|p| p.id == profile_id);
        let supporting_text = (!summary.is_empty()).then_some(summary);

        let hide_action = {
            let weak = weak.clone();
            basic_dialog_action_button(
                SharedString::from(format!("session-hide-{tab_id}")),
                i18n::string("session.workspace.hide_action"),
                BasicDialogActionTone::Default,
            )
            .on_click(move |_, _, cx| {
                weak.update(cx, |this, cx| {
                    this.hide_preserved_history_popup(tab_id, cx);
                })
                .ok();
            })
        };

        let reconnect_action = {
            let mut button = basic_dialog_action_button(
                SharedString::from(format!("session-reconnect-{tab_id}")),
                i18n::string("session.workspace.reconnect_action"),
                BasicDialogActionTone::Default,
            );
            button = button.disabled(is_port_forward || !profile_exists);
            if is_port_forward || !profile_exists {
                button = button.opacity(0.48);
            }

            let weak = weak.clone();
            button.on_click(move |_, _, cx| {
                weak.update(cx, |this, cx| {
                    this.reconnect_session_tab(tab_id, &profile_id_retry, true, cx);
                })
                .ok();
            })
        };

        let body = v_flex()
            .w_full()
            .min_w(px(0.0))
            .gap_3()
            .when_some(failure_status, |this, failure_status| {
                let status = match failure_status {
                    SessionFailureStatus::Closed => i18n::string("session.status.closed"),
                };

                this.child(
                    div()
                        .w_full()
                        .text_center()
                        .text_size(miaominal_settings::FontSize::Input.scaled())
                        .text_color(rgb(roles.on_surface_variant))
                        .child(status),
                )
            })
            .child(
                div()
                    .w_full()
                    .px_3()
                    .py_2()
                    .rounded(px(12.0))
                    .bg(color_with_alpha(roles.error, 0x10))
                    .child(
                        div()
                            .w_full()
                            .text_size(miaominal_settings::FontSize::Subheading.scaled())
                            .line_height(miaominal_settings::scaled_line_height(18.0))
                            .text_color(rgb(roles.on_surface))
                            .child(error),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .text_center()
                    .text_size(miaominal_settings::FontSize::Heading.scaled())
                    .line_height(miaominal_settings::scaled_line_height(20.0))
                    .text_color(rgb(roles.on_surface_variant))
                    .child(i18n::string("session.terminal_messages.read_only_history")),
            )
            .into_any_element();

        let actions = h_flex()
            .w_full()
            .gap_2()
            .justify_end()
            .child(hide_action)
            .child(reconnect_action)
            .into_any_element();

        render_basic_dialog_with_config(
            format!("session-failure-{tab_id}"),
            crate::ui::shell::support::BasicDialogConfig {
                title,
                supporting_text,
                body: Some(body),
                actions,
                icon: Some(BasicDialogIcon {
                    icon: AppIcon::Close,
                    tint: roles.error,
                }),
                header_alignment: BasicDialogHeaderAlignment::Center,
                exit_progress: None,
            },
        )
    }

    fn render_session_reconnect_fab(
        &self,
        profile_id: String,
        write_marker: bool,
        tab_id: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let weak = cx.entity().downgrade();

        div()
            .absolute()
            .right(px(20.0))
            .bottom(px(20.0))
            .child(fab_icon_button(AppIcon::Rotate, move |_window, cx| {
                weak.update(cx, |this, cx| {
                    this.reconnect_session_tab(tab_id, &profile_id, write_marker, cx);
                })
                .ok();
            }))
            .into_any_element()
    }

    pub(in crate::ui::shell) fn render_workspace_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        self.advance_pane_split_animation(window, cx);

        // Snapshot the layout so we can borrow self mutably while walking it.
        let layout = std::mem::replace(
            &mut self.workspace_state.workspace.pane_layout,
            PaneLayout::Leaf(self.workspace_state.workspace.active_pane_id),
        );
        let valid_source_ids = self.pane_drop_source_ids();
        let workspace_body = self.render_pane_layout(&layout, &[], &valid_source_ids, window, cx);
        self.workspace_state.workspace.pane_layout = layout;

        let session_index = self.active_terminal_session_index();

        let show_side_panel = workspace_side_panel_render_state(
            self.panels.session_side_panel_open && session_index.is_some(),
            &mut self.panels.visible_session_side_panel,
            &mut self.panels.session_side_panel_transition,
            window,
        );
        let show_snippets_panel = workspace_side_panel_render_state(
            self.panels.session_snippets_panel_open && session_index.is_some(),
            &mut self.panels.visible_session_snippets_panel,
            &mut self.panels.session_snippets_panel_transition,
            window,
        );
        let entity = cx.entity();
        let side_panel = show_side_panel.and_then(|visibility| {
            session_index
                .and_then(|index| self.workspace_state.tabs.get(index))
                .and_then(TabState::as_session)
                .map(|session| {
                    render_workspace_side_panel(
                        self.render_session_workspace_side_panel(entity.clone(), session, cx),
                        SESSION_MONITOR_PANEL_WIDTH,
                        visibility,
                        WorkspaceSidePanelDock::Left,
                    )
                })
        });
        let snippets_panel = show_snippets_panel.map(|visibility| {
            render_workspace_side_panel(
                self.render_session_snippets_sidebar(),
                SESSION_MONITOR_PANEL_WIDTH,
                visibility,
                WorkspaceSidePanelDock::Right,
            )
        });

        h_flex()
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .when_some(side_panel, |this, panel| this.child(panel))
            .child(
                div()
                    .id("terminal-workspace-center")
                    .flex_1()
                    .size_full()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .child(workspace_body),
            )
            .when_some(snippets_panel, |this, panel| this.child(panel))
            .into_any_element()
    }

    fn render_pane_layout(
        &mut self,
        layout: &PaneLayout,
        path: &[usize],
        valid_source_ids: &[usize],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match layout {
            PaneLayout::Leaf(pane_id) => {
                let embedded_in_split = !path.is_empty();
                let body = if *pane_id == self.workspace_state.workspace.active_pane_id {
                    self.render_active_pane_surface(embedded_in_split, window, cx)
                } else {
                    self.render_parked_pane_surface(*pane_id, embedded_in_split, cx)
                };
                let content = if path.is_empty() {
                    body
                } else {
                    let is_active = *pane_id == self.workspace_state.workspace.active_pane_id;
                    let roles = miaominal_settings::current_theme().material.roles;
                    let header = self.render_pane_header(*pane_id);
                    div()
                        .rounded(px(16.0))
                        .border(px(TERMINAL_PANEL_BORDER))
                        .border_color(if is_active {
                            rgb(roles.primary)
                        } else {
                            color_with_alpha(roles.primary, 0x00)
                        })
                        .overflow_hidden()
                        .bg(rgb(roles.background))
                        .flex()
                        .flex_col()
                        .size_full()
                        .min_w(px(0.0))
                        .min_h(px(0.0))
                        .child(header)
                        .child(
                            div()
                                .flex_grow()
                                .flex_shrink()
                                .flex_basis(gpui::relative(1.0))
                                .min_w(px(0.0))
                                .min_h(px(0.0))
                                .child(body),
                        )
                        .into_any_element()
                };
                let content = self.render_split_animated_pane(path, content);

                self.render_pane_drop_surface(*pane_id, content, valid_source_ids, cx)
            }
            PaneLayout::Split {
                axis,
                children,
                flexes,
            } => {
                let mut container = match axis {
                    super::super::workspace::SplitAxis::Horizontal => div().flex().flex_row(),
                    super::super::workspace::SplitAxis::Vertical => div().flex().flex_col(),
                };
                container = container.size_full().min_w(px(0.0)).min_h(px(0.0));
                for (i, child) in children.iter().enumerate() {
                    let mut child_path = path.to_vec();
                    child_path.push(i);
                    let flex = flexes.get(i).copied().unwrap_or(1.0).max(0.001);
                    let child_element =
                        self.render_pane_layout(child, &child_path, valid_source_ids, window, cx);
                    let wrapped = div()
                        .flex_grow()
                        .flex_shrink()
                        .flex_basis(gpui::relative(flex))
                        .h_full()
                        .min_w(px(0.0))
                        .min_h(px(0.0))
                        .child(child_element);
                    container = container.child(wrapped);
                    if i + 1 < children.len() {
                        let mut bar_path = path.to_vec();
                        bar_path.push(i);
                        container = container.child(self.render_split_bar(*axis, bar_path, cx));
                    }
                }
                container.into_any_element()
            }
        }
    }

    fn render_split_animated_pane(
        &self,
        path: &[usize],
        content: gpui::AnyElement,
    ) -> gpui::AnyElement {
        let Some(animation) = self.workspace_state.workspace.pane_split_animation.as_ref() else {
            return content;
        };
        let Some((&child_index, parent_path)) = path.split_last() else {
            return content;
        };
        if parent_path != animation.path.as_slice() || child_index != animation.new_child_index {
            return content;
        }

        let duration_seconds = animation.duration.as_secs_f32();
        if duration_seconds <= f32::EPSILON {
            return content;
        }

        let progress =
            (animation.started_at.elapsed().as_secs_f32() / duration_seconds).clamp(0.0, 1.0);
        let eased = progress * progress * (3.0 - 2.0 * progress);
        let offset = match animation.kind {
            super::super::panes::PaneSplitAnimationKind::Opening => (1.0 - eased) * 14.0,
            super::super::panes::PaneSplitAnimationKind::Closing => eased * 14.0,
        };
        let animated_is_primary = animation.new_child_index == animation.child_index;
        let (left_offset, top_offset) = match animation.axis {
            super::super::workspace::SplitAxis::Horizontal => {
                let signed = if animated_is_primary { -offset } else { offset };
                (signed, 0.0)
            }
            super::super::workspace::SplitAxis::Vertical => {
                let signed = if animated_is_primary { -offset } else { offset };
                (0.0, signed)
            }
        };
        let opacity = match animation.kind {
            super::super::panes::PaneSplitAnimationKind::Opening => 0.42 + eased * 0.58,
            super::super::panes::PaneSplitAnimationKind::Closing => 1.0 - eased * 0.62,
        };

        div()
            .relative()
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .opacity(opacity)
            .left(px(left_offset))
            .top(px(top_offset))
            .child(content)
            .into_any_element()
    }

    fn render_pane_drop_surface(
        &self,
        pane_id: super::super::panes::PaneId,
        content: gpui::AnyElement,
        valid_source_ids: &[usize],
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if valid_source_ids.is_empty() {
            return content;
        }

        div()
            .relative()
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .child(content)
            .child(self.render_pane_drop_targets(pane_id, valid_source_ids, cx))
            .into_any_element()
    }

    fn render_pane_drop_targets(
        &self,
        pane_id: super::super::panes::PaneId,
        valid_source_ids: &[usize],
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        use super::super::panes::PaneTabDropZone;

        let entity = cx.entity();
        let center_enabled = self
            .pane_tab_index(pane_id)
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(TabState::as_session)
            .is_some();
        let edge_inset = gpui::relative(0.24);

        let render_zone = |base: gpui::Div,
                           zone: PaneTabDropZone,
                           accepts: bool,
                           ids: Vec<usize>,
                           entity: Entity<AppView>| {
            let ids_for_drag = ids.clone();
            let ids_for_drop = ids;
            let entity_for_drop = entity.clone();

            base.drag_over::<DraggedTab>(move |style, payload: &DraggedTab, _, _| {
                if !accepts || !ids_for_drag.contains(&payload.source_tab_id) {
                    return style;
                }
                pane_drop_zone_style(style, zone)
            })
            .on_drop::<DraggedTab>(move |payload: &DraggedTab, window, cx| {
                if !accepts || !ids_for_drop.contains(&payload.source_tab_id) {
                    return;
                }
                let source_tab_id = payload.source_tab_id;
                entity_for_drop.update(cx, |this, cx| {
                    this.handle_pane_tab_drop(source_tab_id, pane_id, zone, window, cx);
                });
            })
            .into_any_element()
        };

        div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .child(render_zone(
                div().absolute().top_0().left_0().right_0().h(edge_inset),
                PaneTabDropZone::Up,
                true,
                valid_source_ids.to_vec(),
                entity.clone(),
            ))
            .child(render_zone(
                div().absolute().bottom_0().left_0().right_0().h(edge_inset),
                PaneTabDropZone::Down,
                true,
                valid_source_ids.to_vec(),
                entity.clone(),
            ))
            .child(render_zone(
                div()
                    .absolute()
                    .top(edge_inset)
                    .bottom(edge_inset)
                    .left_0()
                    .w(edge_inset),
                PaneTabDropZone::Left,
                true,
                valid_source_ids.to_vec(),
                entity.clone(),
            ))
            .child(render_zone(
                div()
                    .absolute()
                    .top(edge_inset)
                    .bottom(edge_inset)
                    .right_0()
                    .w(edge_inset),
                PaneTabDropZone::Right,
                true,
                valid_source_ids.to_vec(),
                entity.clone(),
            ))
            .child(render_zone(
                div()
                    .absolute()
                    .top(edge_inset)
                    .right(edge_inset)
                    .bottom(edge_inset)
                    .left(edge_inset),
                PaneTabDropZone::Center,
                center_enabled,
                valid_source_ids.to_vec(),
                entity,
            ))
            .into_any_element()
    }

    fn render_pane_header(&self, pane_id: super::super::panes::PaneId) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let is_active = pane_id == self.workspace_state.workspace.active_pane_id;
        let tab_index = if is_active {
            self.workspace_state.workspace.active_tab
        } else {
            self.parked_pane(pane_id).and_then(|p| p.active_tab)
        };
        let (label, status_color) = tab_index
            .and_then(|idx| self.workspace_state.tabs.get(idx))
            .map(|tab| {
                let label = tab
                    .as_session()
                    .and_then(|s| {
                        self.data
                            .sessions
                            .iter()
                            .find(|p| p.id == s.profile_id)
                            .map(|p| p.name.clone())
                    })
                    .unwrap_or_else(|| tab.title.to_string());
                let has_activity = tab.as_session().is_some_and(|session| session.has_activity);
                (label, tab_status_indicator_color(tab, has_activity))
            })
            .unwrap_or_else(|| (i18n::string("session.workspace.empty_tab_label"), None));

        div()
            .flex_shrink_0()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_4()
            .when_some(status_color, |this, color| {
                this.child(status_indicator(color, 7.0))
            })
            .child(
                div()
                    .text_size(miaominal_settings::FontSize::Heading.scaled())
                    .text_color(rgb(if is_active {
                        roles.on_surface
                    } else {
                        text_muted
                    }))
                    .when(is_active, |this| this.font_weight(gpui::FontWeight::MEDIUM))
                    .child(SharedString::from(label)),
            )
            .into_any_element()
    }

    fn render_split_bar(
        &self,
        axis: super::super::workspace::SplitAxis,
        path: Vec<usize>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        use super::super::panes::{PaneSplitDragMarker, PaneSplitDragState};
        use super::super::workspace::SplitAxis;
        let Some((&child_index, parent_path)) = path.split_last() else {
            return div().into_any_element();
        };
        let parent_path: Vec<usize> = parent_path.to_vec();
        let parent_path_for_drag = parent_path.clone();
        let parent_path_for_move = parent_path.clone();
        let marker = PaneSplitDragMarker {
            path: parent_path_for_drag.clone(),
            child_index,
            axis,
        };

        let bar_id = SharedString::from(format!(
            "split-bar-{}-{}",
            parent_path
                .iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join("-"),
            child_index
        ));

        const PANE_GAP: f32 = 8.0;
        let is_dragging = self
            .workspace_state
            .workspace
            .pane_split_drag
            .as_ref()
            .is_some_and(|drag| {
                drag.axis == axis && drag.child_index == child_index && drag.path == parent_path
            });

        let mut bar = div().id(bar_id).flex_shrink_0().occlude();
        bar = match axis {
            SplitAxis::Horizontal => bar.w(px(PANE_GAP)).h_full().cursor_col_resize(),
            SplitAxis::Vertical => bar.h(px(PANE_GAP)).w_full().cursor_row_resize(),
        };

        let path_for_start = parent_path.clone();
        bar.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                let path = path_for_start.clone();
                let (flex_a, flex_b) = {
                    let mut node = &this.workspace_state.workspace.pane_layout;
                    for &i in &path {
                        match node {
                            super::super::workspace::PaneLayout::Split { children, .. } => {
                                let Some(next) = children.get(i) else {
                                    return;
                                };
                                node = next;
                            }
                            _ => return,
                        }
                    }
                    if let super::super::workspace::PaneLayout::Split { flexes, .. } = node
                        && let (Some(a), Some(b)) =
                            (flexes.get(child_index), flexes.get(child_index + 1))
                    {
                        (*a, *b)
                    } else {
                        return;
                    }
                };
                let initial_pointer = match axis {
                    SplitAxis::Horizontal => f32::from(event.position.x),
                    SplitAxis::Vertical => f32::from(event.position.y),
                };
                let container_size = this.split_container_size(&path, axis, window);
                this.workspace_state.workspace.pane_split_drag = Some(PaneSplitDragState {
                    path,
                    child_index,
                    axis,
                    initial_pointer,
                    initial_flex_a: flex_a,
                    initial_flex_b: flex_b,
                    container_size,
                });
                cx.notify();
            }),
        )
        .hover(move |this| {
            if is_dragging {
                this
            } else {
                match axis {
                    SplitAxis::Horizontal => this.cursor_col_resize(),
                    SplitAxis::Vertical => this.cursor_row_resize(),
                }
            }
        })
        .on_drag(marker, |m, _offset, _window, cx| cx.new(|_| m.clone()))
        .on_drag_move::<PaneSplitDragMarker>(cx.listener(
            move |this, event: &gpui::DragMoveEvent<PaneSplitDragMarker>, _window, cx| {
                let _ = &parent_path_for_move; // capture for closure type
                let Some(drag) = this.workspace_state.workspace.pane_split_drag.clone() else {
                    return;
                };
                let pointer = match drag.axis {
                    SplitAxis::Horizontal => f32::from(event.event.position.x),
                    SplitAxis::Vertical => f32::from(event.event.position.y),
                };
                let delta_px = pointer - drag.initial_pointer;
                let delta_flex = if drag.container_size > 0.0 {
                    delta_px / drag.container_size
                } else {
                    0.0
                };
                let new_a = (drag.initial_flex_a + delta_flex).clamp(0.05, 0.95);
                let new_b = (drag.initial_flex_b - delta_flex).clamp(0.05, 0.95);
                this.apply_split_flex_delta(&drag.path, drag.child_index, new_a, new_b);
                cx.notify();
            },
        ))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, _event: &MouseUpEvent, _, cx| {
                if this
                    .workspace_state
                    .workspace
                    .pane_split_drag
                    .take()
                    .is_some()
                {
                    cx.notify();
                }
            }),
        )
        .into_any_element()
    }

    fn render_parked_pane_surface(
        &mut self,
        pane_id: super::super::panes::PaneId,
        embedded_in_split: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let parked_tab_index = self
            .parked_pane(pane_id)
            .and_then(|parked| parked.active_tab);
        let cell_width = self
            .parked_pane(pane_id)
            .map(|p| p.terminal_cell_width)
            .unwrap_or(
                self.workspace_state
                    .workspace
                    .active_pane
                    .terminal_cell_width,
            );
        let line_height = self
            .parked_pane(pane_id)
            .map(|p| p.terminal_line_height)
            .unwrap_or(
                self.workspace_state
                    .workspace
                    .active_pane
                    .terminal_line_height,
            );

        let placeholder = parked_tab_index.and_then(|active| {
            self.workspace_state
                .tabs
                .get(active)
                .and_then(|tab| self.render_session_placeholder(tab, !embedded_in_split, cx))
        });
        let history_banner = parked_tab_index.and_then(|active| {
            self.workspace_state
                .tabs
                .get(active)
                .and_then(|tab| self.render_session_history_banner(tab, cx))
        });

        let has_terminal = parked_tab_index
            .and_then(|active| self.workspace_state.tabs.get(active))
            .and_then(TabState::as_session)
            .is_some();

        let weak = cx.entity().downgrade();
        let show_scrollbar = self.terminal_scrollbar_visible(pane_id);
        let terminal_settings = self.settings_store.settings().clone();

        let pane_surface = div()
            .id(terminal_pane_surface_id(pane_id))
            .relative()
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .rounded(px(16.0))
            .overflow_hidden()
            .bg(rgb(roles.background))
            .text_color(rgb(roles.on_surface))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _event: &MouseDownEvent, window, cx| {
                    this.set_active_pane(pane_id, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _event: &MouseDownEvent, window, cx| {
                    this.set_active_pane(pane_id, window, cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseUpEvent, window, cx| {
                    let settings = miaominal_settings::current_settings();
                    let force_context_menu =
                        settings.terminal_shift_right_click_context_menu && event.modifiers.shift;
                    if force_context_menu
                        || settings.terminal_right_click_behavior
                            != TerminalRightClickBehavior::CopySelectionOrPaste
                    {
                        return;
                    }

                    this.set_active_pane(pane_id, window, cx);
                    this.handle_terminal_mouse_up(event, cx);
                }),
            )
            .child(if has_terminal {
                div()
                    .size_full()
                    .px_4()
                    .pb_4()
                    .when(!embedded_in_split, |this| this.pt_4())
                    .child(render_terminal_canvas_for_pane(
                        None,
                        cell_width,
                        line_height,
                        weak,
                        pane_id,
                        show_scrollbar,
                    ))
                    .into_any_element()
            } else {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(miaominal_settings::FontSize::Body.scaled())
                    .text_color(rgb(text_muted))
                    .child(i18n::string("session.workspace.click_to_focus"))
                    .into_any_element()
            })
            .when_some(history_banner, |this, banner| this.child(banner))
            .when_some(placeholder, |this, placeholder| {
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .right_0()
                        .bottom_0()
                        .left_0()
                        .child(placeholder),
                )
            });

        if terminal_settings
            .terminal_right_click_behavior
            .uses_context_menu()
            || terminal_settings.terminal_shift_right_click_context_menu
        {
            let entity = cx.entity();
            pane_surface
                .context_menu({
                    let terminal_settings = terminal_settings.clone();
                    move |menu, window, cx| {
                        if !(terminal_settings
                            .terminal_right_click_behavior
                            .uses_context_menu()
                            || terminal_settings.terminal_shift_right_click_context_menu
                                && window.modifiers().shift)
                        {
                            return menu;
                        }

                        let view = entity.read(cx);
                        let has_selection = view
                            .workspace_state
                            .workspace
                            .parked_panes
                            .get(&pane_id)
                            .and_then(|parked| parked.active_tab)
                            .and_then(|index| view.workspace_state.tabs.get(index))
                            .and_then(TabState::as_session)
                            .is_some_and(|session| session.terminal.has_selection());

                        build_terminal_context_menu(
                            menu,
                            entity.clone(),
                            has_selection,
                            Some(pane_id),
                            window,
                            cx,
                        )
                    }
                })
                .into_any_element()
        } else {
            pane_surface.into_any_element()
        }
    }

    fn render_active_pane_surface(
        &mut self,
        embedded_in_split: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let active_index = self.workspace_state.workspace.active_tab;
        let terminal_focused = self
            .workspace_state
            .workspace
            .active_pane
            .terminal_focus
            .is_focused(window);
        let cell_width = terminal_cell_width(window);
        let line_height = terminal_line_height(window);
        let active_pane_id = self.workspace_state.workspace.active_pane_id;
        let show_scrollbar = self.terminal_scrollbar_visible(active_pane_id);

        let pane_surface = div()
            .id(terminal_pane_surface_id(active_pane_id))
            .size_full()
            .relative()
            .rounded(px(16.0))
            .border_color(rgb(if terminal_focused && active_index.is_some() {
                roles.primary
            } else {
                roles.outline_variant
            }))
            .bg(rgb(roles.surface_container_highest))
            .overflow_hidden()
            .when_some(active_index, |this, index| {
                let tab = &self.workspace_state.tabs[index];
                if tab.as_session().is_none() {
                    return this;
                }
                let placeholder = self.render_session_placeholder(tab, !embedded_in_split, cx);
                let history_banner = self.render_session_history_banner(tab, cx);
                let tab_id = tab.id;
                let hovered_link = self
                    .workspace_state
                    .workspace
                    .active_pane
                    .terminal_hovered_link
                    .clone()
                    .filter(|hovered| hovered.tab_id == tab_id);
                let show_link_cursor = hovered_link.is_some()
                    && self
                        .workspace_state
                        .workspace
                        .active_pane
                        .terminal_link_open_modifier;
                let weak = cx.entity().downgrade();

                let terminal_surface = div()
                    .id(SharedString::from(format!("terminal-output-{}", tab_id)))
                    .track_focus(&self.workspace_state.workspace.active_pane.terminal_focus)
                    .size_full()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .rounded(px(16.0))
                    .overflow_hidden()
                    .px_4()
                    .pb_4()
                    .when(!embedded_in_split, |this| this.pt_4())
                    .bg(rgb(roles.background))
                    .text_color(rgb(roles.on_surface))
                    .when(show_link_cursor, |this| this.cursor_pointer())
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        this.handle_terminal_hover(*hovered, cx);
                    }))
                    .on_modifiers_changed(cx.listener(
                        move |this, event: &ModifiersChangedEvent, _, cx| {
                            this.handle_terminal_modifiers_changed(event, cx);
                        },
                    ))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            window.focus(
                                &this.workspace_state.workspace.active_pane.terminal_focus,
                                cx,
                            );
                            this.handle_terminal_mouse_down(event, cx);
                        }),
                    )
                    .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                        this.handle_terminal_mouse_move(event, cx);
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseUpEvent, _, cx| {
                            this.handle_terminal_mouse_up(event, cx);
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.handle_terminal_mouse_down(event, cx);
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Middle,
                        cx.listener(move |this, event: &MouseUpEvent, _, cx| {
                            this.handle_terminal_mouse_up(event, cx);
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            window.focus(
                                &this.workspace_state.workspace.active_pane.terminal_focus,
                                cx,
                            );
                            this.handle_terminal_mouse_down(event, cx);
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseUpEvent, _, cx| {
                            this.handle_terminal_mouse_up(event, cx);
                        }),
                    )
                    .on_key_down(cx.listener(Self::handle_terminal_key_down))
                    .on_key_up(cx.listener(Self::handle_terminal_key_up))
                    .on_scroll_wheel(cx.listener(move |this, event: &ScrollWheelEvent, _, cx| {
                        this.handle_terminal_scroll_wheel(event, line_height, cx);
                    }))
                    .child(render_terminal_canvas_for_pane(
                        hovered_link,
                        cell_width,
                        line_height,
                        weak,
                        active_pane_id,
                        show_scrollbar,
                    ))
                    .when_some(
                        self.advance_terminal_search_overlay(window),
                        |this, visibility| {
                            this.child(self.render_terminal_search_overlay(visibility, cx))
                        },
                    );

                let terminal_stack = div()
                    .flex_1()
                    .min_h(px(0.0))
                    .relative()
                    .child(terminal_surface)
                    .when_some(placeholder, |this, placeholder| {
                        this.child(
                            div()
                                .absolute()
                                .top_0()
                                .right_0()
                                .bottom_0()
                                .left_0()
                                .child(placeholder),
                        )
                    })
                    .when_some(history_banner, |this, banner| this.child(banner));

                this.child(
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .min_h(px(0.0))
                        .child(terminal_stack),
                )
            })
            .when(active_index.is_none(), |this| {
                this.child(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            v_flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(
                                            miaominal_settings::FontSize::SectionTitle.scaled(),
                                        )
                                        .text_color(rgb(roles.on_surface))
                                        .child(i18n::string("workspace.empty.open_ssh_tab_title")),
                                )
                                .child(
                                    div()
                                        .text_size(miaominal_settings::FontSize::Input.scaled())
                                        .text_color(rgb(text_muted))
                                        .child(i18n::string("workspace.empty.open_ssh_tab_body")),
                                ),
                        ),
                )
            });

        let terminal_settings = self.settings_store.settings().clone();
        if active_index.is_some()
            && (terminal_settings
                .terminal_right_click_behavior
                .uses_context_menu()
                || terminal_settings.terminal_shift_right_click_context_menu)
        {
            let entity = cx.entity();
            pane_surface
                .context_menu({
                    let terminal_settings = terminal_settings.clone();
                    move |menu, window, cx| {
                        if !(terminal_settings
                            .terminal_right_click_behavior
                            .uses_context_menu()
                            || terminal_settings.terminal_shift_right_click_context_menu
                                && window.modifiers().shift)
                        {
                            return menu;
                        }

                        let view = entity.read(cx);
                        let has_selection = view
                            .workspace_state
                            .workspace
                            .active_tab
                            .and_then(|index| view.workspace_state.tabs.get(index))
                            .and_then(TabState::as_session)
                            .is_some_and(|session| session.terminal.has_selection());

                        build_terminal_context_menu(
                            menu,
                            entity.clone(),
                            has_selection,
                            None,
                            window,
                            cx,
                        )
                    }
                })
                .into_any_element()
        } else {
            pane_surface.into_any_element()
        }
    }

    fn render_session_workspace_side_panel(
        &self,
        entity: Entity<Self>,
        session: &SessionTabState,
        cx: &App,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let selected_index = match self.panels.session_side_panel_view {
            SessionSidePanelView::Monitor => 0,
            SessionSidePanelView::Snippets => 1,
        };

        let tabs = TabBar::new("session-side-panel-tabs")
            .segmented()
            .selected_index(selected_index)
            .on_click({
                let entity = entity.clone();
                move |index, _, cx| {
                    entity.update(cx, |this, cx| {
                        match *index {
                            0 => {
                                this.panels.session_side_panel_open = true;
                                this.panels.session_side_panel_view = SessionSidePanelView::Monitor;
                            }
                            _ => {
                                this.panels.session_side_panel_open = true;
                                this.panels.session_side_panel_view =
                                    SessionSidePanelView::Snippets;
                            }
                        }
                        cx.notify();
                    });
                }
            })
            .child(Tab::new().label(i18n::string("workspace.panel.monitor.title")))
            .child(Tab::new().label(i18n::string("snippets.page.snippets")))
            .h(px(36.0));

        let content = if selected_index == 0 {
            self.render_session_monitor_panel(entity, session)
        } else {
            self.render_session_snippets_panel(entity, cx)
        };

        card_surface(roles.surface_container, 16.0)
            .id("session-workspace-side-panel")
            .w(px(SESSION_MONITOR_PANEL_WIDTH))
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
                            .h(px(56.0))
                            .flex_shrink_0()
                            .items_center()
                            .justify_center()
                            .px_3()
                            .child(
                                div()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(tabs),
                            ),
                    )
                    .child(div().flex_1().min_h(px(0.0)).child(content)),
            )
            .into_any_element()
    }

    fn render_session_monitor_panel(
        &self,
        entity: Entity<Self>,
        session: &SessionTabState,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let monitoring =
            self.shared_monitoring_state_for_profile(&session.profile_id, &session.monitoring);
        let monitor_scroll_handle = self.workspace_state.session_monitor_scroll_handle.clone();

        if !monitoring.auto_collect_enabled {
            return v_flex()
                .id("session-monitor-panel-content")
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .p_3()
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Input.scaled())
                        .text_center()
                        .text_color(rgb(roles.on_surface))
                        .child(i18n::string("workspace.panel.monitor.disabled_title")),
                )
                .child(
                    div()
                        .max_w(px(280.0))
                        .text_center()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .text_color(rgb(text_muted))
                        .child(i18n::string("workspace.panel.monitor.disabled_body")),
                )
                .child(editor_button(
                    i18n::string("workspace.panel.monitor.start_now"),
                    false,
                    true,
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.enable_active_session_monitoring(cx);
                        });
                    },
                ))
                .into_any_element();
        }

        if let Some(error) = monitoring.last_error.as_ref() {
            return v_flex()
                .id("session-monitor-panel-content")
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .p_3()
                .child(
                    div()
                        .text_center()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .text_color(rgb(text_muted))
                        .child(error.clone()),
                )
                .into_any_element();
        }

        let content =
            if let Some(snapshot) = monitoring.last_snapshot.as_ref() {
                let limit = self
                    .settings_store
                    .settings()
                    .monitor_history_duration
                    .history_limit();
                let cpu_history = tail_history(&monitoring.cpu_history, limit);
                let memory_history = tail_history(&monitoring.memory_history, limit);
                let swap_history = tail_history(&monitoring.swap_history, limit);
                let disk_history = tail_history(&monitoring.disk_history, limit);
                let network_history = tail_history(&monitoring.network_history, limit);
                let load_history = tail_history(&monitoring.load_history, limit);
                let cpu_peak = cpu_history
                    .iter()
                    .map(|point| point.value)
                    .fold(snapshot.cpu_percent, f64::max);
                let memory_peak = memory_history
                    .iter()
                    .map(|point| point.value)
                    .fold(snapshot.memory_percent, f64::max);
                let swap_peak = swap_history
                    .iter()
                    .map(|point| point.value)
                    .fold(snapshot.swap_percent, f64::max);
                let disk_peak = disk_history
                    .iter()
                    .map(|point| point.value)
                    .fold(snapshot.disk_percent, f64::max);
                let network_peak = network_history.iter().map(|point| point.value).fold(
                    snapshot.network_rx_kbps + snapshot.network_tx_kbps,
                    f64::max,
                );
                let cpu_y_max = nice_chart_max(cpu_peak, SESSION_MONITOR_PERCENT_MIN_Y_MAX);
                let memory_y_max = nice_chart_max(memory_peak, SESSION_MONITOR_PERCENT_MIN_Y_MAX);
                let swap_y_max = nice_chart_max(swap_peak, SESSION_MONITOR_PERCENT_MIN_Y_MAX);
                let disk_y_max = nice_chart_max(disk_peak, SESSION_MONITOR_PERCENT_MIN_Y_MAX);
                let network_y_max = nice_chart_max(network_peak, 8.0);
                let load_peak = load_history
                    .iter()
                    .map(|point| point.value)
                    .fold(snapshot.load, f64::max);
                let load_y_max = nice_chart_max(load_peak, 1.0);
                let cpu_y_ticks = build_chart_ticks(cpu_y_max);
                let memory_y_ticks = build_chart_ticks(memory_y_max);
                let swap_y_ticks = build_chart_ticks(swap_y_max);
                let disk_y_ticks = build_chart_ticks(disk_y_max);
                let network_y_ticks = build_chart_ticks(network_y_max);
                let load_y_ticks = build_chart_ticks(load_y_max);

                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .px_1()
                            .child(
                                div()
                                    .text_size(miaominal_settings::FontSize::Body.scaled())
                                    .text_color(rgb(roles.on_surface))
                                    .child(i18n::string("workspace.panel.monitor.title")),
                            )
                            .child(
                                div()
                                    .text_size(miaominal_settings::scaled_font_size(11.0))
                                    .text_color(rgb(text_muted))
                                    .child(format_monitor_history_window(limit)),
                            ),
                    )
                    .child(self.render_monitor_chart_card(MonitorChartCardConfig {
                        title: i18n::string("workspace.panel.monitor.cpu"),
                        value: format_percentage(snapshot.cpu_percent),
                        detail: None,
                        history: &cpu_history,
                        y_max: cpu_y_max,
                        y_ticks: cpu_y_ticks.clone(),
                        y_tick_labels: build_chart_tick_labels(&cpu_y_ticks, format_percentage),
                        palette_index: 0,
                        mode: MonitorChartCardMode::Full,
                    }))
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(div().flex_1().min_w(px(0.0)).child(
                                self.render_monitor_chart_card(MonitorChartCardConfig {
                                    title: i18n::string("workspace.panel.monitor.memory"),
                                    value: format_percentage(snapshot.memory_percent),
                                    detail: None,
                                    history: &memory_history,
                                    y_max: memory_y_max,
                                    y_ticks: memory_y_ticks.clone(),
                                    y_tick_labels: build_chart_tick_labels(
                                        &memory_y_ticks,
                                        format_percentage,
                                    ),
                                    palette_index: 1,
                                    mode: MonitorChartCardMode::Compact,
                                }),
                            ))
                            .child(div().flex_1().min_w(px(0.0)).child(
                                self.render_monitor_chart_card(MonitorChartCardConfig {
                                    title: i18n::string("workspace.panel.monitor.swap"),
                                    value: format_percentage(snapshot.swap_percent),
                                    detail: None,
                                    history: &swap_history,
                                    y_max: swap_y_max,
                                    y_ticks: swap_y_ticks.clone(),
                                    y_tick_labels: build_chart_tick_labels(
                                        &swap_y_ticks,
                                        format_percentage,
                                    ),
                                    palette_index: 2,
                                    mode: MonitorChartCardMode::Compact,
                                }),
                            )),
                    )
                    .child(self.render_monitor_chart_card(MonitorChartCardConfig {
                        title: i18n::string("workspace.panel.monitor.network"),
                        value: format_rate_label(
                            snapshot.network_rx_kbps + snapshot.network_tx_kbps,
                        ),
                        detail: Some(i18n::string_args(
                            "workspace.panel.monitor.network_detail",
                            &[
                                ("upload", &format_rate_label(snapshot.network_tx_kbps)),
                                ("download", &format_rate_label(snapshot.network_rx_kbps)),
                            ],
                        )),
                        history: &network_history,
                        y_max: network_y_max,
                        y_ticks: network_y_ticks.clone(),
                        y_tick_labels: build_chart_tick_labels(&network_y_ticks, format_rate_label),
                        palette_index: 3,
                        mode: MonitorChartCardMode::Full,
                    }))
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(div().flex_1().min_w(px(0.0)).child(
                                self.render_monitor_chart_card(MonitorChartCardConfig {
                                    title: i18n::string("workspace.panel.monitor.disk"),
                                    value: format_percentage(snapshot.disk_percent),
                                    detail: None,
                                    history: &disk_history,
                                    y_max: disk_y_max,
                                    y_ticks: disk_y_ticks.clone(),
                                    y_tick_labels: build_chart_tick_labels(
                                        &disk_y_ticks,
                                        format_percentage,
                                    ),
                                    palette_index: 4,
                                    mode: MonitorChartCardMode::Compact,
                                }),
                            ))
                            .child(div().flex_1().min_w(px(0.0)).child(
                                self.render_monitor_chart_card(MonitorChartCardConfig {
                                    title: i18n::string("workspace.panel.monitor.load"),
                                    value: format_load_label(snapshot.load),
                                    detail: None,
                                    history: &load_history,
                                    y_max: load_y_max,
                                    y_ticks: load_y_ticks.clone(),
                                    y_tick_labels: build_chart_tick_labels(
                                        &load_y_ticks,
                                        format_load_label,
                                    ),
                                    palette_index: 5,
                                    mode: MonitorChartCardMode::Compact,
                                }),
                            )),
                    )
                    .into_any_element()
            } else {
                return v_flex()
                    .id("session-monitor-panel-content")
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .p_3()
                    .child(md3_spinner(18.0))
                    .child(
                        div()
                            .text_center()
                            .text_size(miaominal_settings::FontSize::Body.scaled())
                            .text_color(rgb(text_muted))
                            .child(i18n::string("workspace.panel.monitor.loading")),
                    )
                    .into_any_element();
            };

        div()
            .id("session-monitor-panel-content")
            .relative()
            .size_full()
            .min_h_0()
            .child(
                div()
                    .id("session-monitor-scroll")
                    .size_full()
                    .track_scroll(&monitor_scroll_handle)
                    .overflow_y_scroll()
                    .child(v_flex().w_full().min_h_full().p_3().child(content)),
            )
            .vertical_scrollbar(&monitor_scroll_handle)
            .into_any_element()
    }

    fn render_session_snippets_sidebar(&self) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;

        card_surface(roles.surface_container, 16.0)
            .id("session-snippets-split-panel")
            .w(px(SESSION_MONITOR_PANEL_WIDTH))
            .h_full()
            .flex_shrink_0()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .into_any_element()
    }

    fn render_monitor_chart_card(&self, config: MonitorChartCardConfig<'_>) -> gpui::AnyElement {
        let MonitorChartCardConfig {
            title,
            value,
            detail,
            history,
            y_max,
            y_ticks,
            y_tick_labels,
            palette_index,
            mode,
        } = config;

        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let axis_label_font_size = miaominal_settings::scaled_font_size(11.0);
        let axis_label_width =
            estimate_monitor_axis_label_width(&y_tick_labels, axis_label_font_size.as_f32());
        let chart_data = history.to_vec();
        let time_axis_labels = (mode == MonitorChartCardMode::Full && !chart_data.is_empty())
            .then(|| build_monitor_time_axis_labels(chart_data.len()));

        let chart = if chart_data.is_empty() {
            div()
                .h(px(SESSION_MONITOR_CHART_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .text_size(miaominal_settings::FontSize::Body.scaled())
                .text_color(rgb(text_muted))
                .child(i18n::string("workspace.panel.monitor.loading"))
                .into_any_element()
        } else {
            let [top_tick_label, middle_tick_label, bottom_tick_label] = y_tick_labels;

            h_flex()
                .w_full()
                .gap_1()
                .items_start()
                .child(
                    v_flex()
                        .w(px(axis_label_width))
                        .h(px(SESSION_MONITOR_CHART_HEIGHT))
                        .justify_between()
                        .text_size(axis_label_font_size)
                        .text_color(rgb(roles.on_surface_variant))
                        .child(div().w_full().text_right().child(top_tick_label))
                        .child(div().w_full().text_right().child(middle_tick_label))
                        .child(div().w_full().text_right().child(bottom_tick_label)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .h(px(SESSION_MONITOR_CHART_HEIGHT))
                        .child(MonitorAreaChart::new(
                            chart_data,
                            y_max,
                            y_ticks,
                            palette_index,
                        )),
                )
                .into_any_element()
        };

        v_flex()
            .w_full()
            .gap_2()
            .rounded(px(14.0))
            .bg(rgb(roles.surface))
            .p_3()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(miaominal_settings::FontSize::Body.scaled())
                            .text_color(rgb(text_muted))
                            .child(title),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(miaominal_settings::FontSize::Subheading.scaled())
                            .text_color(rgb(roles.on_surface))
                            .child(value),
                    )
                    .when_some(detail, |this, detail| {
                        this.child(
                            div()
                                .flex_shrink_0()
                                .text_size(miaominal_settings::scaled_font_size(11.0))
                                .text_color(rgb(text_muted))
                                .child(detail),
                        )
                    }),
            )
            .child(chart)
            .when_some(time_axis_labels, |this, labels| {
                let [left_label, center_label, right_label] = labels;
                this.child(
                    h_flex()
                        .w_full()
                        .gap_1()
                        .child(div().w(px(axis_label_width)))
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w(px(0.0))
                                .justify_between()
                                .text_size(axis_label_font_size)
                                .text_color(rgb(text_muted))
                                .child(div().min_w(px(0.0)).child(left_label))
                                .child(div().min_w(px(0.0)).text_center().child(center_label))
                                .child(div().min_w(px(0.0)).text_right().child(right_label)),
                        ),
                )
            })
            .into_any_element()
    }

    fn render_session_snippets_panel(&self, entity: Entity<Self>, cx: &App) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let filter_text = self
            .workspace_forms
            .snippets_panel
            .filter_input
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();
        let search_matched_snippets: Vec<_> = self
            .data
            .snippets
            .iter()
            .filter(|snippet| miaominal_core::snippet::matches_filter(snippet, &filter_text))
            .cloned()
            .collect();
        let mut package_summaries: Vec<_> =
            Self::collect_available_snippet_packages(&search_matched_snippets)
                .into_iter()
                .map(|package| {
                    let count = search_matched_snippets
                        .iter()
                        .filter(|snippet| snippet.package.eq_ignore_ascii_case(package.as_str()))
                        .count();
                    (package, count)
                })
                .collect();
        package_summaries.sort_by(|left, right| {
            left.0
                .to_ascii_lowercase()
                .cmp(&right.0.to_ascii_lowercase())
        });
        let selected_package_filter = self
            .workspace_forms
            .snippets_panel
            .selected_package_filter
            .as_deref()
            .filter(|selected| {
                package_summaries
                    .iter()
                    .any(|(package, _)| package.eq_ignore_ascii_case(selected))
            });
        let mut visible_snippets: Vec<_> = search_matched_snippets
            .iter()
            .filter(|snippet| {
                selected_package_filter
                    .is_none_or(|package| snippet.package.eq_ignore_ascii_case(package))
            })
            .cloned()
            .collect();
        visible_snippets.sort_by(|left, right| {
            left.description
                .to_ascii_lowercase()
                .cmp(&right.description.to_ascii_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });

        let content = if self.data.snippets.is_empty() {
            shell_empty_state(
                AppIcon::Notebook,
                i18n::string("workspace.panel.snippets.empty"),
            )
            .into_any_element()
        } else if search_matched_snippets.is_empty() {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .text_color(rgb(text_muted))
                        .child(i18n::string("workspace.panel.snippets.no_search_matches")),
                )
                .into_any_element()
        } else if visible_snippets.is_empty() {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .text_color(rgb(text_muted))
                        .child(i18n::string("snippets.empty.no_package_matches")),
                )
                .into_any_element()
        } else {
            let mut list = v_flex().gap_2();
            for snippet in visible_snippets {
                let send_entity = entity.clone();
                let script = snippet.script.clone();
                let preview_line = snippet
                    .script
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or(snippet.script.as_str())
                    .trim();
                let preview = truncate_with_ellipsis(preview_line, 48);
                let button_id = SharedString::from(format!("session-snippet-send-{}", snippet.id));

                list = list.child(
                    div()
                        .w_full()
                        .rounded(px(14.0))
                        .bg(rgb(roles.surface))
                        .p_3()
                        .child(
                            v_flex().gap_2().child(
                                h_flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_size(
                                                        miaominal_settings::FontSize::Body.scaled(),
                                                    )
                                                    .text_color(rgb(roles.on_surface))
                                                    .child(snippet.description.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_size(
                                                        miaominal_settings::FontSize::Body.scaled(),
                                                    )
                                                    .text_color(rgb(text_muted))
                                                    .child(preview),
                                            ),
                                    )
                                    .child(div().id(button_id).child(icon_button(
                                        AppIcon::Play,
                                        36.0,
                                        12.0,
                                        Some(roles.primary),
                                        Some(roles.on_primary),
                                        None,
                                        move |_window, cx| {
                                            let script = script.clone();
                                            send_entity.update(cx, |this, cx| {
                                                this.send_paste_text(script.clone(), cx);
                                            });
                                        },
                                    ))),
                            ),
                        ),
                );
            }
            v_flex()
                .w_full()
                .gap_3()
                .when(!package_summaries.is_empty(), |this| {
                    this.child(
                        v_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(miaominal_settings::FontSize::Body.scaled())
                                    .text_color(rgb(roles.on_surface))
                                    .child(i18n::string("snippets.page.packages")),
                            )
                            .child(
                                v_flex().w_full().gap_2().children(
                                    package_summaries.into_iter().map(|(package, count)| {
                                        let package_name = package.clone();
                                        let is_selected = selected_package_filter
                                            .is_some_and(|selected| {
                                                selected.eq_ignore_ascii_case(package_name.as_str())
                                            });
                                        session_snippet_package_card(
                                            package,
                                            count,
                                            is_selected,
                                            {
                                                let entity = entity.clone();
                                                move |_, cx| {
                                                    let package_name = package_name.clone();
                                                    entity.update(cx, |this, cx| {
                                                        this.handle_workspace_snippets_package_filter_toggle(
                                                            package_name.clone(),
                                                            cx,
                                                        );
                                                    });
                                                }
                                            },
                                        )
                                    }),
                                ),
                            ),
                    )
                })
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            div()
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .text_color(rgb(roles.on_surface))
                                .child(i18n::string("snippets.page.snippets")),
                        )
                        .child(list),
                )
                .into_any_element()
        };

        v_flex()
            .id("session-snippets-panel-content")
            .size_full()
            .gap_3()
            .overflow_hidden()
            .p_3()
            .when(!self.data.snippets.is_empty(), |this| {
                this.child(search_filter_input(
                    &self.workspace_forms.snippets_panel.filter_input,
                    SearchInputStyle::Compact,
                    None,
                ))
            })
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scrollbar()
                    .child(content),
            )
            .into_any_element()
    }

    fn advance_terminal_search_overlay(&mut self, window: &mut Window) -> Option<f32> {
        let search = &mut self.workspace_forms.search;

        if let Some(animation) = search.animation {
            let duration_seconds = animation.duration.as_secs_f32();
            if duration_seconds <= f32::EPSILON {
                search.visibility = animation.to;
                search.animation = None;
            } else {
                let elapsed = Instant::now().saturating_duration_since(animation.started_at);
                let progress = (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0);
                let eased = progress * progress * (3.0 - 2.0 * progress);
                search.visibility = animation.from + (animation.to - animation.from) * eased;

                if progress >= 1.0 {
                    search.visibility = animation.to;
                    search.animation = None;
                } else {
                    window.request_animation_frame();
                }
            }
        }

        if search.visibility <= f32::EPSILON && !search.open {
            search.visible = false;
            search.total = 0;
            search.current = None;
            search.status = None;
            return None;
        }

        if search.open || search.visibility > f32::EPSILON {
            search.visible = true;
            return Some(search.visibility.clamp(0.0, 1.0));
        }

        search.visible = false;
        None
    }

    fn render_terminal_search_overlay(
        &self,
        visibility: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let button_background = roles.surface_container_high;
        let search = &self.workspace_forms.search;
        let total = search.total;
        let current = search.current;
        let counter = if let Some(message) = &search.status {
            message.clone()
        } else if total == 0 {
            "0/0".to_string()
        } else {
            let display_index = current.map(|i| i + 1).unwrap_or(0);
            format!("{display_index}/{total}")
        };

        let prev_entity = cx.entity().clone();
        let next_entity = cx.entity().clone();
        let close_entity = cx.entity().clone();

        div()
            .absolute()
            .top(px(12.0))
            .right(px(28.0))
            .occlude()
            .w(px(440.0))
            .opacity(visibility)
            .child(
                search_filter_input(
                    &self.workspace_forms.search.input,
                    SearchInputStyle::Compact,
                    Some(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .pr_1()
                            .child(
                                div()
                                    .min_w(px(48.0))
                                    .text_size(miaominal_settings::FontSize::Body.scaled())
                                    .text_color(rgb(text_muted))
                                    .child(counter),
                            )
                            .child(icon_button(
                                AppIcon::ChevronUp,
                                24.0,
                                8.0,
                                Some(button_background),
                                Some(text_muted),
                                None,
                                move |_, cx| {
                                    let entity = prev_entity.clone();
                                    entity.update(cx, |this, cx| this.terminal_search_prev(cx));
                                },
                            ))
                            .child(icon_button(
                                AppIcon::ChevronDown,
                                24.0,
                                8.0,
                                Some(button_background),
                                Some(text_muted),
                                None,
                                move |_, cx| {
                                    let entity = next_entity.clone();
                                    entity.update(cx, |this, cx| this.terminal_search_next(cx));
                                },
                            ))
                            .child(icon_button(
                                AppIcon::Close,
                                24.0,
                                8.0,
                                Some(button_background),
                                Some(text_muted),
                                None,
                                move |window, cx| {
                                    let entity = close_entity.clone();
                                    entity.update(cx, |this, cx| {
                                        this.close_terminal_search(window, cx)
                                    });
                                },
                            ))
                            .into_any_element(),
                    ),
                )
                .bg(rgb(roles.surface_container_highest)),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod monitor_layout_tests {
    use super::*;

    #[test]
    fn monitor_time_labels_stay_short_for_minute_windows() {
        for point_count in [150, 300, 900] {
            let labels = build_monitor_time_axis_labels(point_count);

            assert!(
                labels
                    .iter()
                    .all(|label| !label.contains("m ") && !label.contains('秒')),
                "unexpected verbose labels for {point_count} points: {labels:?}"
            );
        }
    }
}
