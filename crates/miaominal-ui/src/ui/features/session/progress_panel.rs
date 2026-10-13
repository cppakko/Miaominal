//! Retained progress panel state owned by one live Session tab.
use crate::ui::components::progress_panel::{
    ProgressPanelDrag, ProgressPanelTransition, progress_center_render_visibility,
    update_progress_center_state,
};
use gpui_kit::Window;
use std::time::Instant;
const SESSION_SFTP_PROGRESS_DEFAULT_FLEX: f32 = 0.26;

#[derive(Debug, Clone)]
pub(crate) struct SessionSftpProgressLayoutState {
    visible: bool,
    transition: Option<ProgressPanelTransition>,
    flex: f32,
    drag: Option<ProgressPanelDrag>,
}

impl Default for SessionSftpProgressLayoutState {
    fn default() -> Self {
        Self {
            visible: false,
            transition: None,
            flex: SESSION_SFTP_PROGRESS_DEFAULT_FLEX,
            drag: None,
        }
    }
}

impl SessionSftpProgressLayoutState {
    pub(crate) fn visible(&self) -> bool {
        self.visible
    }

    pub(crate) fn set_visible(&mut self, visible: bool) -> bool {
        update_progress_center_state(
            &mut self.visible,
            &mut self.transition,
            visible,
            Instant::now(),
        )
    }

    pub(crate) fn render_visibility(&mut self, window: &mut Window) -> Option<f32> {
        progress_center_render_visibility(self.visible, &mut self.transition, window)
    }

    pub(crate) fn flex(&self) -> f32 {
        self.flex
    }

    pub(crate) fn set_flex(&mut self, flex: f32) {
        self.flex = flex;
    }

    pub(crate) fn drag(&self) -> Option<ProgressPanelDrag> {
        self.drag.clone()
    }

    pub(crate) fn set_drag(&mut self, drag: Option<ProgressPanelDrag>) {
        self.drag = drag;
    }

    pub(crate) fn take_drag(&mut self) -> Option<ProgressPanelDrag> {
        self.drag.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_progress_layout_state_is_independent_per_tab() {
        let mut first = SessionSftpProgressLayoutState::default();
        let second = SessionSftpProgressLayoutState::default();

        assert!(first.set_visible(true));
        first.set_flex(0.42);

        assert!(first.visible());
        assert_eq!(first.flex(), 0.42);
        assert!(!second.visible());
        assert_eq!(second.flex(), SESSION_SFTP_PROGRESS_DEFAULT_FLEX);
    }
}
