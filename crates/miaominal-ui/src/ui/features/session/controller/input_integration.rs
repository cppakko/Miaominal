//! The fixture injects transport channels and workspace identities; rendering,
//! canvas input registration and key/pointer handlers are the production paths.
use super::{SessionController, tests::session_payload};
use crate::ui::contracts::{PaneId, TabId, TerminalWorkspaceRequest};
use crate::ui::features::session::input::{
    PaneViewState, TerminalDragState, TerminalInputHost, TerminalKeyboardState,
    TerminalMouseGesture, TerminalPanePresentation, bind_terminal_input_handlers,
};
use crate::ui::features::session::view::canvas::{
    TerminalCanvasHost, TerminalCanvasPrepaint, render_terminal_canvas_for_pane,
};
use gpui_kit::test::{TestSupportExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, Bounds, Context, Entity, InputEvent as _, IntoElement, KeyDownEvent,
    KeyUpEvent, Keystroke, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Render, TestAppContext, Window, WindowBounds, WindowOptions, div, point, prelude::*, px, size,
};
use miaominal_ssh::{SessionChannels, SessionCommand, session_channels};
use std::time::Instant;

const FIRST: TabId = TabId::new(401);
const SECOND: TabId = TabId::new(402);

struct TerminalInputFixture {
    owner: Entity<SessionController>,
    pane: PaneViewState,
    active: Option<TabId>,
    drag: TerminalDragState,
    requested: Vec<TerminalWorkspaceRequest>,
    parked_input: Vec<(TerminalKeyboardState, Option<TerminalMouseGesture>)>,
}

impl Render for TerminalInputFixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let region = div()
            .id("terminal-input")
            .test_support()
            .track_focus(&self.pane.terminal_focus)
            .size_full();
        bind_terminal_input_handlers(region, 18., cx).child(render_terminal_canvas_for_pane(
            None,
            8.,
            18.,
            cx.weak_entity(),
            PaneId(1),
            false,
        ))
    }
}

impl TerminalInputHost for TerminalInputFixture {
    fn terminal_owner(&self) -> Entity<SessionController> {
        self.owner.clone()
    }
    fn active_terminal_tab_id(&self) -> Option<TabId> {
        self.active
    }
    fn active_terminal_pane_id(&self) -> PaneId {
        PaneId(1)
    }
    fn active_terminal_pane(&self) -> &PaneViewState {
        &self.pane
    }
    fn active_terminal_pane_mut(&mut self) -> &mut PaneViewState {
        &mut self.pane
    }
    fn terminal_pane_ids(&self) -> Vec<PaneId> {
        vec![PaneId(1)]
    }
    fn terminal_pane_presentation(&self, id: PaneId) -> Option<TerminalPanePresentation> {
        (id == PaneId(1)).then_some(TerminalPanePresentation {
            tab_id: self.active,
            bounds: self.pane.terminal_bounds,
            cell_width: self.pane.terminal_cell_width,
            line_height: self.pane.terminal_line_height,
            pointer_position: self.pane.terminal_pointer_position,
            dragging_scrollbar: self.pane.terminal_scrollbar_drag.is_some(),
            scrollbar_interaction: self.pane.terminal_scrollbar_last_interaction_at,
        })
    }
    fn for_each_parked_terminal_input(
        &mut self,
        mut visit: impl FnMut(&mut TerminalKeyboardState, &mut Option<TerminalMouseGesture>),
    ) {
        for (keyboard, gesture) in &mut self.parked_input {
            visit(keyboard, gesture);
        }
    }
    fn terminal_drag(&self) -> &TerminalDragState {
        &self.drag
    }
    fn terminal_drag_mut(&mut self) -> &mut TerminalDragState {
        &mut self.drag
    }
    fn set_terminal_scrollbar_interaction(&mut self, id: PaneId, at: Instant) -> bool {
        if id != PaneId(1) {
            return false;
        }
        self.pane.terminal_scrollbar_last_interaction_at = Some(at);
        true
    }
    fn dispatch_terminal_workspace_request(
        &mut self,
        request: TerminalWorkspaceRequest,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.requested.push(request);
        cx.notify();
    }
}

impl TerminalCanvasHost for TerminalInputFixture {
    fn prepare_terminal_canvas_prepaint(
        &mut self,
        _: PaneId,
        bounds: Bounds<Pixels>,
        cell_width: f32,
        line_height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<TerminalCanvasPrepaint> {
        self.pane.terminal_bounds = Some(bounds);
        self.pane.terminal_cell_width = cell_width;
        self.pane.terminal_line_height = line_height;
        let id = self.active?;
        let snapshot = self
            .owner
            .read(cx)
            .terminal_snapshot(id, self.pane.terminal_focus.is_focused(window))?;
        Some(TerminalCanvasPrepaint::new(
            snapshot,
            self.pane.terminal_focus.clone(),
            self.owner.downgrade(),
            id,
            None,
            None,
        ))
    }
    fn handle_terminal_outside_mouse_move(
        &mut self,
        _: PaneId,
        _: &MouseMoveEvent,
        _: &mut Context<Self>,
    ) {
    }
    fn handle_terminal_outside_mouse_up(
        &mut self,
        _: PaneId,
        _: &MouseUpEvent,
        _: &mut Context<Self>,
    ) {
    }
}

fn open_input_fixture(
    cx: &mut TestAppContext,
    modes: &str,
) -> (
    AnyWindowHandle,
    Entity<TerminalInputFixture>,
    SessionChannels,
    SessionChannels,
) {
    cx.update(gpui_kit::init);
    let first = session_channels();
    let second = session_channels();
    let senders = [
        first.connection.commands.clone(),
        second.connection.commands.clone(),
    ];
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(640.), px(480.)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    let owner = cx.new(|_| SessionController::new_for_test());
                    for (id, commands) in [FIRST, SECOND].into_iter().zip(senders) {
                        let mut payload = session_payload("fixture");
                        payload.commands = Some(commands);
                        payload.terminal.push_text(modes);
                        owner.read(cx).insert_tab(id, payload);
                    }
                    let pane = PaneViewState::new(cx.focus_handle());
                    pane.terminal_focus.focus(window, cx);
                    TerminalInputFixture {
                        owner,
                        pane,
                        active: Some(FIRST),
                        drag: TerminalDragState::default(),
                        requested: Vec::new(),
                        parked_input: Vec::new(),
                    }
                })
            },
        )
        .expect("open terminal input fixture")
    });
    (window, view, first, second)
}

fn take_sent(channels: &mut SessionChannels) -> Vec<Vec<u8>> {
    let mut sent = Vec::new();
    while let Ok(command) = channels.commands.try_recv() {
        if let SessionCommand::Send(bytes) = command {
            sent.push(bytes);
        }
    }
    sent
}

#[gpui_kit::test]
fn production_keyboard_dispatch_binds_held_release_to_the_original_terminal(
    cx: &mut TestAppContext,
) {
    let (handle, view, mut first, mut second) = open_input_fixture(cx, "");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("terminal-input").focused(), Some(true));
        let key = Keystroke::parse("enter").unwrap();
        window.dispatch_event(
            KeyDownEvent {
                keystroke: key.clone(),
                is_held: false,
                prefer_character_input: false,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert_eq!(take_sent(&mut first).len(), 1);
        assert!(take_sent(&mut second).is_empty());
        assert_eq!(
            view.read(cx).pane.terminal_keyboard.pressed_target("enter"),
            Some(FIRST)
        );
        view.update(cx, |view, cx| {
            view.parked_input.push((
                std::mem::take(&mut view.pane.terminal_keyboard),
                view.pane.terminal_mouse_gesture.take(),
            ));
            view.active = Some(SECOND);
            cx.notify();
        });
        window.render_frame(cx);
        window.dispatch_event(
            KeyDownEvent {
                keystroke: key.clone(),
                is_held: true,
                prefer_character_input: false,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert_eq!(take_sent(&mut first).len(), 1);
        assert!(take_sent(&mut second).is_empty());
        window.dispatch_event(
            KeyUpEvent {
                keystroke: key.clone(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        let release = take_sent(&mut first);
        // The production terminal currently uses legacy keyboard mode, which
        // sends no release bytes. The held target must still be retired here.
        assert!(release.is_empty());
        assert!(
            view.read(cx).parked_input[0]
                .0
                .pressed_target("enter")
                .is_none()
        );
        assert!(take_sent(&mut second).is_empty());
        window.press("enter", cx);
        assert_eq!(take_sent(&mut second).len(), 1);
        assert!(take_sent(&mut first).is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn production_pointer_release_rejects_replaced_terminal_and_accepts_a_fresh_press(
    cx: &mut TestAppContext,
) {
    let (handle, view, mut first, mut second) = open_input_fixture(cx, "\x1b[?1000h\x1b[?1006h");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let position = window.find("terminal-input").bounds().origin + point(px(20.), px(20.));
        window.dispatch_event(
            MouseMoveEvent {
                position,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            MouseDownEvent {
                position,
                button: MouseButton::Left,
                click_count: 1,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert_eq!(take_sent(&mut first).len(), 1);
        view.update(cx, |view, cx| {
            view.active = Some(SECOND);
            cx.notify();
        });
        window.render_frame(cx);
        window.dispatch_event(
            MouseUpEvent {
                position,
                button: MouseButton::Left,
                click_count: 1,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert!(take_sent(&mut second).is_empty());
        assert!(!view.read(cx).pane.terminal_mouse_reporting.is_active());
        window.click_at("terminal-input", point(px(20.), px(20.)), cx);
        let fresh = take_sent(&mut second);
        assert_eq!(fresh.len(), 2);
        assert!(fresh[0].ends_with(b"M"));
        assert!(fresh[1].ends_with(b"m"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn production_pointer_selection_release_does_not_mutate_a_replacement_terminal(
    cx: &mut TestAppContext,
) {
    let (handle, view, mut first, mut second) = open_input_fixture(cx, "a line to select");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let position = window.find("terminal-input").bounds().origin + point(px(12.), px(8.));
        window.dispatch_event(
            MouseDownEvent {
                position,
                button: MouseButton::Left,
                click_count: 1,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert!(view.read(cx).pane.terminal_mouse_gesture.is_some());
        view.update(cx, |view, cx| {
            view.active = Some(SECOND);
            cx.notify();
        });
        window.render_frame(cx);
        window.dispatch_event(
            MouseUpEvent {
                position: position + point(px(80.), px(0.)),
                button: MouseButton::Left,
                click_count: 1,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert!(view.read(cx).pane.terminal_mouse_gesture.is_none());
        assert!(
            !view
                .read(cx)
                .owner
                .read(cx)
                .tab(SECOND)
                .unwrap()
                .terminal
                .has_selection()
        );
        assert!(take_sent(&mut first).is_empty());
        assert!(take_sent(&mut second).is_empty());
    })
    .unwrap();
}
