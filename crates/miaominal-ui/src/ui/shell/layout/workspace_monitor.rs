use crate::ui::components::{editor_button, md3_spinner};
use crate::ui::i18n;
use gpui::{linear_color_stop, linear_gradient};
use gpui_component::{
    ActiveTheme,
    plot::{
        Grid, IntoPlot, Plot, StrokeStyle,
        scale::{Scale, ScaleLinear, ScalePoint},
        shape::Area,
    },
};

use super::super::*;

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

const SESSION_MONITOR_CHART_HEIGHT: f32 = 60.0;
const SESSION_MONITOR_SAMPLE_INTERVAL_SECS: usize = 2;
const SESSION_MONITOR_PERCENT_MIN_Y_MAX: f64 = 2.0;
fn tail_history(history: &[MonitorChartPoint], limit: usize) -> Vec<MonitorChartPoint> {
    let start = history.len().saturating_sub(limit);
    history[start..].to_vec()
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
