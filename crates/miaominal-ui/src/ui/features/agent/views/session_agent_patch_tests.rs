use super::*;
use crate::ui::features::agent::conversation_view::SessionAgentMessageView;
use gpui_kit::TestAppContext;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt as _;

const PATCH: &str = "--- a/config.toml\n+++ b/config.toml\n@@ -1,2 +1,2 @@\n enabled = true\n-port = 22\n+port = 2222\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+hello\n";

fn patch_message(arguments: String) -> SessionAgentMessage {
    SessionAgentMessage::tool_call(SessionAgentToolCall {
        id: "patch-fixture".into(),
        name: "apply_patch".into(),
        arguments,
        summary: "Update configuration".into(),
        status: SessionAgentToolStatus::WaitingForConfirmation,
        requires_confirmation: true,
        confirmation_note: None,
        expanded: true,
    })
}

struct PatchFixture {
    message: Entity<SessionAgentMessageView>,
    width: f32,
}

impl Render for PatchFixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let projection = self.message.read(cx);
        div().w(px(self.width)).child(render_apply_patch_tool_body(
            projection.message().tool_call.as_ref().unwrap(),
            ToolTerminalColors {
                surface: roles.surface,
                surface_container_lowest: roles.surface_container_lowest,
                on_surface: roles.on_surface,
                error: roles.error,
                text_muted: roles.on_surface_variant,
            },
            projection.patch_diff(),
        ))
    }
}

#[gpui_kit::test]
fn patch_review_renders_files_resizes_and_keeps_folds(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let mut fixture = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| PatchFixture {
            message: cx.new(|cx| {
                SessionAgentMessageView::new(
                    patch_message(serde_json::json!({"patch": PATCH}).to_string()),
                    cx,
                )
            }),
            width: 282.0,
        });
        fixture = Some(view.clone());
        Root::new(view, window, cx)
    });
    let fixture = fixture.unwrap();
    let state = cx.update(|window, cx| {
        window.render_frame(cx);
        let state = fixture.read(cx).message.read(cx).patch_diff().unwrap().0;
        assert_eq!(state.read(cx).files().len(), 2);
        assert_eq!(state.read(cx).files()[0].additions(), 1);
        assert_eq!(state.read(cx).files()[0].deletions(), 1);
        assert_eq!(state.read(cx).mode(), DiffMode::Unified);
        let bounds = window
            .find("session-agent-patch-review-patch-fixture")
            .bounds();
        assert!(bounds.size.height > px(0.0));
        assert!(bounds.size.width <= px(282.0));
        window
            .within(("diff-file", 0usize))
            .click("collapse-file", cx);
        assert!(state.read(cx).is_file_collapsed("config.toml"));
        fixture.read(cx).message.clone().update(cx, |view, cx| {
            view.update_message(
                |message| {
                    let tool = message.tool_call.as_mut().unwrap();
                    tool.status = SessionAgentToolStatus::Completed;
                    tool.expanded = false;
                },
                cx,
            );
            view.update_message(
                |message| message.tool_call.as_mut().unwrap().expanded = true,
                cx,
            );
            assert_eq!(view.patch_diff().unwrap().0.entity_id(), state.entity_id());
        });
        assert!(state.read(cx).is_file_collapsed("config.toml"));
        fixture.update(cx, |fixture, cx| {
            fixture.width = 900.0;
            cx.notify();
        });
        window.render_frame(cx);
        state
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert_eq!(state.read(cx).mode(), DiffMode::Split);
        fixture.update(cx, |fixture, cx| {
            fixture.width = 282.0;
            cx.notify();
        });
        window.render_frame(cx);
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert_eq!(state.read(cx).mode(), DiffMode::Unified);
        for scale in [1.0, 1.5, 2.0] {
            window.set_scale_factor(scale);
            window.render_frame(cx);
            assert!(
                window
                    .find("session-agent-patch-review-patch-fixture")
                    .bounds()
                    .size
                    .width
                    <= px(282.0)
            );
        }
    });
}

#[gpui_kit::test]
fn patch_review_falls_back_for_streaming_and_invalid_patches(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let message = cx.new(|cx| {
        SessionAgentMessageView::new(patch_message("{\"patch\":\"--- a/file".into()), cx)
    });
    assert!(message.read_with(cx, |view, _| view.patch_diff().is_none()));
    message.update(cx, |view, cx| {
        view.set_message_snapshot(
            patch_message(serde_json::json!({"patch": PATCH}).to_string()),
            cx,
        );
        assert!(view.patch_diff().is_some());
        view.set_message_snapshot(
            patch_message(serde_json::json!({"patch": "invalid patch"}).to_string()),
            cx,
        );
        assert!(view.patch_diff().is_none());
        assert!(
            view.message()
                .tool_call
                .as_ref()
                .unwrap()
                .arguments
                .contains("invalid patch")
        );
    });
}

#[gpui_kit::test]
fn completed_edit_file_uses_the_returned_diff(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let message = cx.new(|cx| {
        SessionAgentMessageView::new(
            SessionAgentMessage::tool_call(SessionAgentToolCall {
                id: "edit-fixture".into(),
                name: "edit_file".into(),
                arguments: serde_json::json!({
                    "path": "config.toml",
                    "old_string": "port = 22",
                    "new_string": "port = 2222"
                })
                .to_string(),
                summary: "Edit config.toml".into(),
                status: SessionAgentToolStatus::Completed,
                requires_confirmation: false,
                confirmation_note: Some(
                    "--- config.toml\n+++ config.toml\n@@ -1 +1 @@\n-port = 22\n+port = 2222\n"
                        .into(),
                ),
                expanded: true,
            }),
            cx,
        )
    });
    assert!(message.read_with(cx, |view, _| view.patch_diff().is_some()));
}

#[gpui_kit::test]
fn completed_write_file_uses_the_returned_diff(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let message = cx.new(|cx| {
        SessionAgentMessageView::new(
            SessionAgentMessage::tool_call(SessionAgentToolCall {
                id: "write-fixture".into(),
                name: "write_file".into(),
                arguments: serde_json::json!({
                    "path": "new.txt",
                    "content": "hello\n"
                })
                .to_string(),
                summary: "Write new.txt".into(),
                status: SessionAgentToolStatus::Completed,
                requires_confirmation: false,
                confirmation_note: Some(
                    "--- /dev/null\n+++ new.txt\n@@ -0,0 +1 @@\n+hello\n".into(),
                ),
                expanded: true,
            }),
            cx,
        )
    });
    assert!(message.read_with(cx, |view, _| view.patch_diff().is_some()));
}
