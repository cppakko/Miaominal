use super::super::workspace::PaneLayout;
use super::super::*;
use super::workspace_side_panel::{
    WorkspaceSidePanelDock, render_workspace_side_panel, workspace_side_panel_render_state,
};
use gpui_component::ElementExt as _;
use std::{cell::RefCell, rc::Rc};

const SESSION_SFTP_BOTTOM_PROGRESS_GAP: f32 = 8.0;
const SESSION_SFTP_PROGRESS_MIN_HEIGHT: f32 = 220.0;
const SESSION_WORKSPACE_MIN_HEIGHT: f32 = 240.0;
const SESSION_SFTP_MIN_SPLIT_FLEX: f32 = 0.05;

#[derive(Clone, Copy)]
struct SessionSftpProgressResizeMarker;

impl Render for SessionSftpProgressResizeMarker {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size(px(1.0))
    }
}

fn session_sftp_usable_height(container_height: Pixels) -> f32 {
    (container_height.as_f32() - SESSION_SFTP_BOTTOM_PROGRESS_GAP).max(1.0)
}

fn clamp_session_sftp_progress_flex(container_height: Pixels, requested: f32) -> f32 {
    let available = session_sftp_usable_height(container_height);
    let min =
        (SESSION_SFTP_PROGRESS_MIN_HEIGHT / available).clamp(SESSION_SFTP_MIN_SPLIT_FLEX, 0.95);
    let max = (1.0
        - (SESSION_WORKSPACE_MIN_HEIGHT / available).clamp(SESSION_SFTP_MIN_SPLIT_FLEX, 0.95))
    .clamp(0.05, 0.95);

    if max <= min {
        return 0.5;
    }

    requested.clamp(min, max)
}

fn resized_session_sftp_progress_flex(
    container_height: Pixels,
    initial_flex: f32,
    pointer_delta: f32,
) -> f32 {
    let requested = initial_flex - pointer_delta / session_sftp_usable_height(container_height);
    clamp_session_sftp_progress_flex(container_height, requested)
}

fn render_session_sftp_progress_resize_handle(
    is_dragging: bool,
    container_height: Rc<RefCell<Pixels>>,
    cx: &mut Context<AppView>,
) -> gpui::AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    div()
        .id("session-sftp-progress-resize-handle")
        .size_full()
        .cursor_row_resize()
        .occlude()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                let container_height = *container_height.borrow();
                let stored_flex = this.controllers.sftp.read(cx).session_progress_flex();
                let initial_flex = clamp_session_sftp_progress_flex(container_height, stored_flex);
                this.controllers
                    .sftp
                    .read(cx)
                    .set_session_progress_flex(initial_flex);
                this.controllers
                    .sftp
                    .read(cx)
                    .set_session_progress_drag(Some(SessionSftpProgressCenterDragState {
                        initial_pointer: f32::from(event.position.y),
                        initial_flex,
                        container_height,
                    }));
                cx.stop_propagation();
                cx.notify();
            }),
        )
        .hover(move |this| {
            if is_dragging {
                this.bg(color_with_alpha(roles.primary, 0x22))
            } else {
                this.cursor_row_resize()
            }
        })
        .on_drag(
            SessionSftpProgressResizeMarker,
            |marker, _offset, _window, cx| cx.new(|_| *marker),
        )
        .into_any_element()
}

pub(in crate::ui::shell) fn render_workspace_surface(
    app: &mut AppView,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> gpui::AnyElement {
    app.advance_pane_split_animation(window, cx);
    let workspace_height = Rc::new(RefCell::new(px(0.0)));

    // Snapshot the layout so the renderer can borrow the app mutably while walking it.
    let layout = std::mem::replace(
        &mut app.workspace.workspace.pane_layout,
        PaneLayout::Leaf(app.workspace.workspace.active_pane_id),
    );
    let valid_source_ids = app.pane_drop_source_ids();
    let workspace_body = app.render_pane_layout(&layout, &[], &valid_source_ids, window, cx);
    app.workspace.workspace.pane_layout = layout;

    let session_index = app.active_terminal_session_index(cx);

    let session_panel = app.controllers.session.read(cx);
    let desired_side_panel_visible = session_panel.side_panel_open() && session_index.is_some();
    let (mut side_panel_visible, mut side_panel_transition) =
        session_panel.side_panel_transition_state();
    let show_side_panel = workspace_side_panel_render_state(
        desired_side_panel_visible,
        &mut side_panel_visible,
        &mut side_panel_transition,
        window,
    );
    session_panel.set_side_panel_transition_state(side_panel_visible, side_panel_transition);

    let (desired_agent_panel_visible, mut agent_panel_visible, mut agent_panel_transition) = {
        let agent_controller = app.controllers.agent.read(cx);
        let (visible, transition) = agent_controller.panel_transition_state();
        (
            agent_controller.panel_open() && session_index.is_some(),
            visible,
            transition,
        )
    };
    if !desired_agent_panel_visible {
        app.controllers.agent.update(cx, |controller, cx| {
            controller.finish_text_drag(cx);
        });
    }
    let show_agent_panel = workspace_side_panel_render_state(
        desired_agent_panel_visible,
        &mut agent_panel_visible,
        &mut agent_panel_transition,
        window,
    );
    app.controllers
        .agent
        .read(cx)
        .set_panel_transition_state(agent_panel_visible, agent_panel_transition);
    if !desired_agent_panel_visible && show_agent_panel.is_none() {
        app.controllers.agent.update(cx, |controller, cx| {
            controller.finish_text_drag(cx);
            controller.release_conversation_view(cx);
        });
    }
    let agent_panel_width = super::session_agent_panel::clamp_session_agent_panel_width(
        app.controllers.agent.read(cx).panel_width(),
    );
    app.controllers
        .agent
        .read(cx)
        .set_panel_width(agent_panel_width);
    let side_panel = show_side_panel.and_then(|visibility| {
        let session_tab_id = session_index.and_then(|index| app.workspace.tabs.id_at(index))?;
        let session = app.session_tab(session_tab_id, cx)?;
        Some(render_workspace_side_panel(
            super::workspace_side_panel::render_session_workspace_side_panel(
                app,
                app.controllers.session.clone(),
                session_tab_id,
                &session,
                cx,
            ),
            SESSION_MONITOR_PANEL_WIDTH,
            visibility,
            WorkspaceSidePanelDock::Left,
        ))
    });
    let has_agent_session = session_index
        .and_then(|index| app.workspace.tabs.id_at(index))
        .and_then(|tab_id| app.session_tab(tab_id, cx))
        .is_some();
    let agent_panel = show_agent_panel
        .filter(|_| has_agent_session)
        .map(|visibility| {
            let controller = app.controllers.agent.clone();
            let render_controller = controller.clone();
            let settings = app.controllers.settings.clone();
            let terminal_selection_drag_active = app.terminal_originated_selection_drag_active();
            let panel = controller.update(cx, |controller, cx| {
                controller.render_session_agent_sidebar(
                    render_controller,
                    settings,
                    terminal_selection_drag_active,
                    window,
                    cx,
                )
            });
            render_workspace_side_panel(
                panel,
                agent_panel_width,
                visibility,
                WorkspaceSidePanelDock::Right,
            )
        });
    let sftp_progress_panel = render_session_workspace_sftp_progress_panel(app, window, cx);
    let sftp_progress_visibility = sftp_progress_panel
        .as_ref()
        .map(|(_, visibility)| *visibility)
        .unwrap_or(0.0);
    let stored_sftp_progress_flex = app.controllers.sftp.read(cx).session_progress_flex();
    let sftp_progress_flex = if stored_sftp_progress_flex.is_finite() {
        stored_sftp_progress_flex.clamp(SESSION_SFTP_MIN_SPLIT_FLEX, 0.95)
    } else {
        SESSION_SFTP_PROGRESS_DEFAULT_FLEX
    };
    let workspace_row_flex = 1.0 - sftp_progress_flex * sftp_progress_visibility;
    let is_sftp_progress_dragging = app
        .controllers
        .sftp
        .read(cx)
        .session_progress_drag()
        .is_some();

    let workspace_row = h_flex()
        .w_full()
        .flex_grow(1.0)
        .flex_shrink(1.0)
        .flex_basis(gpui::relative(workspace_row_flex))
        .min_w(px(0.0))
        .min_h(px(0.0))
        .on_mouse_move(
            cx.listener(move |this, event: &MouseMoveEvent, _window, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let Some(drag) = this.controllers.agent.read(cx).panel_drag() else {
                    return;
                };

                let pointer = f32::from(event.position.x);
                let delta = pointer - drag.initial_pointer;
                let next_width = super::session_agent_panel::clamp_session_agent_panel_width(
                    drag.initial_width - delta,
                );
                if (this.controllers.agent.read(cx).panel_width() - next_width).abs() > f32::EPSILON
                {
                    this.controllers.agent.read(cx).set_panel_width(next_width);
                    cx.notify();
                }
                cx.stop_propagation();
            }),
        )
        .capture_any_mouse_up(cx.listener(move |this, event: &MouseUpEvent, _window, cx| {
            if event.button != MouseButton::Left {
                return;
            }
            if this.controllers.agent.read(cx).take_panel_drag().is_some() {
                cx.stop_propagation();
                cx.notify();
            }
        }))
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
        .when_some(agent_panel, |this, panel| this.child(panel))
        .into_any_element();

    v_flex()
        .size_full()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .overflow_hidden()
        .on_prepaint({
            let workspace_height = workspace_height.clone();
            move |bounds, _window, _cx| {
                *workspace_height.borrow_mut() = bounds.size.height;
            }
        })
        .on_mouse_move(
            cx.listener(move |this, event: &MouseMoveEvent, _window, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let Some(drag) = this.controllers.sftp.read(cx).session_progress_drag() else {
                    return;
                };

                let pointer_delta = f32::from(event.position.y) - drag.initial_pointer;
                let next_flex = resized_session_sftp_progress_flex(
                    drag.container_height,
                    drag.initial_flex,
                    pointer_delta,
                );
                let current_flex = this.controllers.sftp.read(cx).session_progress_flex();
                if (current_flex - next_flex).abs() > f32::EPSILON {
                    this.controllers
                        .sftp
                        .read(cx)
                        .set_session_progress_flex(next_flex);
                    cx.notify();
                }
                cx.stop_propagation();
            }),
        )
        .capture_any_mouse_up(cx.listener(move |this, event: &MouseUpEvent, _window, cx| {
            if event.button != MouseButton::Left {
                return;
            }
            if this
                .controllers
                .sftp
                .read(cx)
                .take_session_progress_drag()
                .is_some()
            {
                cx.stop_propagation();
                cx.notify();
            }
        }))
        .child(workspace_row)
        .when_some(sftp_progress_panel, |this, (panel, visibility)| {
            this.child(
                div()
                    .w_full()
                    .h(px(SESSION_SFTP_BOTTOM_PROGRESS_GAP * visibility))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .opacity(visibility)
                    .child(render_session_sftp_progress_resize_handle(
                        is_sftp_progress_dragging,
                        workspace_height.clone(),
                        cx,
                    )),
            )
            .child(
                div()
                    .flex_grow(visibility)
                    .flex_shrink(1.0)
                    .flex_basis(gpui::relative(sftp_progress_flex * visibility))
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .px_3()
                    .pb_3()
                    .opacity(visibility)
                    .child(panel),
            )
        })
        .into_any_element()
}

fn render_session_workspace_sftp_progress_panel(
    app: &mut AppView,
    window: &mut Window,
    cx: &App,
) -> Option<(gpui::AnyElement, f32)> {
    let controller = app.controllers.sftp.clone();
    let visibility = controller
        .read(cx)
        .session_progress_render_visibility(window)
        .unwrap_or(0.0);
    if visibility <= 0.0 {
        return None;
    }

    let ordered_tab_ids = app.workspace.tabs.ids().collect::<Vec<_>>();
    let preferred_tab_id = app.session_side_panel_sftp_tab_id(cx);
    let panel = controller.read(cx).render_sftp_progress_center(
        controller.clone(),
        "session-sftp-progress-center",
        &ordered_tab_ids,
        preferred_tab_id,
    );

    Some((panel, visibility))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_sftp_progress_resize_moves_in_the_expected_direction() {
        let height = px(1000.0);
        let initial = SESSION_SFTP_PROGRESS_DEFAULT_FLEX;

        assert!(resized_session_sftp_progress_flex(height, initial, -100.0) > initial);
        assert!(resized_session_sftp_progress_flex(height, initial, 100.0) < initial);
    }

    #[test]
    fn session_sftp_progress_resize_preserves_minimum_panel_heights() {
        let height = px(1000.0);
        let available = session_sftp_usable_height(height);
        let minimum_progress = SESSION_SFTP_PROGRESS_MIN_HEIGHT / available;
        let maximum_progress = 1.0 - SESSION_WORKSPACE_MIN_HEIGHT / available;

        let smallest = resized_session_sftp_progress_flex(height, 0.5, 10_000.0);
        let largest = resized_session_sftp_progress_flex(height, 0.5, -10_000.0);

        assert!((smallest - minimum_progress).abs() < 0.001);
        assert!((largest - maximum_progress).abs() < 0.001);
    }
}
