use super::super::super::*;

impl AppView {
    pub(in crate::ui::shell) fn handle_forward_view_mode_change(
        &mut self,
        mode: ProfileViewMode,
        cx: &mut Context<Self>,
    ) {
        self.panel_view.forward_view_mode = mode;
        cx.notify();
    }
}
