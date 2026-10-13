use super::*;
use crate::ui::contracts::overlay::OverlaySnapshot;
use gpui_kit::WeakFocusHandle;
use gpui_kit::component::FocusTrapElement as _;

struct FocusedOverlay {
    snapshot: OverlaySnapshot,
    focus: FocusHandle,
    restore: Option<WeakFocusHandle>,
    scope: OverlayScope,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct OverlayScope {
    tab: Option<TabId>,
    pane: PaneId,
    section: SidebarSection,
}
struct ExitingOverlay {
    snapshot: OverlaySnapshot,
    started_at: Instant,
}

#[derive(Default)]
pub(super) struct WindowOverlayState {
    active: Vec<FocusedOverlay>,
    exiting: Vec<ExitingOverlay>,
    trigger: Option<WeakFocusHandle>,
}
impl WindowOverlayState {
    pub(super) fn handle_plain_key(&self, key: &str, window: &mut Window, cx: &mut App) -> bool {
        let Some(snapshot) = self.top() else {
            return false;
        };
        match key {
            "escape" => {
                if snapshot.is_escape_dismissible() {
                    snapshot.dismiss(window, cx);
                }
                true
            }
            "enter" => {
                snapshot.submit(window, cx);
                true
            }
            _ => false,
        }
    }
    pub(super) fn top(&self) -> Option<OverlaySnapshot> {
        self.active.last().map(|surface| surface.snapshot.clone())
    }
    fn remember_trigger(&mut self, window: &Window, cx: &App) {
        self.trigger = window.focused(cx).map(|focus| focus.downgrade());
    }
    fn sync(
        &mut self,
        snapshots: Vec<OverlaySnapshot>,
        fallback: &FocusHandle,
        scope: OverlayScope,
        window: &mut Window,
        cx: &mut App,
    ) {
        let old_top = self.active.last().map(|surface| {
            (
                surface.snapshot.stable_key().to_owned(),
                surface.snapshot.revision(),
                surface.scope,
            )
        });
        let mut previous = std::mem::take(&mut self.active);
        let restore = previous.first().and_then(|surface| {
            (surface.scope == scope)
                .then(|| surface.restore.clone())
                .flatten()
        });
        for snapshot in snapshots {
            self.exiting
                .retain(|surface| surface.snapshot.stable_key() != snapshot.stable_key());
            if let Some(index) = previous.iter().position(|surface| {
                surface.snapshot.stable_key() == snapshot.stable_key()
                    && surface.snapshot.revision() == snapshot.revision()
            }) {
                let mut surface = previous.remove(index);
                surface.snapshot = snapshot;
                if surface.scope != scope {
                    surface.restore = Some(fallback.downgrade());
                    surface.scope = scope;
                }
                self.active.push(surface);
            } else {
                let replacement_restore = previous
                    .iter()
                    .find(|surface| {
                        surface.snapshot.stable_key() == snapshot.stable_key()
                            && surface.scope == scope
                    })
                    .and_then(|surface| surface.restore.clone());
                let focus = snapshot
                    .keyboard_focus()
                    .cloned()
                    .unwrap_or_else(|| cx.focus_handle());
                self.active.push(FocusedOverlay {
                    snapshot,
                    focus,
                    restore: replacement_restore
                        .or_else(|| self.trigger.clone())
                        .or_else(|| Some(fallback.downgrade())),
                    scope,
                });
            }
        }
        let next_top = self.active.last().map(|surface| {
            (
                surface.snapshot.stable_key().to_owned(),
                surface.snapshot.revision(),
                surface.scope,
            )
        });
        if old_top != next_top {
            if let Some(top) = self.active.last() {
                top.focus.focus(window, cx);
            } else if old_top.is_some() {
                restore
                    .and_then(|focus| focus.upgrade())
                    .unwrap_or_else(|| fallback.clone())
                    .focus(window, cx);
            }
            self.trigger = None;
        } else if let Some(top) = self.active.last()
            && !top.focus.contains_focused(window, cx)
        {
            top.focus.focus(window, cx);
        }
    }
    fn begin_exit(&mut self, snapshot: OverlaySnapshot) {
        self.exiting
            .retain(|surface| surface.snapshot.stable_key() != snapshot.stable_key());
        self.exiting.push(ExitingOverlay {
            snapshot,
            started_at: Instant::now(),
        });
    }
    fn render(&mut self, height: f32, window: &mut Window, cx: &App) -> AnyElement {
        let now = Instant::now();
        let duration = crate::ui::components::animation::OVERLAY_ENTER_DURATION;
        self.exiting
            .retain(|surface| now.saturating_duration_since(surface.started_at) < duration);
        if !self.exiting.is_empty() {
            window.request_animation_frame();
        }
        let exiting = self
            .exiting
            .iter()
            .map(|surface| {
                let progress = (now
                    .saturating_duration_since(surface.started_at)
                    .as_secs_f32()
                    / duration.as_secs_f32())
                .clamp(0., 1.);
                // Exiting presentation keeps its captured forms; it cannot act on them.
                div()
                    .absolute()
                    .inset_0()
                    .size_full()
                    .occlude()
                    .capture_any_mouse_down(|_, _, cx| cx.stop_propagation())
                    .capture_any_mouse_up(|_, _, cx| cx.stop_propagation())
                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                    .child(surface.snapshot.render(Some(progress), height, cx))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let mut layer = div().absolute().inset_0().size_full().children(exiting);
        for (index, surface) in self.active.iter().enumerate() {
            let content = div()
                .absolute()
                .inset_0()
                .size_full()
                .child(surface.snapshot.render(None, height, cx));
            if index + 1 == self.active.len() {
                layer = layer.child(content.focus_trap(
                    SharedString::from(format!(
                        "overlay:{}:{}",
                        surface.snapshot.stable_key(),
                        surface.snapshot.revision()
                    )),
                    &surface.focus,
                ));
            } else {
                layer = layer.child(
                    content
                        .capture_any_mouse_down(|_, _, cx| cx.stop_propagation())
                        .capture_any_mouse_up(|_, _, cx| cx.stop_propagation()),
                );
            }
        }
        layer.into_any_element()
    }
}
impl AppView {
    fn overlay_snapshots(&self, cx: &App) -> Vec<OverlaySnapshot> {
        let ordered = self.workspace.tabs.ids().collect::<Vec<_>>();
        let mut visible = Vec::new();
        visible.extend(self.workspace.workspace.active_tab);
        visible.extend(
            ordered
                .iter()
                .copied()
                .filter(|id| !visible.contains(id))
                .collect::<Vec<_>>(),
        );
        let mut snapshots = self.controllers.session.read(cx).overlay_snapshots(
            self.controllers.session.clone(),
            &visible,
            cx,
        );
        for surface in [
            self.controllers
                .hosts
                .read(cx)
                .current_overlay(self.controllers.hosts.clone()),
            self.controllers
                .trusted_hosts
                .read(cx)
                .current_overlay(self.controllers.trusted_hosts.clone()),
            self.controllers
                .snippets
                .read(cx)
                .current_overlay(self.controllers.snippets.clone()),
            self.controllers
                .forwarding
                .read(cx)
                .current_overlay(self.controllers.forwarding.clone()),
        ]
        .into_iter()
        .flatten()
        {
            snapshots.push(surface);
        }
        snapshots.extend(
            self.controllers
                .keychain
                .read(cx)
                .overlay_snapshots(self.controllers.keychain.clone(), cx),
        );
        snapshots.extend(
            self.controllers
                .agent
                .read(cx)
                .overlay_snapshots(self.controllers.agent.clone(), cx),
        );
        snapshots.extend(
            self.controllers
                .settings
                .read(cx)
                .overlay_snapshots(self.controllers.settings.clone(), cx),
        );
        let visible_sftp = self
            .workspace
            .active_topbar_tab
            .filter(|id| {
                self.workspace
                    .tabs
                    .get(*id)
                    .is_some_and(|tab| tab.is_sftp())
            })
            .into_iter()
            .chain(self.session_side_panel_sftp_tab_id(cx))
            .collect::<Vec<_>>();
        snapshots.extend(self.controllers.sftp.read(cx).overlay_snapshots(
            self.controllers.sftp.clone(),
            &visible_sftp,
            &ordered,
            cx,
        ));
        snapshots
    }
    pub(super) fn sync_window_overlays(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.schedule_pane_split_animation(window, cx);
        self.sync_agent_surface_lifecycle(cx);
        let ordered = self.workspace.tabs.ids().collect::<Vec<_>>();
        self.controllers
            .session
            .clone()
            .update(cx, |controller, cx| {
                controller.sync_visible_authentication_inputs(
                    self.workspace.workspace.active_tab,
                    &ordered,
                    false,
                    window,
                    cx,
                );
            });
        let snapshots = self.overlay_snapshots(cx);
        self.shell.overlay_state.sync(
            snapshots,
            &self.workspace.workspace.active_pane.terminal_focus,
            OverlayScope {
                tab: self.workspace.active_topbar_tab,
                pane: self.workspace.workspace.active_pane_id,
                section: self.shell.shell_state.sidebar_section,
            },
            window,
            cx,
        );
    }

    pub(super) fn sync_agent_surface_lifecycle(&mut self, cx: &mut Context<Self>) {
        let active = self
            .active_terminal_session_index(cx)
            .and_then(|index| self.workspace.tabs.id_at(index))
            .and_then(|id| self.workspace.tabs.get(id))
            .is_some_and(|tab| !tab.is_local_terminal())
            && self.controllers.agent.read(cx).panel_open();
        self.controllers.agent.update(cx, |controller, cx| {
            controller.set_surface_active(active, cx);
        });
    }
    pub(super) fn remember_overlay_trigger_focus(&mut self, window: &Window, cx: &App) {
        self.shell.overlay_state.remember_trigger(window, cx);
    }
    pub(super) fn start_dialog_exit(&mut self, snapshot: OverlaySnapshot, cx: &mut Context<Self>) {
        self.shell.overlay_state.begin_exit(snapshot);
        cx.notify();
    }
    pub(super) fn render_window_overlays(
        &mut self,
        height: f32,
        window: &mut Window,
        cx: &App,
    ) -> AnyElement {
        self.shell.overlay_state.render(height, window, cx)
    }
}

#[cfg(test)]
mod tests;
