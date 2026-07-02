use super::super::workspace::PaneLayout;
use super::super::*;
use super::workspace_side_panel::{
    WorkspaceSidePanelDock, render_workspace_side_panel, workspace_side_panel_render_state,
};

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

        h_flex()
            .size_full()
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
            .into_any_element()
    }
}
