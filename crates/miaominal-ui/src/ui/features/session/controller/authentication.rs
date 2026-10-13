use super::*;
use crate::ui::contracts::overlay::OverlaySnapshot;
use crate::ui::features::session::view::authentication::{
    host_key_overlay_snapshot, keyboard_interactive_overlay_snapshot,
};

#[cfg(test)]
mod tests;

impl SessionController {
    pub(in crate::ui) fn is_host_key_request_current(
        &self,
        tab_id: TabId,
        request_id: u64,
    ) -> bool {
        self.tab(tab_id).is_some_and(|tab| {
            tab.pending_host_key.is_some() && tab.pending_host_key_request_id == request_id
        })
    }

    pub(in crate::ui) fn is_keyboard_interactive_request_current(
        &self,
        tab_id: TabId,
        request_id: u64,
    ) -> bool {
        self.tab(tab_id).is_some_and(|tab| {
            tab.pending_keyboard_interactive.is_some()
                && tab.pending_keyboard_interactive_request_id == request_id
        })
    }

    /// Window lifecycle preparation. Rendering only reads these retained inputs.
    pub(in crate::ui) fn sync_visible_authentication_inputs(
        &self,
        active_tab_id: Option<TabId>,
        ordered_tab_ids: &[TabId],
        retain_exit_inputs: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pending = active_tab_id
            .into_iter()
            .chain(ordered_tab_ids.iter().copied())
            .find_map(|tab_id| {
                self.tab(tab_id).and_then(|tab| {
                    tab.pending_keyboard_interactive.clone().map(|challenge| {
                        (
                            (tab_id, tab.pending_keyboard_interactive_request_id),
                            challenge,
                        )
                    })
                })
            });
        if let Some((target, challenge)) = pending {
            if self.kbi_input_target.get() == Some(target) {
                return;
            }
            *self.kbi_inputs.borrow_mut() = challenge
                .prompts
                .iter()
                .map(|prompt| new_input_state("", "", !prompt.echo, window, cx))
                .collect();
            self.kbi_input_target.set(Some(target));
            cx.notify();
        } else if !retain_exit_inputs {
            let had_target = self.kbi_input_target.take().is_some();
            let mut inputs = self.kbi_inputs.borrow_mut();
            let had_inputs = !inputs.is_empty();
            inputs.clear();
            if had_target || had_inputs {
                drop(inputs);
                cx.notify();
            }
        }
    }

    /// Capture immutable views without allocating or changing feature state.
    pub(in crate::ui) fn overlay_snapshots(
        &self,
        entity: Entity<Self>,
        visible_tab_ids: &[TabId],
        cx: &App,
    ) -> Vec<OverlaySnapshot> {
        let mut snapshots = Vec::new();
        if let Some((tab_id, request_id, prompt)) = visible_tab_ids.iter().find_map(|tab_id| {
            self.tab(*tab_id).and_then(|tab| {
                tab.pending_host_key
                    .clone()
                    .map(|prompt| (*tab_id, tab.pending_host_key_request_id, prompt))
            })
        }) {
            snapshots.push(host_key_overlay_snapshot(
                entity.clone(),
                tab_id,
                request_id,
                prompt,
            ));
        }
        if let Some((tab_id, request_id, challenge)) = visible_tab_ids.iter().find_map(|tab_id| {
            self.tab(*tab_id).and_then(|tab| {
                tab.pending_keyboard_interactive.clone().map(|challenge| {
                    (
                        *tab_id,
                        tab.pending_keyboard_interactive_request_id,
                        challenge,
                    )
                })
            })
        }) {
            let inputs = if self.kbi_input_target.get() == Some((tab_id, request_id)) {
                self.keyboard_interactive_inputs()
            } else {
                Vec::new()
            };
            snapshots.push(keyboard_interactive_overlay_snapshot(
                entity, tab_id, request_id, challenge, inputs, cx,
            ));
        }
        snapshots
    }
}
