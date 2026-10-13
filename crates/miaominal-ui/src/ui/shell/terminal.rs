//! Window adapter for Session input. This is the only input implementation that
//! knows AppView; Session policy sees geometry, identities and workspace requests.
use super::*;
use crate::ui::contracts::TerminalWorkspaceRequest;
use crate::ui::features::session::{
    PaneViewState, TerminalDragState, TerminalInputHost, TerminalKeyboardState,
    TerminalMouseGesture, TerminalPanePresentation,
};

impl TerminalInputHost for AppView {
    fn terminal_owner(&self) -> Entity<SessionController> {
        self.controllers.session.clone()
    }

    fn active_terminal_tab_id(&self) -> Option<TabId> {
        self.workspace.workspace.active_tab
    }

    fn active_terminal_pane_id(&self) -> PaneId {
        self.workspace.workspace.active_pane_id
    }

    fn active_terminal_pane(&self) -> &PaneViewState {
        &self.workspace.workspace.active_pane
    }

    fn active_terminal_pane_mut(&mut self) -> &mut PaneViewState {
        &mut self.workspace.workspace.active_pane
    }

    fn terminal_pane_ids(&self) -> Vec<PaneId> {
        std::iter::once(self.workspace.workspace.active_pane_id)
            .chain(self.workspace.workspace.parked_panes.keys().copied())
            .collect()
    }

    fn terminal_pane_presentation(&self, pane_id: PaneId) -> Option<TerminalPanePresentation> {
        if pane_id == self.workspace.workspace.active_pane_id {
            let state = &self.workspace.workspace.active_pane;
            Some(TerminalPanePresentation {
                tab_id: self.workspace.workspace.active_tab,
                bounds: state.terminal_bounds,
                cell_width: state.terminal_cell_width,
                line_height: state.terminal_line_height,
                pointer_position: state.terminal_pointer_position,
                dragging_scrollbar: state.terminal_scrollbar_drag.is_some(),
                scrollbar_interaction: state.terminal_scrollbar_last_interaction_at,
            })
        } else {
            let state = self.workspace.workspace.parked_panes.get(&pane_id)?;
            Some(TerminalPanePresentation {
                tab_id: state.active_tab,
                bounds: state.terminal_bounds,
                cell_width: state.terminal_cell_width,
                line_height: state.terminal_line_height,
                pointer_position: state.terminal_pointer_position,
                dragging_scrollbar: state.terminal_scrollbar_drag.is_some(),
                scrollbar_interaction: state.terminal_scrollbar_last_interaction_at,
            })
        }
    }

    fn for_each_parked_terminal_input(
        &mut self,
        mut visit: impl FnMut(&mut TerminalKeyboardState, &mut Option<TerminalMouseGesture>),
    ) {
        for pane in self.workspace.workspace.parked_panes.values_mut() {
            visit(
                &mut pane.terminal_keyboard,
                &mut pane.terminal_mouse_gesture,
            );
        }
        for workspace in self.workspace.parked_workspaces.values_mut() {
            visit(
                &mut workspace.active_pane.terminal_keyboard,
                &mut workspace.active_pane.terminal_mouse_gesture,
            );
            for pane in workspace.parked_panes.values_mut() {
                visit(
                    &mut pane.terminal_keyboard,
                    &mut pane.terminal_mouse_gesture,
                );
            }
        }
    }

    fn terminal_drag(&self) -> &TerminalDragState {
        &self.workspace.terminal_drag
    }

    fn terminal_drag_mut(&mut self) -> &mut TerminalDragState {
        &mut self.workspace.terminal_drag
    }

    fn set_terminal_scrollbar_interaction(&mut self, pane_id: PaneId, at: Instant) -> bool {
        if pane_id == self.workspace.workspace.active_pane_id {
            self.workspace
                .workspace
                .active_pane
                .terminal_scrollbar_last_interaction_at = Some(at);
        } else if let Some(state) = self.workspace.workspace.parked_panes.get_mut(&pane_id) {
            state.terminal_scrollbar_last_interaction_at = Some(at);
        } else {
            return false;
        }
        true
    }

    fn dispatch_terminal_workspace_request(
        &mut self,
        request: TerminalWorkspaceRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match request {
            TerminalWorkspaceRequest::OpenSearch => self.open_terminal_search(window, cx),
            TerminalWorkspaceRequest::Split(direction) => {
                self.split_active_pane(direction, window, cx)
            }
            TerminalWorkspaceRequest::ClosePane => self.close_active_pane(window, cx),
        }
    }
}
