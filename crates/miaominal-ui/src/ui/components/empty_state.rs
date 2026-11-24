use crate::ui::assets::AppIcon;
use crate::ui::components::shell_tokens::{ShellColors, space};
use crate::ui::components::tokens::color_with_alpha;
use gpui_kit::component::{
    Icon,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, Background, Bounds, Div, ElementId, FontWeight, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, PathBuilder, Pixels, Point, SharedString, Styled as _,
    TestSupportExt as _, Window, canvas, div, linear_color_stop, linear_gradient, point,
    prelude::*, px, rgb,
};
use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(in crate::ui) enum EmptyStateTone {
    /// Unified Material 3 primary tonal presentation for all standard empty states.
    #[default]
    Primary,
    /// Recoverable error state (`error_container`).
    ErrorRecovery,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(in crate::ui) enum EmptyStateSize {
    /// Full management page or main workspace viewport.
    #[default]
    Page,
    /// Side panel, drawer, or inspector pane.
    Panel,
    /// Compact card region, table viewport, or popover list.
    Compact,
    /// Inline row inside a dropdown picker, combobox, or settings bar.
    Inline,
}

#[derive(Clone, Copy)]
struct TonePalette {
    backdrop_from: Hsla,
    backdrop_to: Hsla,
    core_from: Hsla,
    core_to: Hsla,
    core_icon: Hsla,
    inline_bg: Hsla,
    inline_fg: Hsla,
}

impl EmptyStateTone {
    fn palette(self) -> TonePalette {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let dark = material.dark;

        match self {
            Self::Primary => TonePalette {
                backdrop_from: color_with_alpha(
                    roles.primary_container,
                    if dark { 0x48 } else { 0x62 },
                )
                .into(),
                backdrop_to: color_with_alpha(
                    roles.secondary_container,
                    if dark { 0x38 } else { 0x4E },
                )
                .into(),
                core_from: rgb(roles.primary_container).into(),
                core_to: rgb(roles.secondary_container).into(),
                core_icon: rgb(roles.on_primary_container).into(),
                inline_bg: rgb(roles.primary_container).into(),
                inline_fg: rgb(roles.on_primary_container).into(),
            },
            Self::ErrorRecovery => TonePalette {
                backdrop_from: color_with_alpha(
                    roles.error_container,
                    if dark { 0x48 } else { 0x66 },
                )
                .into(),
                backdrop_to: color_with_alpha(
                    roles.surface_container_highest,
                    if dark { 0x3C } else { 0x54 },
                )
                .into(),
                core_from: rgb(roles.error_container).into(),
                core_to: rgb(roles.error_container).into(),
                core_icon: rgb(roles.on_error_container).into(),
                inline_bg: rgb(roles.error_container).into(),
                inline_fg: rgb(roles.on_error_container).into(),
            },
        }
    }
}

/// Paints a smooth Material 3 Expressive 4-petal organic clover using cubic Bézier splines with
/// exact analytic tangents.
fn build_m3_clover_path(bounds: Bounds<Pixels>) -> Option<gpui_kit::Path> {
    const LOBES: f32 = 4.0;
    const AMPLITUDE: f32 = 0.068;

    let center_x = f32::from(bounds.center().x);
    let center_y = f32::from(bounds.center().y);
    let base_radius =
        (f32::from(bounds.size.width).min(f32::from(bounds.size.height)) * 0.5) / (1.0 + AMPLITUDE);
    if base_radius <= 0.5 {
        return None;
    }

    let samples = 48usize;
    let step = TAU / samples as f32;
    let handle_scale = step / 3.0;

    let sample = |index: usize| -> (Point<Pixels>, (f32, f32)) {
        let theta = index as f32 * step;
        let angle_arg = LOBES * theta;
        let r = base_radius * (1.0 + AMPLITUDE * angle_arg.cos());
        let dr = -base_radius * AMPLITUDE * LOBES * angle_arg.sin();
        let (sin_t, cos_t) = theta.sin_cos();
        let x = center_x + r * cos_t;
        let y = center_y + r * sin_t;
        let tx = dr * cos_t - r * sin_t;
        let ty = dr * sin_t + r * cos_t;
        (point(px(x), px(y)), (tx, ty))
    };

    let mut path = PathBuilder::fill();
    let (start_pt, mut prev_tangent) = sample(0);
    let mut prev_pt = start_pt;
    path.move_to(start_pt);

    for i in 1..=samples {
        let (next_pt, next_tangent) = sample(i % samples);
        let ctrl_a = point(
            px(f32::from(prev_pt.x) + prev_tangent.0 * handle_scale),
            px(f32::from(prev_pt.y) + prev_tangent.1 * handle_scale),
        );
        let ctrl_b = point(
            px(f32::from(next_pt.x) - next_tangent.0 * handle_scale),
            px(f32::from(next_pt.y) - next_tangent.1 * handle_scale),
        );
        path.cubic_bezier_to(next_pt, ctrl_a, ctrl_b);
        prev_pt = next_pt;
        prev_tangent = next_tangent;
    }

    path.close();
    path.build().ok()
}

pub(in crate::ui) struct EmptyState {
    icon: Icon,
    title: SharedString,
    description: Option<SharedString>,
    tone: EmptyStateTone,
    size: EmptyStateSize,
    min_height: Option<f32>,
    action: Option<AnyElement>,
}

impl EmptyState {
    pub fn new(icon: impl Into<Icon>, copy: impl Into<SharedString>) -> Self {
        let raw: SharedString = copy.into();
        let (title, description) = split_title_and_description(raw);
        Self {
            icon: icon.into(),
            title,
            description,
            tone: EmptyStateTone::Primary,
            size: EmptyStateSize::Page,
            min_height: None,
            action: None,
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        let text: SharedString = description.into();
        if !text.as_ref().trim().is_empty() {
            self.description = Some(text);
        }
        self
    }

    pub fn tone(mut self, tone: EmptyStateTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn size(mut self, size: EmptyStateSize) -> Self {
        self.size = size;
        self
    }

    pub fn min_height(mut self, min_height: f32) -> Self {
        self.min_height = Some(min_height);
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }

    pub fn build(self) -> Div {
        let roles = miaominal_settings::current_theme().material.roles;
        let palette = self.tone.palette();

        if self.size == EmptyStateSize::Inline {
            return self.build_inline(roles.on_surface, roles.on_surface_variant, palette);
        }

        if self.size == EmptyStateSize::Compact {
            return self.build_compact(roles.on_surface, roles.on_surface_variant, palette);
        }

        let is_page = self.size == EmptyStateSize::Page;
        let default_min_height = if is_page { 220.0 } else { 164.0 };
        let outer_size = if is_page { 168.0 } else { 108.0 };
        let core_size = if is_page { 112.0 } else { 70.0 };
        let icon_size = if is_page { 54.0 } else { 34.0 };
        let max_width = if is_page { 480.0 } else { 300.0 };
        let block_gap = if is_page { 18.0 } else { 14.0 };
        let title_font = if is_page {
            miaominal_settings::FontSize::PageTitle.scaled()
        } else {
            miaominal_settings::FontSize::Title.scaled()
        };
        let title_line_height = if is_page { 28.0 } else { 22.0 };
        let desc_font = if is_page {
            miaominal_settings::FontSize::Heading.scaled()
        } else {
            miaominal_settings::FontSize::Input.scaled()
        };
        let desc_line_height = if is_page { 22.0 } else { 18.0 };

        let min_height = self.min_height.unwrap_or(default_min_height);
        let emblem = build_emblem(self.icon, outer_size, core_size, icon_size, palette);

        let text_group = v_flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap(space(if is_page { 8.0 } else { 6.0 }))
            .child(
                div()
                    .w_full()
                    .text_size(title_font)
                    .line_height(miaominal_settings::scaled_line_height(title_line_height))
                    .font_weight(FontWeight::MEDIUM)
                    .text_center()
                    .text_color(rgb(roles.on_surface))
                    .child(self.title),
            )
            .when_some(self.description, |this, description| {
                this.child(
                    div()
                        .w_full()
                        .text_size(desc_font)
                        .line_height(miaominal_settings::scaled_line_height(desc_line_height))
                        .text_center()
                        .text_color(rgb(roles.on_surface_variant))
                        .child(description),
                )
            });

        div()
            .size_full()
            .min_w_0()
            .min_h(space(min_height))
            .flex()
            .items_center()
            .justify_center()
            .px(space(if is_page { 24.0 } else { 16.0 }))
            .py(space(if is_page { 16.0 } else { 12.0 }))
            .child(
                v_flex()
                    .id("empty-state-content")
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .max_w(space(max_width))
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .gap(space(block_gap))
                    .child(emblem)
                    .child(text_group)
                    .when_some(self.action, |this, action| {
                        this.child(
                            div()
                                .pt(space(4.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(action),
                        )
                    }),
            )
    }

    pub fn build_page(self) -> Div {
        scrollable_empty_page(self.build().min_h_0().p_0())
    }

    fn build_compact(self, on_surface: u32, on_surface_variant: u32, palette: TonePalette) -> Div {
        let min_height = self.min_height.unwrap_or(0.0);
        let emblem = build_emblem(self.icon, 66.0, 44.0, 22.0, palette);

        div()
            .size_full()
            .min_w_0()
            .min_h(space(min_height))
            .flex()
            .items_center()
            .justify_center()
            .px_3()
            .py_2()
            .child(
                v_flex()
                    .id("empty-state-content")
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .max_w(space(268.0))
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .gap(space(8.0))
                    .child(emblem)
                    .child(
                        v_flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .gap(space(4.0))
                            .child(
                                div()
                                    .w_full()
                                    .text_size(miaominal_settings::FontSize::Heading.scaled())
                                    .line_height(miaominal_settings::scaled_line_height(19.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_center()
                                    .text_color(rgb(on_surface))
                                    .child(self.title),
                            )
                            .when_some(self.description, |this, description| {
                                this.child(
                                    div()
                                        .w_full()
                                        .text_size(miaominal_settings::FontSize::Input.scaled())
                                        .line_height(miaominal_settings::scaled_line_height(17.0))
                                        .text_center()
                                        .text_color(rgb(on_surface_variant))
                                        .child(description),
                                )
                            }),
                    )
                    .when_some(self.action, |this, action| {
                        this.child(
                            div()
                                .pt(space(2.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(action),
                        )
                    }),
            )
    }

    fn build_inline(self, on_surface: u32, on_surface_variant: u32, palette: TonePalette) -> Div {
        div().w_full().min_w_0().child(
            h_flex()
                .id("empty-state-content")
                .test_support()
                .w_full()
                .min_w_0()
                .items_center()
                .gap(space(10.0))
                .px(space(12.0))
                .py(space(10.0))
                .child(
                    div()
                        .size(space(28.0))
                        .flex_shrink_0()
                        .rounded(space(8.0))
                        .bg(palette.inline_bg)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(palette.inline_fg)
                        .child(self.icon.size(space(15.0))),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(space(1.0))
                        .child(
                            div()
                                .w_full()
                                .text_size(miaominal_settings::FontSize::Input.scaled())
                                .line_height(miaominal_settings::scaled_line_height(17.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(on_surface))
                                .child(self.title),
                        )
                        .when_some(self.description, |this, description| {
                            this.child(
                                div()
                                    .w_full()
                                    .text_size(miaominal_settings::FontSize::Body.scaled())
                                    .line_height(miaominal_settings::scaled_line_height(15.0))
                                    .text_color(rgb(on_surface_variant))
                                    .child(description),
                            )
                        }),
                )
                .when_some(self.action, |this, action| {
                    this.child(div().flex_shrink_0().child(action))
                }),
        )
    }
}

/// Renders a flat, minimalist two-plane Material 3 tonal emblem (soft organic halo + crisp
/// circular tonal core + oversized vector glyph).
pub(in crate::ui) fn m3_tonal_emblem(
    icon: impl Into<Icon>,
    outer_size: f32,
    core_size: f32,
    icon_size: f32,
    tone: EmptyStateTone,
) -> Div {
    build_emblem(
        icon.into(),
        outer_size,
        core_size,
        icon_size,
        tone.palette(),
    )
}

fn build_emblem(
    icon: Icon,
    outer_size: f32,
    core_size: f32,
    icon_size: f32,
    palette: TonePalette,
) -> Div {
    let backdrop_bg: Background = linear_gradient(
        135.0,
        linear_color_stop(palette.backdrop_from, 0.0),
        linear_color_stop(palette.backdrop_to, 1.0),
    );
    let core_bg: Background = linear_gradient(
        145.0,
        linear_color_stop(palette.core_from, 0.0),
        linear_color_stop(palette.core_to, 1.0),
    );

    div()
        .relative()
        .size(space(outer_size))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .child(
            canvas(
                |_bounds, _window, _cx| (),
                move |bounds, _, window, _cx| {
                    if let Some(path) = build_m3_clover_path(bounds) {
                        window.paint_path(path, backdrop_bg);
                    }
                },
            )
            .size(space(outer_size)),
        )
        .child(
            div()
                .absolute()
                .size(space(core_size))
                .rounded_full()
                .bg(core_bg)
                .flex()
                .items_center()
                .justify_center()
                .text_color(palette.core_icon)
                .child(icon.size(space(icon_size))),
        )
}

fn split_title_and_description(copy: SharedString) -> (SharedString, Option<SharedString>) {
    let raw = copy.as_ref().trim();
    if let Some((first, rest)) = raw.split_once('\n') {
        let title = first.trim();
        let desc = rest.trim();
        if !title.is_empty() && !desc.is_empty() {
            return (
                SharedString::from(title.to_string()),
                Some(SharedString::from(desc.to_string())),
            );
        }
    }
    (copy, None)
}

/// Material 3 Filled Tonal pill button for empty-state primary/recovery actions.
pub(in crate::ui) fn empty_state_tonal_action(
    id: impl Into<ElementId>,
    icon: Option<AppIcon>,
    label: impl Into<SharedString>,
    cx: &App,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    let label = label.into();
    let colors = ShellColors::current();
    let background = colors.selected;
    let foreground = colors.on_selected;

    Button::new(id)
        .ghost()
        .h(space(38.0))
        .px(space(20.0))
        .rounded_full()
        .custom(
            ButtonCustomVariant::new(cx)
                .color(background)
                .foreground(foreground)
                .hover(background.blend(foreground.opacity(0.08)))
                .active(background.blend(foreground.opacity(0.14))),
        )
        .bg(background)
        .text_color(foreground)
        .child(
            h_flex()
                .items_center()
                .gap(space(8.0))
                .when_some(icon, |row, icon| {
                    row.child(Icon::new(icon).size(space(16.0)))
                })
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Heading.scaled())
                        .font_weight(FontWeight::MEDIUM)
                        .child(label),
                ),
        )
        .on_click(move |_, window, cx| on_click(window, cx))
}

pub(in crate::ui) fn scrollable_empty_page(content: Div) -> Div {
    // Scrollable gives the content an automatic height and a viewport-sized
    // minimum: short content centers, while oversized content grows downward
    // instead of placing its first half above the reachable scroll range.
    div()
        .size_full()
        .min_w_0()
        .min_h_0()
        .child(content.min_h_0().px_5().overflow_y_scrollbar())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        Bounds, Context, Render, ScrollDelta, TestAppContext, Window, WindowBounds, WindowOptions,
        point, px, size,
    };
    use std::cell::Cell;
    use std::rc::Rc;

    fn compact_empty_state(icon: AppIcon, copy: impl Into<SharedString>, min_height: f32) -> Div {
        EmptyState::new(icon, copy)
            .size(EmptyStateSize::Compact)
            .min_height(min_height)
            .build()
    }

    fn empty_page(icon: AppIcon, copy: impl Into<SharedString>) -> Div {
        EmptyState::new(icon, copy).build_page()
    }

    struct EmptyPageHarness {
        base_font: f32,
        compact: bool,
    }

    impl Render for EmptyPageHarness {
        fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            window.set_rem_size(px(self.base_font));
            let content = if self.compact {
                compact_empty_state(AppIcon::FolderOpen, "No favorites", 0.0)
            } else {
                empty_page(
                    AppIcon::Notebook,
                    "No snippets\nCreate a snippet to get started.",
                )
            };
            v_flex()
                .size_full()
                .child(div().h(space(88.0)).flex_shrink_0())
                .child(
                    div()
                        .id("empty-body")
                        .test_support()
                        .flex_1()
                        .min_h_0()
                        .child(content),
                )
        }
    }

    #[gpui_kit::test]
    fn empty_page_content_tracks_short_viewports_zoom_and_display_scale(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for base_font in [14.0, 18.0, 22.0] {
            for (width, body_height) in [(800.0, 600.0), (332.0, 233.0)] {
                let scale = base_font / 14.0;
                let (handle, _) = cx.update(|cx| {
                    gpui_kit::open_window(
                        WindowOptions {
                            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                                Default::default(),
                                size(px(width * scale), px((88.0 + body_height) * scale)),
                            ))),
                            ..Default::default()
                        },
                        cx,
                        |_, cx| {
                            cx.new(|_| EmptyPageHarness {
                                base_font,
                                compact: false,
                            })
                        },
                    )
                    .expect("open empty page geometry window")
                });
                cx.update_window(handle, |_, window, cx| {
                    for display_scale in [1.0, 1.25, 1.5, 2.0] {
                        window.set_scale_factor(display_scale);
                        window.render_frame(cx);
                        let body = window.find("empty-body").bounds();
                        let content = window.find("empty-state-content").bounds();
                        let tolerance = 1.0 / display_scale;
                        assert!(f32::from(content.center().x - body.center().x).abs() <= tolerance);
                        if content.size.height + window.rem_size() * 2.0 <= body.size.height {
                            assert!(
                                f32::from(content.center().y - body.center().y).abs() <= tolerance
                            );
                            assert!(content.top() >= body.top());
                            assert!(content.bottom() <= body.bottom());
                        } else {
                            // Larger copy can wrap past the short viewport.
                            // It must stay top-reachable and scroll to its end.
                            assert!(content.top() >= body.top());
                            window.scroll(
                                "empty-body",
                                ScrollDelta::Pixels(point(px(0.0), px(-1000.0))),
                                cx,
                            );
                            assert!(
                                window.find("empty-state-content").bounds().bottom()
                                    <= body.bottom()
                            );
                            window.scroll(
                                "empty-body",
                                ScrollDelta::Pixels(point(px(0.0), px(1000.0))),
                                cx,
                            );
                            assert!(
                                (window.find("empty-state-content").bounds().top() - content.top())
                                    .as_f32()
                                    .abs()
                                    <= tolerance
                            );
                        }
                        assert!(content.left() >= body.left());
                        assert!(content.right() <= body.right());
                    }
                })
                .unwrap();
            }
        }
    }

    #[gpui_kit::test]
    fn oversized_empty_page_keeps_its_first_and_last_content_reachable(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (handle, _) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        Default::default(),
                        size(px(332.0), px(260.0)),
                    ))),
                    ..Default::default()
                },
                cx,
                |_, cx| {
                    cx.new(|_| EmptyPageHarness {
                        base_font: 22.0,
                        compact: false,
                    })
                },
            )
            .expect("open oversized empty-page window")
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let body = window.find("empty-body").bounds();
            let before = window.find("empty-state-content").bounds();
            assert!(before.size.height > body.size.height);
            assert!(before.top() >= body.top());
            window.scroll(
                "empty-body",
                ScrollDelta::Pixels(point(px(0.0), px(-1000.0))),
                cx,
            );
            let after = window.find("empty-state-content").bounds();
            assert!(after.top() < before.top());
            assert!(after.bottom() <= body.bottom());
            window.scroll(
                "empty-body",
                ScrollDelta::Pixels(point(px(0.0), px(1000.0))),
                cx,
            );
            let restored = window.find("empty-state-content").bounds();
            assert!((restored.top() - before.top()).as_f32().abs() <= 1.0);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn compact_and_page_empty_states_keep_their_local_center(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for compact in [true, false] {
            let (handle, _) = cx.update(|cx| {
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            Default::default(),
                            size(px(332.0), px(if compact { 208.0 } else { 420.0 })),
                        ))),
                        ..Default::default()
                    },
                    cx,
                    |_, cx| {
                        cx.new(|_| EmptyPageHarness {
                            base_font: 14.0,
                            compact,
                        })
                    },
                )
                .expect("open local empty state window")
            });
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                let body = window.find("empty-body").bounds();
                let content = window.find("empty-state-content").bounds();
                assert!(f32::from(content.center().x - body.center().x).abs() <= 1.0);
                assert!(f32::from(content.center().y - body.center().y).abs() <= 1.0);
                assert!(content.top() >= body.top());
                assert!(content.bottom() <= body.bottom());
            })
            .unwrap();
        }
    }

    struct ExpressiveActionHarness {
        clicks: Rc<Cell<usize>>,
    }

    impl Render for ExpressiveActionHarness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let clicks = self.clicks.clone();
            div().id("expressive-body").size_full().child(
                EmptyState::new(AppIcon::Filter, "No matching hosts")
                    .description("Clear the group filter to see all saved hosts.")
                    .action(empty_state_tonal_action(
                        "empty-state-reset-action",
                        Some(AppIcon::Rotate),
                        "Reset filter",
                        cx,
                        move |_, _| {
                            clicks.set(clicks.get() + 1);
                        },
                    ))
                    .build_page(),
            )
        }
    }

    #[gpui_kit::test]
    fn expressive_empty_state_renders_and_dispatches_tonal_action(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let clicks = Rc::new(Cell::new(0));
        let clicks_for_view = clicks.clone();
        let (handle, _) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        Default::default(),
                        size(px(480.0), px(360.0)),
                    ))),
                    ..Default::default()
                },
                cx,
                |_, cx| {
                    cx.new(|_| ExpressiveActionHarness {
                        clicks: clicks_for_view,
                    })
                },
            )
            .expect("open expressive empty state window")
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("empty-state-content").visible());
            assert!(window.find("empty-state-reset-action").visible());
            window.click("empty-state-reset-action", cx);
        })
        .unwrap();
        assert_eq!(clicks.get(), 1);
    }
}
