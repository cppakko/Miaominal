use super::*;
use crate::ui::contracts::overlay::OverlaySnapshot;
use gpui_kit::App;

impl SftpController {
    /// Workspace supplies visible placements; SFTP owns prompt selection and payloads.
    pub(in crate::ui) fn overlay_snapshots(
        &self,
        entity: Entity<Self>,
        visible_tab_ids: &[TabId],
        ordered_tab_ids: &[TabId],
        cx: &App,
    ) -> Vec<OverlaySnapshot> {
        let selected = visible_tab_ids
            .iter()
            .find_map(|tab_id| self.prompt(*tab_id).map(|prompt| (*tab_id, prompt)))
            .or_else(|| {
                ordered_tab_ids.iter().find_map(|tab_id| {
                    self.prompt(*tab_id)
                        .filter(|prompt| {
                            matches!(prompt.kind, SftpPromptKind::ConfirmHostKey { .. })
                        })
                        .map(|prompt| (*tab_id, prompt))
                })
            });
        selected
            .map(|(tab_id, prompt)| self.prompt_overlay_snapshot(tab_id, prompt, entity, cx))
            .into_iter()
            .collect()
    }

    pub(super) fn prompt_overlay_snapshot(
        &self,
        tab_id: TabId,
        prompt: SftpPromptState,
        entity: Entity<Self>,
        cx: &App,
    ) -> OverlaySnapshot {
        let revision = self
            .interactions
            .borrow()
            .get(&tab_id)
            .map(|interaction| interaction.prompt_revision)
            .unwrap_or_default();
        let render_entity = entity.clone();
        let render_target = OverlayTarget::new(entity.clone(), move |this: &Self| {
            this.interactions
                .borrow()
                .get(&tab_id)
                .is_some_and(|interaction| {
                    interaction.prompt_revision == revision && interaction.prompt.is_some()
                })
        });
        let prompt_input = self.prompt_input();
        let dismiss_entity = entity;
        let is_host_key_prompt = matches!(prompt.kind, SftpPromptKind::ConfirmHostKey { .. });
        let mut snapshot = OverlaySnapshot::new(
            format!("sftp-prompt-{tab_id}"),
            revision,
            move |exit_progress, viewport_height, cx| {
                render_entity.read(cx).render_sftp_prompt_overlay(
                    render_target.clone(),
                    prompt_input.clone(),
                    tab_id,
                    &prompt,
                    exit_progress,
                    viewport_height,
                )
            },
            move |_, cx| {
                dismiss_entity.update(cx, |this, cx| {
                    let matches =
                        this.interactions
                            .borrow()
                            .get(&tab_id)
                            .is_some_and(|interaction| {
                                interaction.prompt_revision == revision
                                    && interaction.prompt.is_some()
                            });
                    if matches {
                        this.cancel_prompt(tab_id, cx);
                    }
                });
            },
        );
        if !is_host_key_prompt {
            snapshot = snapshot.with_keyboard_focus(self.prompt_input().focus_handle(cx));
        }
        snapshot
    }
}
