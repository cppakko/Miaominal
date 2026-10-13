//! Retained progress panel state owned by one live Session tab.
use crate::ui::components::progress_panel::{
    ProgressPanelDrag, ProgressPanelTransition, progress_center_render_visibility,
    update_progress_center_state,
};
use gpui_kit::{Pixels, Window};
use std::time::Instant;
const SESSION_SFTP_PROGRESS_DEFAULT_FLEX: f32 = 0.26;

const SESSION_SFTP_BOTTOM_PROGRESS_GAP: f32 = 8.0;
const SESSION_SFTP_PROGRESS_MIN_HEIGHT: f32 = 220.0;
const SESSION_WORKSPACE_MIN_HEIGHT: f32 = 240.0;
const SESSION_SFTP_MIN_SPLIT_FLEX: f32 = 0.05;

fn session_sftp_usable_height(container_height: Pixels) -> f32 {
    (container_height.as_f32() - SESSION_SFTP_BOTTOM_PROGRESS_GAP).max(1.0)
}

pub(super) fn clamp_session_sftp_progress_flex(container_height: Pixels, requested: f32) -> f32 {
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

pub(super) fn resized_session_sftp_progress_flex(
    container_height: Pixels,
    initial_flex: f32,
    pointer_delta: f32,
) -> f32 {
    let requested = initial_flex - pointer_delta / session_sftp_usable_height(container_height);
    clamp_session_sftp_progress_flex(container_height, requested)
}

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
    use gpui_kit::px;
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
