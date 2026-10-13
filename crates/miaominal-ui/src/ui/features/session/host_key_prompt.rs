use crate::ui::shell::{App, HostKeyDecision, HostKeyPrompt, SessionController};

impl SessionController {
    pub(in crate::ui::shell) fn render_trusted_host_key_prompt(
        &self,
        on_decision: impl Fn(HostKeyDecision, &mut App) + Clone + 'static,
        prompt: &HostKeyPrompt,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
    ) -> gpui_kit::AnyElement {
        crate::ui::components::host_key_prompt::render_host_key_prompt(
            on_decision,
            prompt,
            exit_progress,
            bottom_popup_viewport_height,
            "trusted-host-key",
        )
    }
}
