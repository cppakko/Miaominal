use super::*;
use crate::ui::contracts::overlay::OverlaySnapshot;

#[derive(Clone)]
pub(super) enum AgentOverlay {
    ChatSessionDelete(PendingChatSessionDeleteState),
    ChatSessionRename(PendingChatSessionRenameState),
}

impl AgentOverlay {
    fn surface_key(&self) -> &'static str {
        match self {
            Self::ChatSessionDelete(..) => "chat-session-delete",
            Self::ChatSessionRename(..) => "chat-session-rename",
        }
    }
}

impl AgentController {
    pub(in crate::ui) fn overlay_snapshots(
        &self,
        entity: Entity<Self>,
        cx: &App,
    ) -> Vec<OverlaySnapshot> {
        let mut snapshots = Vec::new();
        if let Some(prompt) = self.pending_chat_session_delete.clone() {
            snapshots.push(self.overlay_snapshot(
                AgentOverlay::ChatSessionDelete(prompt),
                entity.clone(),
                cx,
            ));
        }
        if let Some(prompt) = self.pending_chat_session_rename.clone() {
            snapshots.push(self.overlay_snapshot(
                AgentOverlay::ChatSessionRename(prompt),
                entity.clone(),
                cx,
            ));
        }
        snapshots
    }

    fn overlay_active(&self, prompt: &AgentOverlay) -> bool {
        match prompt {
            AgentOverlay::ChatSessionDelete(..) => self.pending_chat_session_delete.is_some(),
            AgentOverlay::ChatSessionRename(..) => self.pending_chat_session_rename.is_some(),
        }
    }

    pub(super) fn overlay_snapshot(
        &self,
        prompt: AgentOverlay,
        entity: Entity<Self>,
        cx: &App,
    ) -> OverlaySnapshot {
        let key = prompt.surface_key();
        let revision = self.overlay_revisions.current(key);
        let render_entity = entity.clone();
        let render_prompt = prompt.clone();
        let target_prompt = prompt.clone();
        let render_target = OverlayTarget::new(entity.clone(), move |this: &Self| {
            this.overlay_revisions.current(key) == revision && this.overlay_active(&target_prompt)
        });
        let retained_input = self.forms.rename_title_input.clone();
        let dismiss_entity = entity.clone();
        let dismiss_prompt = prompt.clone();
        let mut snapshot = OverlaySnapshot::new(
            key,
            revision,
            move |exit_progress, viewport_height, cx| {
                render_entity.read(cx).render_owned_overlay(
                    render_target.clone(),
                    retained_input.clone(),
                    &render_prompt,
                    exit_progress,
                    viewport_height,
                    cx,
                )
            },
            move |window, cx| {
                dismiss_entity.update(cx, |this, cx| {
                    if this.overlay_revisions.current(key) != revision
                        || !this.overlay_active(&dismiss_prompt)
                    {
                        return;
                    }
                    this.dismiss_owned_overlay(&dismiss_prompt, window, cx);
                });
            },
        );
        let focus = match &prompt {
            AgentOverlay::ChatSessionDelete(..) => None,
            AgentOverlay::ChatSessionRename(..) => {
                Some(self.forms.rename_title_input.focus_handle(cx))
            }
        };
        if let Some(focus) = focus {
            snapshot = snapshot.with_keyboard_focus(focus);
        }

        snapshot
    }

    fn render_owned_overlay(
        &self,
        entity: OverlayTarget<Self>,
        retained_input: Entity<InputState>,
        prompt: &AgentOverlay,
        exit_progress: Option<f32>,
        _viewport_height: f32,
        cx: &App,
    ) -> AnyElement {
        match prompt {
            AgentOverlay::ChatSessionDelete(prompt) => {
                self.render_chat_session_delete_prompt(entity, prompt, exit_progress)
            }
            AgentOverlay::ChatSessionRename(prompt) => self.render_chat_session_rename_prompt(
                entity,
                retained_input,
                prompt,
                exit_progress,
                cx,
            ),
        }
    }

    fn dismiss_owned_overlay(
        &mut self,
        prompt: &AgentOverlay,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match prompt {
            AgentOverlay::ChatSessionDelete(..) => {
                self.cancel_chat_session_delete(cx);
            }
            AgentOverlay::ChatSessionRename(..) => {
                self.cancel_chat_session_rename(cx);
            }
        }
    }
}
