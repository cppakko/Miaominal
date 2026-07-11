use super::super::workspace::PaneLayout;
use super::super::*;
use super::workspace_side_panel::{
    WorkspaceSidePanelDock, render_workspace_side_panel, workspace_side_panel_render_state,
};

const SESSION_SFTP_BOTTOM_PROGRESS_FLEX: f32 = 0.26;
const SESSION_SFTP_BOTTOM_PROGRESS_GAP: f32 = 8.0;

impl AppView {
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
        let show_agent_panel = workspace_side_panel_render_state(
            self.panels.session_agent_panel_open && session_index.is_some(),
            &mut self.panels.visible_session_agent_panel,
            &mut self.panels.session_agent_panel_transition,
            window,
        );
        self.workspace_state.session_agent_panel_width =
            super::session_agent_panel::clamp_session_agent_panel_width(
                self.workspace_state.session_agent_panel_width,
            );
        let agent_panel_width = self.workspace_state.session_agent_panel_width;
        let entity = cx.entity();
        let side_panel = show_side_panel.and_then(|visibility| {
            session_index
                .and_then(|index| self.workspace_state.tabs.get(index))
                .and_then(|tab| tab.as_session().map(|session| (tab.id, session)))
                .map(|(session_tab_id, session)| {
                    render_workspace_side_panel(
                        self.render_session_workspace_side_panel(
                            entity.clone(),
                            session_tab_id,
                            session,
                            cx,
                        ),
                        SESSION_MONITOR_PANEL_WIDTH,
                        visibility,
                        WorkspaceSidePanelDock::Left,
                    )
                })
        });
        let has_agent_session = session_index
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(TabState::as_session)
            .is_some();
        let agent_panel = show_agent_panel
            .filter(|_| has_agent_session)
            .map(|visibility| {
                render_workspace_side_panel(
                    self.render_session_agent_sidebar(entity.clone(), window, cx),
                    agent_panel_width,
                    visibility,
                    WorkspaceSidePanelDock::Right,
                )
            });
        let sftp_progress_panel =
            self.render_session_workspace_sftp_progress_panel(entity.clone(), window);
        let sftp_progress_visibility = sftp_progress_panel
            .as_ref()
            .map(|(_, visibility)| *visibility)
            .unwrap_or(0.0);
        let workspace_row_flex = 1.0 - SESSION_SFTP_BOTTOM_PROGRESS_FLEX * sftp_progress_visibility;

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
                    let Some(drag) = this.workspace_state.session_agent_panel_drag.clone() else {
                        return;
                    };

                    let pointer = f32::from(event.position.x);
                    let delta = pointer - drag.initial_pointer;
                    let next_width = super::session_agent_panel::clamp_session_agent_panel_width(
                        drag.initial_width - delta,
                    );
                    if (this.workspace_state.session_agent_panel_width - next_width).abs()
                        > f32::EPSILON
                    {
                        this.workspace_state.session_agent_panel_width = next_width;
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
                    .workspace_state
                    .session_agent_panel_drag
                    .take()
                    .is_some()
                {
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
            .child(workspace_row)
            .when_some(sftp_progress_panel, |this, (panel, visibility)| {
                this.child(
                    div()
                        .w_full()
                        .h(px(SESSION_SFTP_BOTTOM_PROGRESS_GAP * visibility))
                        .flex_shrink_0(),
                )
                .child(
                    div()
                        .flex_grow(visibility)
                        .flex_shrink(1.0)
                        .flex_basis(gpui::relative(
                            SESSION_SFTP_BOTTOM_PROGRESS_FLEX * visibility,
                        ))
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
        &mut self,
        entity: Entity<Self>,
        window: &mut Window,
    ) -> Option<(gpui::AnyElement, f32)> {
        let visibility = self
            .session_sftp_progress_center_render_visibility(window)
            .unwrap_or(0.0);
        if visibility <= 0.0 {
            return None;
        }

        let panel = self.render_sftp_progress_center(entity, "session-sftp-progress-center");

        Some((panel, visibility))
    }
}
