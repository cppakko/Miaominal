use crate::ui::assets::AppIcon;
use crate::ui::components::shell_tokens::{ShellColors, space};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Icon, v_flex};
use gpui_kit::{Div, SharedString, TestSupportExt as _, div, prelude::*};

#[derive(Clone, Copy)]
struct EmptyStateLayout {
    min_height: f32,
    icon_size: f32,
    max_width: f32,
    gap: f32,
    compact: bool,
}

impl EmptyStateLayout {
    fn page() -> Self {
        Self {
            min_height: 240.0,
            icon_size: 128.0,
            max_width: 380.0,
            gap: 12.0,
            compact: false,
        }
    }

    fn compact(min_height: f32) -> Self {
        Self {
            min_height,
            icon_size: 48.0,
            max_width: 260.0,
            gap: 8.0,
            compact: true,
        }
    }
}

pub(in crate::ui) fn empty_state(icon: AppIcon, copy: impl Into<SharedString>) -> Div {
    empty_state_with_layout(icon, copy.into(), EmptyStateLayout::page())
}

pub(in crate::ui) fn compact_empty_state(
    icon: AppIcon,
    copy: impl Into<SharedString>,
    min_height: f32,
) -> Div {
    empty_state_with_layout(icon, copy.into(), EmptyStateLayout::compact(min_height))
}

fn empty_state_with_layout(icon: AppIcon, copy: SharedString, layout: EmptyStateLayout) -> Div {
    let colors = ShellColors::current();
    let text_size = if layout.compact {
        miaominal_settings::FontSize::Body.scaled()
    } else {
        miaominal_settings::FontSize::Title.scaled()
    };
    let line_height = if layout.compact { 18.0 } else { 24.0 };

    let container = div()
        .size_full()
        .min_w_0()
        .min_h(space(layout.min_height))
        .flex()
        .items_center()
        .justify_center();

    let container = if layout.compact {
        container.px_3().py_2()
    } else {
        container.px_4().py_6()
    };

    container.child(
        v_flex()
            .id("empty-state-content")
            .test_support()
            .w_full()
            .min_w_0()
            .max_w(space(layout.max_width))
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .gap(space(layout.gap))
            .child(
                div()
                    .size(space(layout.icon_size))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(colors.muted)
                    .child(Icon::new(icon).size(space(layout.icon_size))),
            )
            .child(
                div()
                    .w_full()
                    .text_size(text_size)
                    .line_height(miaominal_settings::scaled_line_height(line_height))
                    .text_center()
                    .text_color(colors.primary)
                    .child(copy),
            ),
    )
}

pub(in crate::ui) fn empty_page(icon: AppIcon, copy: impl Into<SharedString>) -> Div {
    scrollable_empty_page(empty_state(icon, copy))
}

fn scrollable_empty_page(content: Div) -> Div {
    // Scrollable gives the content an automatic height and a viewport-sized
    // minimum: short content centers, while oversized content grows downward
    // instead of placing its first half above the reachable scroll range.
    div()
        .size_full()
        .min_w_0()
        .min_h_0()
        .child(content.min_h_0().px_5().py_4().overflow_y_scrollbar())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        Bounds, Context, Render, ScrollDelta, TestAppContext, Window, WindowBounds, WindowOptions,
        point, px, size,
    };
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
}
