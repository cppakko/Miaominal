use super::super::{SessionController, tests::session_payload};
use crate::ui::components::forms::set_input_value;
use crate::ui::contracts::overlay::OverlaySnapshot;
use crate::ui::contracts::workspace::TabId;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, Bounds, Context, Entity, IntoElement, Render, TestAppContext, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, size,
};
use miaominal_core::forwarding::KbiPrompt;
use miaominal_ssh::{
    HostKeyDecision, HostKeyPrompt, KbiChallenge, SessionChannels, SessionCommand, session_channels,
};

const TAB: TabId = TabId::new(611);
struct AuthenticationFixture {
    owner: Entity<SessionController>,
    snapshot: OverlaySnapshot,
}
impl Render for AuthenticationFixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(self.snapshot.render(None, 700., cx))
    }
}

fn challenge() -> KbiChallenge {
    KbiChallenge {
        name: "Authentication".into(),
        instructions: String::new(),
        prompts: vec![KbiPrompt {
            prompt: "Code".into(),
            echo: false,
        }],
    }
}
fn host_prompt() -> HostKeyPrompt {
    HostKeyPrompt {
        host: "fixture.invalid".into(),
        port: 22,
        algorithm: "ed25519".into(),
        fingerprint: "SHA256:fixture".into(),
        previous_fingerprint: None,
    }
}

fn open_authentication_fixture(
    cx: &mut TestAppContext,
    keyboard: bool,
) -> (
    AnyWindowHandle,
    Entity<AuthenticationFixture>,
    SessionChannels,
) {
    cx.update(gpui_kit::init);
    let channels = session_channels();
    let commands = channels.connection.commands.clone();
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(800.), px(700.)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    let owner = cx.new(|_| SessionController::new_for_test());
                    let mut payload = session_payload("fixture");
                    payload.commands = Some(commands);
                    if keyboard {
                        payload.pending_keyboard_interactive = Some(challenge());
                        payload.pending_keyboard_interactive_request_id = 81;
                    } else {
                        payload.pending_host_key = Some(host_prompt());
                        payload.pending_host_key_request_id = 101;
                    }
                    owner.read(cx).insert_tab(TAB, payload);
                    owner.update(cx, |owner, cx| {
                        owner.sync_visible_authentication_inputs(
                            Some(TAB),
                            &[TAB],
                            false,
                            window,
                            cx,
                        )
                    });
                    let snapshot = owner
                        .read(cx)
                        .overlay_snapshots(owner.clone(), &[TAB], cx)
                        .remove(0);
                    AuthenticationFixture { owner, snapshot }
                })
            },
        )
        .expect("open authentication fixture")
    });
    (window, view, channels)
}

#[gpui_kit::test]
fn retained_keyboard_prompt_rejects_old_buttons_submit_and_dismiss_for_same_length_request(
    cx: &mut TestAppContext,
) {
    let (handle, view, mut channels) = open_authentication_fixture(cx, true);
    cx.update_window(handle, |_, window, cx| {
        let owner = view.read(cx).owner.clone();
        let old = view.read(cx).snapshot.clone();
        let old_input = owner.read(cx).keyboard_interactive_inputs().remove(0);
        set_input_value(&old_input, "old-secret", window, cx);
        owner.update(cx, |owner, cx| {
            let mut tab = owner.tab_mut(TAB).unwrap();
            tab.pending_keyboard_interactive_request_id = 82;
            tab.pending_keyboard_interactive = Some(challenge());
            drop(tab);
            owner.sync_visible_authentication_inputs(Some(TAB), &[TAB], false, window, cx);
        });
        let new_input = owner.read(cx).keyboard_interactive_inputs().remove(0);
        assert_ne!(old_input.entity_id(), new_input.entity_id());
        assert_eq!(old_input.read(cx).value().as_ref(), "old-secret");
        assert!(new_input.read(cx).value().is_empty());
        window.render_frame(cx);
        window.click("keyboard-interactive-submit", cx);
        assert!(old.submit(window, cx));
        old.dismiss(window, cx);
        assert!(
            owner
                .read(cx)
                .is_keyboard_interactive_request_current(TAB, 82)
        );
        assert!(channels.commands.try_recv().is_err());
        set_input_value(&new_input, "new-code", window, cx);
        let fresh = owner
            .read(cx)
            .overlay_snapshots(owner.clone(), &[TAB], cx)
            .remove(0);
        view.update(cx, |view, cx| {
            view.snapshot = fresh;
            cx.notify();
        });
        window.render_frame(cx);
        window.click("keyboard-interactive-submit", cx);
        match channels
            .commands
            .try_recv()
            .expect("current request should receive answers")
        {
            SessionCommand::KeyboardInteractiveResponse {
                request_id,
                answers,
            } => {
                assert_eq!(request_id, 82);
                assert_eq!(answers, vec!["new-code".to_owned()]);
            }
            command => panic!("unexpected authentication command: {command:?}"),
        }
        assert!(
            !owner
                .read(cx)
                .is_keyboard_interactive_request_current(TAB, 82)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn retained_host_key_button_and_dismiss_cannot_resolve_a_new_request(cx: &mut TestAppContext) {
    let (handle, view, mut channels) = open_authentication_fixture(cx, false);
    cx.update_window(handle, |_, window, cx| {
        let owner = view.read(cx).owner.clone();
        let old = view.read(cx).snapshot.clone();
        owner.update(cx, |owner, _| {
            let mut tab = owner.tab_mut(TAB).unwrap();
            tab.pending_host_key_request_id = 102;
            tab.pending_host_key = Some(host_prompt());
        });
        window.render_frame(cx);
        window.click("host-key-once", cx);
        old.dismiss(window, cx);
        assert!(owner.read(cx).is_host_key_request_current(TAB, 102));
        assert!(channels.commands.try_recv().is_err());
        let fresh = owner
            .read(cx)
            .overlay_snapshots(owner.clone(), &[TAB], cx)
            .remove(0);
        view.update(cx, |view, cx| {
            view.snapshot = fresh;
            cx.notify();
        });
        window.render_frame(cx);
        window.click("host-key-once", cx);
        match channels
            .commands
            .try_recv()
            .expect("current host-key request should receive a decision")
        {
            SessionCommand::HostKeyDecision {
                request_id,
                decision: HostKeyDecision::AcceptOnce,
            } => assert_eq!(request_id, 102),
            command => panic!("unexpected authentication command: {command:?}"),
        }
        assert!(!owner.read(cx).is_host_key_request_current(TAB, 102));
    })
    .unwrap();
}
