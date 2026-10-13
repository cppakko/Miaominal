use super::*;
use crate::ui::components::progress_panel::ProgressPanelDrag;
use crate::ui::features::session::progress_panel::{
    clamp_session_sftp_progress_flex, resized_session_sftp_progress_flex,
};
use gpui_kit::Pixels;

impl SessionController {
    pub(in crate::ui) fn sftp_progress_layout_snapshot(
        &self,
        tab_id: TabId,
    ) -> Option<(f32, bool)> {
        self.tab(tab_id).map(|tab| {
            (
                tab.sftp_progress_layout.flex(),
                tab.sftp_progress_layout.drag().is_some(),
            )
        })
    }

    pub(in crate::ui) fn set_sftp_progress_visible(&self, tab_id: TabId, visible: bool) -> bool {
        self.tab_mut(tab_id)
            .is_some_and(|mut tab| tab.sftp_progress_layout.set_visible(visible))
    }

    pub(in crate::ui) fn begin_sftp_progress_resize(
        &self,
        tab_id: TabId,
        pointer_y: f32,
        height: Pixels,
    ) -> bool {
        let Some(mut tab) = self.tab_mut(tab_id) else {
            return false;
        };
        let initial_flex =
            clamp_session_sftp_progress_flex(height, tab.sftp_progress_layout.flex());
        tab.sftp_progress_layout.set_flex(initial_flex);
        tab.sftp_progress_layout.set_drag(Some(ProgressPanelDrag {
            initial_pointer: pointer_y,
            initial_flex,
            container_height: height,
        }));
        true
    }

    pub(in crate::ui) fn update_sftp_progress_resize(
        &self,
        tab_id: TabId,
        pointer_y: f32,
    ) -> Option<bool> {
        let mut tab = self.tab_mut(tab_id)?;
        let drag = tab.sftp_progress_layout.drag()?;
        let next = resized_session_sftp_progress_flex(
            drag.container_height,
            drag.initial_flex,
            pointer_y - drag.initial_pointer,
        );
        let changed = (tab.sftp_progress_layout.flex() - next).abs() > f32::EPSILON;
        if changed {
            tab.sftp_progress_layout.set_flex(next);
        }
        Some(changed)
    }

    pub(in crate::ui) fn finish_sftp_progress_resize(&self, tab_id: TabId) -> bool {
        self.tab_mut(tab_id)
            .is_some_and(|mut tab| tab.sftp_progress_layout.take_drag().is_some())
    }

    pub(in crate::ui) fn sample_sftp_progress_visibility(
        &self,
        tab_id: TabId,
        window: &mut Window,
    ) -> Option<f32> {
        self.tab_mut(tab_id)?
            .sftp_progress_layout
            .render_visibility(window)
    }
}
