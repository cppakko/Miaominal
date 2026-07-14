use crate::ui::components::{editor_button, md3_select, md3_spinner};
use crate::ui::i18n;
use gpui::{StatefulInteractiveElement, linear_color_stop, linear_gradient};
use gpui_component::{
    ActiveTheme, Size,
    plot::{
        Grid, IntoPlot, Plot, StrokeStyle,
        scale::{Scale, ScaleLinear, ScalePoint},
        shape::Area,
    },
};
use miaominal_core::forwarding::SessionMonitorPlatform;

use super::super::*;

const SESSION_MONITOR_CHART_HEIGHT: f32 = 88.0;
const SESSION_MONITOR_SAMPLE_INTERVAL_SECS: usize = 2;
const SESSION_MONITOR_NETWORK_MIN_Y_MAX: f64 = 8.0;

fn monitor_tooltip(
    text: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView {
    let text = text.into();
    move |window, cx| gpui_component::tooltip::Tooltip::new(text.clone()).build(window, cx)
}

fn short_monitor_timestamp(value: &str) -> String {
    value
        .split_whitespace()
        .find(|component| {
            let parts = component.split(':').collect::<Vec<_>>();
            parts.len() == 3
                && parts[0].len() == 2
                && parts[1].len() == 2
                && parts[2].len() == 2
                && parts
                    .iter()
                    .all(|part| part.chars().all(|ch| ch.is_ascii_digit()))
        })
        .unwrap_or(value)
        .to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum MonitorHealth {
    Normal,
    Warning,
    Critical,
}

struct MonitorTrendCardConfig<'a> {
    title: String,
    primary_label: String,
    primary_value: String,
    primary_history: &'a [MonitorChartPoint],
    primary_color: u32,
    secondary_label: String,
    secondary_value: String,
    secondary_history: &'a [MonitorChartPoint],
    secondary_color: u32,
    y_max: f64,
    y_ticks: Vec<f64>,
    y_tick_labels: [String; 3],
}

fn tail_history(history: &[MonitorChartPoint], limit: usize) -> Vec<MonitorChartPoint> {
    let start = history.len().saturating_sub(limit);
    history[start..].to_vec()
}

fn monitor_axis_labels(
    primary: &[MonitorChartPoint],
    secondary: &[MonitorChartPoint],
) -> Vec<String> {
    let mut labels = primary
        .iter()
        .chain(secondary.iter())
        .map(|point| point.label.clone())
        .collect::<Vec<_>>();
    labels.sort_by(
        |left, right| match (left.parse::<usize>(), right.parse::<usize>()) {
            (Ok(left), Ok(right)) => left.cmp(&right),
            _ => left.cmp(right),
        },
    );
    labels.dedup();
    labels
}

#[derive(Clone, IntoPlot)]
struct MonitorTrendChart {
    primary: Vec<MonitorChartPoint>,
    primary_color: u32,
    secondary: Vec<MonitorChartPoint>,
    secondary_color: u32,
    y_max: f64,
    y_ticks: Vec<f64>,
}

impl MonitorTrendChart {
    fn new(
        primary: Vec<MonitorChartPoint>,
        primary_color: u32,
        secondary: Vec<MonitorChartPoint>,
        secondary_color: u32,
        y_max: f64,
        y_ticks: Vec<f64>,
    ) -> Self {
        Self {
            primary,
            primary_color,
            secondary,
            secondary_color,
            y_max,
            y_ticks,
        }
    }
}

impl Plot for MonitorTrendChart {
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        if self.primary.is_empty() && self.secondary.is_empty() {
            return;
        }

        let width = bounds.size.width.as_f32();
        let height = bounds.size.height.as_f32();
        let y_max = self.y_max.max(1.0);
        let labels = monitor_axis_labels(&self.primary, &self.secondary);
        let y = ScaleLinear::new(vec![0.0, y_max], vec![height - 3.0, 3.0]);
        let grid_ticks = self
            .y_ticks
            .iter()
            .filter_map(|tick| y.tick(tick))
            .collect::<Vec<_>>();

        if !grid_ticks.is_empty() {
            Grid::new()
                .y(grid_ticks)
                .stroke(cx.theme().border.opacity(0.72))
                .dash_array(&[px(4.0), px(2.0)])
                .paint(&bounds, window);
        }

        for (data, color) in [
            (&self.primary, self.primary_color),
            (&self.secondary, self.secondary_color),
        ] {
            if data.is_empty() {
                continue;
            }

            let x = ScalePoint::new(labels.clone(), vec![0.0, width]);
            let y = ScaleLinear::new(vec![0.0, y_max], vec![height - 3.0, 3.0]);
            let stroke = rgb(color);
            let fill = linear_gradient(
                0.0,
                linear_color_stop(stroke.opacity(0.18), 1.0),
                linear_color_stop(cx.theme().background.opacity(0.02), 0.0),
            );

            Area::new()
                .data(data.clone())
                .x(move |point| x.tick(&point.label))
                .y0(height - 3.0)
                .y1(move |point| y.tick(&point.value))
                .fill(fill)
                .stroke(stroke)
                .stroke_style(StrokeStyle::Linear)
                .paint(&bounds, window);
        }
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

fn format_usage_detail(used: u64, total: u64) -> String {
    let used = format_byte_size(Some(used)).to_string();
    let total = format_byte_size(Some(total)).to_string();
    i18n::string_args(
        "workspace.panel.monitor.usage_detail",
        &[("used", &used), ("total", &total)],
    )
}

fn format_uptime(total_seconds: u64) -> String {
    let total_minutes = total_seconds / 60;
    let days = total_minutes / (24 * 60);
    let hours = (total_minutes / 60) % 24;
    let minutes = total_minutes % 60;

    if days > 0 {
        return i18n::string_args(
            "workspace.panel.monitor.uptime_days_hours",
            &[("days", &days.to_string()), ("hours", &hours.to_string())],
        );
    }

    if total_minutes >= 60 {
        return i18n::string_args(
            "workspace.panel.monitor.uptime_hours_minutes",
            &[
                ("hours", &(total_minutes / 60).to_string()),
                ("minutes", &minutes.to_string()),
            ],
        );
    }

    i18n::string_args(
        "workspace.panel.monitor.uptime_minutes",
        &[("minutes", &minutes.to_string())],
    )
}

fn platform_label(platform: SessionMonitorPlatform) -> &'static str {
    match platform {
        SessionMonitorPlatform::Linux => "Linux",
        SessionMonitorPlatform::Macos => "macOS",
        SessionMonitorPlatform::Windows => "Windows",
    }
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

    (max_chars * font_size * 0.72 + font_size).clamp(20.0, 48.0)
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

fn threshold_health(value: f64, warning: f64, critical: f64) -> MonitorHealth {
    if value >= critical {
        MonitorHealth::Critical
    } else if value >= warning {
        MonitorHealth::Warning
    } else {
        MonitorHealth::Normal
    }
}

fn load_health(snapshot: &SessionMonitorSnapshot) -> MonitorHealth {
    let Some(cpu_count) = snapshot.logical_cpu_count.filter(|count| *count > 0) else {
        return MonitorHealth::Normal;
    };
    let per_core = snapshot.load.max(0.0) / cpu_count as f64;
    match snapshot.platform {
        SessionMonitorPlatform::Windows => threshold_health(per_core, 0.5, 1.0),
        SessionMonitorPlatform::Linux | SessionMonitorPlatform::Macos => {
            threshold_health(per_core, 0.8, 1.0)
        }
    }
}

fn monitor_health(snapshot: &SessionMonitorSnapshot, cpu_sample_ready: bool) -> MonitorHealth {
    let mut health =
        threshold_health(snapshot.memory_percent, 80.0, 90.0).max(load_health(snapshot));
    if snapshot.disk_total_bytes.is_some_and(|total| total > 0) {
        health = health.max(threshold_health(snapshot.disk_percent, 80.0, 90.0));
    }
    if cpu_sample_ready {
        health = health.max(threshold_health(snapshot.cpu_percent, 80.0, 95.0));
    }
    health
}

fn monitor_health_label(health: MonitorHealth) -> String {
    i18n::string(match health {
        MonitorHealth::Normal => "workspace.panel.monitor.health_normal",
        MonitorHealth::Warning => "workspace.panel.monitor.health_warning",
        MonitorHealth::Critical => "workspace.panel.monitor.health_critical",
    })
}

fn monitor_health_accent(health: MonitorHealth) -> u32 {
    let material = miaominal_settings::current_theme().material;
    match health {
        MonitorHealth::Normal => material.extended.success.color,
        MonitorHealth::Warning => material.extended.warning.color,
        MonitorHealth::Critical => material.roles.error,
    }
}

impl AppView {
    pub(in crate::ui::shell::layout) fn render_session_monitor_panel(
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

        let Some(snapshot) = monitoring.last_snapshot.as_ref() else {
            if let Some(error) = monitoring.last_error.as_ref() {
                return self.render_monitor_error_state(entity, error, text_muted);
            }

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

        let limit = self
            .settings_store
            .settings()
            .monitor_history_duration
            .history_limit();
        let cpu_history = tail_history(&monitoring.cpu_history, limit);
        let memory_history = tail_history(&monitoring.memory_history, limit);
        let network_rx_history = tail_history(&monitoring.network_rx_history, limit);
        let network_tx_history = tail_history(&monitoring.network_tx_history, limit);
        let network_peak = network_rx_history
            .iter()
            .chain(network_tx_history.iter())
            .map(|point| point.value)
            .fold(
                snapshot.network_rx_kbps.max(snapshot.network_tx_kbps),
                f64::max,
            );
        let network_y_max = nice_chart_max(network_peak, SESSION_MONITOR_NETWORK_MIN_Y_MAX);
        let network_y_ticks = build_chart_ticks(network_y_max);
        let cpu_sample_ready = monitoring.cpu_sample_ready;
        let network_sample_ready = monitoring.network_sample_ready;
        let overall_health = monitor_health(snapshot, cpu_sample_ready);
        let profile = self
            .data
            .sessions
            .iter()
            .find(|profile| profile.id == session.profile_id)
            .or(session.pending_profile.as_ref());
        let endpoint = profile
            .map(SessionProfile::summary)
            .unwrap_or_else(|| session.profile_id.clone());
        let hostname = snapshot
            .hostname
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .or_else(|| {
                profile
                    .map(|profile| profile.name.trim())
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| endpoint.clone());
        let mut system_details = vec![platform_label(snapshot.platform).to_string()];
        if let Some(cpu_count) = snapshot.logical_cpu_count {
            system_details.push(i18n::string_args(
                "workspace.panel.monitor.cpu_cores",
                &[("count", &cpu_count.to_string())],
            ));
        }
        if let Some(uptime) = snapshot.uptime_seconds {
            system_details.push(i18n::string_args(
                "workspace.panel.monitor.uptime",
                &[("value", &format_uptime(uptime))],
            ));
        }
        let system_details = system_details.join(" · ");
        let system_details_display = truncate_with_ellipsis(&system_details, 80);
        let updated_at = format_local_timestamp(monitoring.last_updated_at).to_string();
        let updated_label = i18n::string_args(
            "workspace.panel.monitor.updated_at",
            &[("time", &short_monitor_timestamp(&updated_at))],
        );
        let updated_tooltip = i18n::string_args(
            "workspace.panel.monitor.updated_at",
            &[("time", &updated_at)],
        );
        let status_badge = if monitoring.last_error.is_some() {
            self.render_monitor_status_badge(
                i18n::string("workspace.panel.monitor.health_stale"),
                MonitorHealth::Warning,
            )
        } else {
            self.render_monitor_status_badge(monitor_health_label(overall_health), overall_health)
        };
        let cpu_health = if cpu_sample_ready {
            threshold_health(snapshot.cpu_percent, 80.0, 95.0)
        } else {
            MonitorHealth::Normal
        };
        let memory_health = threshold_health(snapshot.memory_percent, 80.0, 90.0);
        let disk_available = snapshot.disk_total_bytes.is_some_and(|total| total > 0);
        let disk_health = if disk_available {
            threshold_health(snapshot.disk_percent, 80.0, 90.0)
        } else {
            MonitorHealth::Normal
        };
        let load_health = load_health(snapshot);
        let cpu_value = if cpu_sample_ready {
            format_percentage(snapshot.cpu_percent)
        } else {
            i18n::string("workspace.panel.monitor.warming_up")
        };
        let cpu_detail = snapshot
            .logical_cpu_count
            .map(|count| {
                i18n::string_args(
                    "workspace.panel.monitor.cpu_cores",
                    &[("count", &count.to_string())],
                )
            })
            .unwrap_or_else(|| i18n::string("workspace.panel.monitor.unavailable"));
        let memory_usage =
            format_usage_detail(snapshot.memory_used_bytes, snapshot.memory_total_bytes);
        let disk_detail = match (snapshot.disk_used_bytes, snapshot.disk_total_bytes) {
            (Some(used), Some(total)) if total > 0 => format_usage_detail(used, total),
            _ => i18n::string("workspace.panel.monitor.unavailable"),
        };
        let disk_value = if disk_available {
            format_percentage(snapshot.disk_percent)
        } else {
            i18n::string("workspace.panel.monitor.unavailable")
        };
        let load_per_core = snapshot
            .logical_cpu_count
            .filter(|count| *count > 0)
            .map(|count| snapshot.load.max(0.0) / count as f64)
            .map(|value| {
                i18n::string_args(
                    "workspace.panel.monitor.load_per_core",
                    &[("value", &format_load_label(value))],
                )
            })
            .unwrap_or_else(|| i18n::string("workspace.panel.monitor.unavailable"));
        let load_title = i18n::string(match snapshot.platform {
            SessionMonitorPlatform::Windows => "workspace.panel.monitor.processor_queue",
            SessionMonitorPlatform::Linux | SessionMonitorPlatform::Macos => {
                "workspace.panel.monitor.load"
            }
        });
        let network_download = if network_sample_ready {
            format_rate_label(snapshot.network_rx_kbps)
        } else {
            i18n::string("workspace.panel.monitor.warming_up")
        };
        let network_upload = if network_sample_ready {
            format_rate_label(snapshot.network_tx_kbps)
        } else {
            i18n::string("workspace.panel.monitor.warming_up")
        };
        let percent_ticks = vec![0.0, 50.0, 100.0];
        let chart_primary = roles.primary;
        let chart_secondary = roles.tertiary;
        let chart_download = material.extended.info.color;
        let chart_upload = material.extended.warning.color;
        let history_select = self.panel_forms.settings.monitor_history_select.clone();

        let header = v_flex()
            .w_full()
            .flex_shrink_0()
            .gap_1()
            .px_3()
            .pt_3()
            .pb_2()
            .child(
                h_flex()
                    .w_full()
                    .items_start()
                    .justify_between()
                    .gap_2()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(0.0))
                            .gap_1()
                            .child(
                                div()
                                    .id("session-monitor-hostname")
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(miaominal_settings::FontSize::Subheading.scaled())
                                    .text_color(rgb(roles.on_surface))
                                    .tooltip(monitor_tooltip(hostname.clone()))
                                    .child(hostname),
                            )
                            .child(
                                div()
                                    .id("session-monitor-endpoint")
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(miaominal_settings::FontSize::Body.scaled())
                                    .text_color(rgb(text_muted))
                                    .tooltip(monitor_tooltip(endpoint.clone()))
                                    .child(endpoint),
                            ),
                    )
                    .child(status_badge),
            )
            .child(
                div()
                    .id("session-monitor-system-details")
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(miaominal_settings::scaled_font_size(11.0))
                    .text_color(rgb(text_muted))
                    .tooltip(monitor_tooltip(system_details.clone()))
                    .child(system_details_display),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .id("session-monitor-updated-at")
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(miaominal_settings::scaled_font_size(11.0))
                            .text_color(rgb(text_muted))
                            .tooltip(monitor_tooltip(updated_tooltip))
                            .child(updated_label),
                    )
                    .child(
                        div()
                            .w(px(112.0))
                            .flex_shrink_0()
                            .child(md3_select(&history_select).with_size(Size::Small).w_full()),
                    ),
            );

        let content = v_flex()
            .w_full()
            .gap_3()
            .when_some(monitoring.last_error.as_ref(), |this, error| {
                this.child(self.render_monitor_stale_banner(entity.clone(), error))
            })
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        h_flex()
                            .w_full()
                            .items_stretch()
                            .gap_2()
                            .child(div().flex_1().min_w(px(0.0)).child(
                                self.render_monitor_metric_card(
                                    "session-monitor-cpu-detail",
                                    i18n::string("workspace.panel.monitor.cpu"),
                                    cpu_value.clone(),
                                    cpu_detail,
                                    cpu_health,
                                    cpu_sample_ready.then_some(snapshot.cpu_percent),
                                ),
                            ))
                            .child(div().flex_1().min_w(px(0.0)).child(
                                self.render_monitor_metric_card(
                                    "session-monitor-memory-detail",
                                    i18n::string("workspace.panel.monitor.memory"),
                                    format_percentage(snapshot.memory_percent),
                                    memory_usage,
                                    memory_health,
                                    Some(snapshot.memory_percent),
                                ),
                            )),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .items_stretch()
                            .gap_2()
                            .child(div().flex_1().min_w(px(0.0)).child(
                                self.render_monitor_metric_card(
                                    "session-monitor-disk-detail",
                                    i18n::string("workspace.panel.monitor.disk"),
                                    disk_value,
                                    disk_detail,
                                    disk_health,
                                    disk_available.then_some(snapshot.disk_percent),
                                ),
                            ))
                            .child(div().flex_1().min_w(px(0.0)).child(
                                self.render_monitor_metric_card(
                                    "session-monitor-load-detail",
                                    load_title,
                                    format_load_label(snapshot.load),
                                    load_per_core,
                                    load_health,
                                    None,
                                ),
                            )),
                    ),
            )
            .child(self.render_monitor_trend_card(MonitorTrendCardConfig {
                title: i18n::string("workspace.panel.monitor.resource_trend"),
                primary_label: i18n::string("workspace.panel.monitor.cpu"),
                primary_value: cpu_value,
                primary_history: &cpu_history,
                primary_color: chart_primary,
                secondary_label: i18n::string("workspace.panel.monitor.memory"),
                secondary_value: format_percentage(snapshot.memory_percent),
                secondary_history: &memory_history,
                secondary_color: chart_secondary,
                y_max: 100.0,
                y_ticks: percent_ticks.clone(),
                y_tick_labels: build_chart_tick_labels(&percent_ticks, format_percentage),
            }))
            .child(self.render_monitor_trend_card(MonitorTrendCardConfig {
                title: i18n::string("workspace.panel.monitor.network_trend"),
                primary_label: i18n::string("workspace.panel.monitor.download"),
                primary_value: network_download,
                primary_history: &network_rx_history,
                primary_color: chart_download,
                secondary_label: i18n::string("workspace.panel.monitor.upload"),
                secondary_value: network_upload,
                secondary_history: &network_tx_history,
                secondary_color: chart_upload,
                y_max: network_y_max,
                y_ticks: network_y_ticks.clone(),
                y_tick_labels: build_chart_tick_labels(&network_y_ticks, format_rate_label),
            }))
            .into_any_element();

        v_flex()
            .id("session-monitor-panel-content")
            .size_full()
            .min_h_0()
            .child(header)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("session-monitor-scroll")
                            .size_full()
                            .track_scroll(&monitor_scroll_handle)
                            .overflow_y_scroll()
                            .child(
                                v_flex()
                                    .w_full()
                                    .min_h_full()
                                    .px_3()
                                    .pt_2()
                                    .pb_3()
                                    .child(content),
                            ),
                    )
                    .vertical_scrollbar(&monitor_scroll_handle),
            )
            .into_any_element()
    }

    fn render_monitor_status_badge(
        &self,
        label: String,
        health: MonitorHealth,
    ) -> gpui::AnyElement {
        let accent = monitor_health_accent(health);
        let background_alpha = if health == MonitorHealth::Critical {
            0x20
        } else {
            0x16
        };
        h_flex()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .rounded(px(999.0))
            .bg(color_with_alpha(accent, background_alpha))
            .text_size(miaominal_settings::scaled_font_size(11.0))
            .text_color(rgb(accent))
            .child(div().size(px(5.0)).rounded(px(999.0)).bg(rgb(accent)))
            .child(label)
            .into_any_element()
    }

    fn render_monitor_metric_card(
        &self,
        detail_id: &'static str,
        title: String,
        value: String,
        detail: String,
        health: MonitorHealth,
        progress_percent: Option<f64>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let background = match health {
            MonitorHealth::Critical => color_with_alpha(roles.error, 0x18),
            MonitorHealth::Normal | MonitorHealth::Warning => rgb(roles.surface),
        };
        let accent = monitor_health_accent(health);
        let value_color = match health {
            MonitorHealth::Normal => roles.on_surface,
            MonitorHealth::Warning => material.extended.warning.color,
            MonitorHealth::Critical => roles.error,
        };
        let compact_value = value.clone();

        v_flex()
            .w_full()
            .min_h(px(84.0))
            .justify_between()
            .gap_1()
            .rounded(px(14.0))
            .bg(background)
            .p_2()
            .child(
                h_flex()
                    .w_full()
                    .min_w(px(0.0))
                    .items_center()
                    .gap_1()
                    .child(div().size(px(6.0)).rounded(px(999.0)).bg(rgb(accent)))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(miaominal_settings::FontSize::Body.scaled())
                            .text_color(rgb(text_muted))
                            .child(title),
                    )
                    .when(progress_percent.is_some(), |this| {
                        this.child(
                            div()
                                .flex_shrink_0()
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .text_color(rgb(value_color))
                                .child(compact_value),
                        )
                    }),
            )
            .when_some(progress_percent, |this, progress_percent| {
                let progress = if progress_percent.is_finite() {
                    (progress_percent.clamp(0.0, 100.0) / 100.0) as f32
                } else {
                    0.0
                };
                this.child(
                    div()
                        .w_full()
                        .h(px(6.0))
                        .overflow_hidden()
                        .rounded(px(999.0))
                        .bg(rgb(roles.surface_container_highest))
                        .child(
                            div()
                                .h_full()
                                .w(gpui::relative(progress))
                                .rounded(px(999.0))
                                .bg(rgb(accent)),
                        ),
                )
            })
            .when(progress_percent.is_none(), |this| {
                this.child(
                    div()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(miaominal_settings::FontSize::Subheading.scaled())
                        .text_color(rgb(value_color))
                        .child(value),
                )
            })
            .child(
                div()
                    .id(detail_id)
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(miaominal_settings::scaled_font_size(11.0))
                    .text_color(rgb(text_muted))
                    .tooltip(monitor_tooltip(detail.clone()))
                    .child(detail),
            )
            .into_any_element()
    }

    fn render_monitor_stale_banner(&self, entity: Entity<Self>, error: &str) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let warning = material.extended.warning;
        let error_preview = truncate_with_ellipsis(error, 140);
        v_flex()
            .w_full()
            .gap_1()
            .rounded(px(14.0))
            .bg(color_with_alpha(warning.color, 0x18))
            .p_2()
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_size(miaominal_settings::FontSize::Body.scaled())
                            .text_color(rgb(warning.color))
                            .child(i18n::string("workspace.panel.monitor.stale_title")),
                    )
                    .child(editor_button(
                        i18n::string("workspace.panel.monitor.retry"),
                        false,
                        true,
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.enable_active_session_monitoring(cx);
                            });
                        },
                    )),
            )
            .child(
                div()
                    .id("session-monitor-stale-error")
                    .max_h(px(34.0))
                    .overflow_hidden()
                    .text_size(miaominal_settings::scaled_font_size(11.0))
                    .text_color(rgb(material.roles.on_surface_variant))
                    .tooltip(monitor_tooltip(error.to_string()))
                    .child(error_preview),
            )
            .into_any_element()
    }

    fn render_monitor_error_state(
        &self,
        entity: Entity<Self>,
        error: &str,
        text_muted: u32,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let error_preview = truncate_with_ellipsis(error, 180);
        v_flex()
            .id("session-monitor-panel-content")
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .p_3()
            .child(
                div()
                    .text_center()
                    .text_size(miaominal_settings::FontSize::Input.scaled())
                    .text_color(rgb(roles.on_surface))
                    .child(i18n::string("workspace.panel.monitor.error_title")),
            )
            .child(
                div()
                    .id("session-monitor-error-message")
                    .max_w(px(280.0))
                    .max_h(px(52.0))
                    .overflow_hidden()
                    .text_center()
                    .text_size(miaominal_settings::FontSize::Body.scaled())
                    .text_color(rgb(text_muted))
                    .tooltip(monitor_tooltip(error.to_string()))
                    .child(error_preview),
            )
            .child(editor_button(
                i18n::string("workspace.panel.monitor.retry"),
                false,
                true,
                move |_, cx| {
                    entity.update(cx, |this, cx| {
                        this.enable_active_session_monitoring(cx);
                    });
                },
            ))
            .into_any_element()
    }

    fn render_monitor_trend_card(&self, config: MonitorTrendCardConfig<'_>) -> gpui::AnyElement {
        let MonitorTrendCardConfig {
            title,
            primary_label,
            primary_value,
            primary_history,
            primary_color,
            secondary_label,
            secondary_value,
            secondary_history,
            secondary_color,
            y_max,
            y_ticks,
            y_tick_labels,
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
        let primary_data = primary_history.to_vec();
        let secondary_data = secondary_history.to_vec();
        let point_count = monitor_axis_labels(&primary_data, &secondary_data).len();
        let time_axis_labels =
            (point_count > 0).then(|| build_monitor_time_axis_labels(point_count));

        let chart = if point_count == 0 {
            div()
                .h(px(SESSION_MONITOR_CHART_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .text_size(miaominal_settings::FontSize::Body.scaled())
                .text_color(rgb(text_muted))
                .child(i18n::string("workspace.panel.monitor.warming_up"))
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
                        .child(MonitorTrendChart::new(
                            primary_data,
                            primary_color,
                            secondary_data,
                            secondary_color,
                            y_max,
                            y_ticks,
                        )),
                )
                .into_any_element()
        };

        v_flex()
            .w_full()
            .gap_1()
            .rounded(px(14.0))
            .bg(rgb(roles.surface))
            .p_2()
            .child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(miaominal_settings::FontSize::Body.scaled())
                    .text_color(rgb(roles.on_surface))
                    .child(title),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(div().flex_1().min_w(px(0.0)).overflow_hidden().child(
                        self.render_monitor_legend_item(
                            primary_color,
                            primary_label,
                            primary_value,
                        ),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .flex()
                            .justify_end()
                            .child(self.render_monitor_legend_item(
                                secondary_color,
                                secondary_label,
                                secondary_value,
                            )),
                    ),
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

    fn render_monitor_legend_item(
        &self,
        color: u32,
        label: String,
        value: String,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        h_flex()
            .min_w(px(0.0))
            .overflow_hidden()
            .gap_1()
            .items_center()
            .child(
                div()
                    .size(px(6.0))
                    .flex_shrink_0()
                    .rounded(px(999.0))
                    .bg(rgb(color)),
            )
            .child(
                div()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(miaominal_settings::scaled_font_size(11.0))
                    .text_color(rgb(roles.on_surface_variant))
                    .child(label),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(miaominal_settings::scaled_font_size(11.0))
                    .text_color(rgb(roles.on_surface))
                    .child(value),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod monitor_layout_tests {
    use super::*;

    fn snapshot(platform: SessionMonitorPlatform) -> SessionMonitorSnapshot {
        SessionMonitorSnapshot {
            platform,
            hostname: Some("host".into()),
            logical_cpu_count: Some(4),
            uptime_seconds: Some(60),
            cpu_percent: 10.0,
            memory_percent: 20.0,
            memory_used_bytes: 2,
            memory_total_bytes: 10,
            swap_percent: 0.0,
            swap_used_bytes: 0,
            swap_total_bytes: 0,
            disk_percent: 30.0,
            disk_used_bytes: Some(3),
            disk_total_bytes: Some(10),
            network_rx_kbps: 0.0,
            network_tx_kbps: 0.0,
            load: 1.0,
        }
    }

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

    #[test]
    fn monitor_axis_labels_merge_equal_length_histories_after_rewarm() {
        let primary = ["3", "4", "6"].map(|label| MonitorChartPoint {
            label: label.into(),
            value: 1.0,
        });
        let secondary = ["4", "5", "6"].map(|label| MonitorChartPoint {
            label: label.into(),
            value: 2.0,
        });

        assert_eq!(
            monitor_axis_labels(&primary, &secondary),
            ["3", "4", "5", "6"]
        );
    }

    #[test]
    fn monitor_timestamp_uses_compact_clock_time() {
        assert_eq!(short_monitor_timestamp("2026-07-13 09:08:07"), "09:08:07");
        assert_eq!(short_monitor_timestamp("unknown"), "unknown");
    }

    #[test]
    fn monitor_health_uses_fixed_resource_thresholds() {
        let mut value = snapshot(SessionMonitorPlatform::Linux);
        assert_eq!(monitor_health(&value, true), MonitorHealth::Normal);

        value.memory_percent = 80.0;
        assert_eq!(monitor_health(&value, true), MonitorHealth::Warning);

        value.memory_percent = 20.0;
        value.disk_percent = 90.0;
        assert_eq!(monitor_health(&value, true), MonitorHealth::Critical);
    }

    #[test]
    fn monitor_health_ignores_cpu_until_rate_sample_is_ready() {
        let mut value = snapshot(SessionMonitorPlatform::Linux);
        value.cpu_percent = 99.0;

        assert_eq!(monitor_health(&value, false), MonitorHealth::Normal);
        assert_eq!(monitor_health(&value, true), MonitorHealth::Critical);
    }

    #[test]
    fn load_health_is_platform_aware() {
        let mut unix = snapshot(SessionMonitorPlatform::Linux);
        unix.load = 3.2;
        assert_eq!(load_health(&unix), MonitorHealth::Warning);

        let mut windows = snapshot(SessionMonitorPlatform::Windows);
        windows.load = 2.0;
        assert_eq!(load_health(&windows), MonitorHealth::Warning);
        windows.load = 4.0;
        assert_eq!(load_health(&windows), MonitorHealth::Critical);
    }
}
