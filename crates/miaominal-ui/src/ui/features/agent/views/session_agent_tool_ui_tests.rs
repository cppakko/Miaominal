use super::*;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Bounds, TestAppContext};

struct ToolBlockFixture {
    text: String,
    width: f32,
}

impl Render for ToolBlockFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let roles = miaominal_settings::current_theme().material.roles;
        div()
            .size_full()
            .child(v_flex().id("tool-block-fixture").w(px(self.width)).child(
                render_tool_terminal_block(
                    "fixture",
                    "Output".into(),
                    None,
                    self.text.clone(),
                    ToolTerminalColors {
                        surface: roles.surface,
                        surface_container_lowest: roles.surface_container_lowest,
                        on_surface: roles.on_surface,
                        error: roles.error,
                        text_muted: roles.on_surface_variant,
                    },
                    false,
                ),
            ))
    }
}

fn viewport(window: &Window) -> Bounds<Pixels> {
    window
        .find("session-agent-tool-block-fixture-Output-block")
        .bounds()
}

#[gpui_kit::test]
fn tool_output_grows_then_scrolls_without_clipping_its_first_line(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let mut fixture = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|_| ToolBlockFixture {
            text: "exit_code: 0\nready".into(),
            width: 282.0,
        });
        fixture = Some(view.clone());
        Root::new(view, window, cx)
    });
    let fixture = fixture.unwrap();
    cx.update(|window, cx| {
        window.render_frame(cx);
        let short = viewport(window);
        assert!(
            short.size.height > px(20.0),
            "short output must remain visible: {short:?}"
        );
        assert!(
            short.size.height < px(100.0),
            "short output must not fill its cap: {short:?}"
        );
        assert!(
            short.top() >= px(0.0),
            "the first line must not be above the viewport"
        );

        fixture.update(cx, |fixture, cx| {
            fixture.text = (0..5).map(|i| format!("line {i}\n")).collect();
            cx.notify();
        });
        window.render_frame(cx);
        let medium = viewport(window);
        assert!(
            medium.size.height > short.size.height,
            "more lines should grow naturally"
        );

        fixture.update(cx, |fixture, cx| {
            fixture.text = (0..100).map(|i| format!("line {i}: output\n")).collect();
            cx.notify();
        });
        window.render_frame(cx);
        let long = viewport(window);
        // The label lives outside the scrolling content viewport.
        let cap = window.rem_size() * (TOOL_BLOCK_MAX_HEIGHT_REM + 3.0);
        assert!(long.size.height > medium.size.height);
        assert!(
            long.size.height <= cap + px(1.0),
            "large output must scroll, not expand: {long:?} / {cap:?}"
        );
        assert!(
            long.size.width <= px(282.0),
            "scroll area must fit the narrow card"
        );

        // Replacing a long result with a short one must shrink the same retained
        // element instead of keeping an old, fixed height.
        fixture.update(cx, |fixture, cx| {
            fixture.text = "done".into();
            cx.notify();
        });
        window.render_frame(cx);
        assert!(viewport(window).size.height < short.size.height);
    });
}

#[gpui_kit::test]
fn long_tool_text_reflows_with_a_narrow_panel(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let mut fixture = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|_| ToolBlockFixture {
            text: "systemctl status nginx --no-pager --full /var/log/nginx/access.log".into(),
            width: 282.0,
        });
        fixture = Some(view.clone());
        Root::new(view, window, cx)
    });
    let fixture = fixture.unwrap();
    cx.update(|window, cx| {
        window.render_frame(cx);
        let narrow = viewport(window);
        fixture.update(cx, |fixture, cx| {
            fixture.width = 600.0;
            cx.notify();
        });
        window.render_frame(cx);
        let wide = viewport(window);
        assert!(
            narrow.size.height > wide.size.height,
            "text must wrap rather than disappear horizontally: narrow={narrow:?} wide={wide:?}"
        );
        assert!(wide.size.height > px(0.0));
        for scale in [1.0, 1.5, 2.0] {
            window.set_scale_factor(scale);
            window.render_frame(cx);
            assert!(viewport(window).size.height > px(0.0));
        }
    });
}
