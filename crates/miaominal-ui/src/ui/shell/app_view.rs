use crate::ui::contracts::session::PreparedSessionTab;
use crate::ui::contracts::session::SessionTabSnapshot;

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppWindowRole {
    Primary,
    Detached,
}

fn should_prompt_terminal_sftp_download(
    active_sftp_tab_id: Option<TabId>,
    session_sftp_tab_id: Option<TabId>,
    target_tab_id: TabId,
) -> bool {
    active_sftp_tab_id != Some(target_tab_id) && session_sftp_tab_id == Some(target_tab_id)
}

pub struct AppView {
    pub(in crate::ui::shell) controllers: ControllerSet,
    pub(in crate::ui::shell) workspace: WorkspaceModel,
    pub(in crate::ui::shell) shell: ShellUiState,
    pub(in crate::ui::shell) application_generations:
        crate::ui::application::ApplicationGenerations,
    pub(in crate::ui::shell) applying_application_snapshot: bool,
    pub(in crate::ui::shell) window_role: AppWindowRole,
    pub(in crate::ui::shell) _subscriptions: RootSubscriptions,
}

pub(in crate::ui::shell) struct ShellUiState {
    pub(in crate::ui::shell) pane_animation_frame_pending: bool,
    pub(in crate::ui::shell) workspace_forms: WorkspaceForms,
    pub(in crate::ui::shell) shell_state: ShellState,
    pub(in crate::ui::shell) overlay_state: super::overlays::WindowOverlayState,
    pub(in crate::ui::shell) status_message: String,
    pub(in crate::ui::shell) deferred_app_command: Option<DeferredAppCommand>,
}

impl AppView {
    pub(in crate::ui::shell) fn session_tab(
        &self,
        tab_id: TabId,
        cx: &App,
    ) -> Option<SessionTabSnapshot> {
        self.workspace.tabs.get(tab_id)?;
        self.controllers.session.read(cx).tab_snapshot(tab_id)
    }

    pub(in crate::ui::shell) fn insert_session_tab(
        &mut self,
        tab: TabState,
        session: PreparedSessionTab,
        cx: &App,
    ) {
        let tab_id = tab.id;
        debug_assert!(tab.is_session());
        assert!(
            self.workspace.tabs.get(tab_id).is_none(),
            "duplicate session tab metadata for {tab_id}"
        );
        self.controllers
            .session
            .read(cx)
            .insert_tab(tab_id, session);
        self.register_session_tab_metadata(tab, cx);
    }

    pub(in crate::ui::shell) fn register_session_tab_metadata(&mut self, tab: TabState, cx: &App) {
        let tab_id = tab.id;
        debug_assert!(tab.is_session());
        assert!(
            self.workspace.tabs.get(tab_id).is_none(),
            "duplicate session tab metadata for {tab_id}"
        );
        assert!(
            self.controllers.session.read(cx).contains_tab(tab_id),
            "missing session tab payload for {tab_id}"
        );
        self.workspace.tabs.push(tab);
        self.sync_session_port_snapshot(cx);
    }

    pub(in crate::ui::shell) fn remove_tab_metadata_after_controller_close(
        &mut self,
        tab_id: TabId,
        cx: &App,
    ) -> Option<TabState> {
        let tab = self.workspace.tabs.remove_id(tab_id)?;
        self.workspace.parked_workspaces.remove(&tab_id);
        self.sync_session_port_snapshot(cx);
        Some(tab)
    }

    pub(in crate::ui::shell) fn replace_with_session_tab(
        &mut self,
        index: usize,
        tab: TabState,
        session: PreparedSessionTab,
        cx: &App,
    ) {
        debug_assert!(tab.is_session());
        let next_id = tab.id;
        let previous = self.workspace.tabs.replace(index, tab);
        self.workspace.parked_workspaces.remove(&previous.id);
        if previous.is_session() {
            assert!(
                self.controllers.session.read(cx).close_tab(previous.id),
                "session tab metadata must have payload"
            );
        }
        self.controllers
            .session
            .read(cx)
            .insert_tab(next_id, session);
        self.sync_session_port_snapshot(cx);
    }

    pub(in crate::ui::shell) fn remove_tab_payload_and_metadata(
        &mut self,
        tab_id: TabId,
        cx: &App,
    ) -> Option<TabState> {
        self.workspace.tabs.get(tab_id)?;
        let tab = self.workspace.tabs.remove_id(tab_id)?;
        self.workspace.parked_workspaces.remove(&tab_id);
        if tab.is_session() {
            assert!(
                self.controllers.session.read(cx).close_tab(tab_id),
                "session tab metadata must have payload"
            );
        }
        self.sync_session_port_snapshot(cx);
        Some(tab)
    }

    pub(in crate::ui::shell) fn finish_any_active_sftp_drag_selection(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        self.controllers.sftp.update(cx, |controller, cx| {
            controller.finish_any_active_drag_selection(cx)
        })
    }

    fn active_session_sftp_drag_tab_id(&self, cx: &App) -> Option<TabId> {
        let session = self.controllers.session.read(cx);
        let panel_visible =
            session.side_panel_open() && session.side_panel_view() == SessionSidePanelView::Sftp;
        if !panel_visible {
            return None;
        }

        self.session_side_panel_sftp_tab_id(cx)
    }

    pub(in crate::ui::shell) fn update_session_sftp_drag_selection(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab_id) = self.active_session_sftp_drag_tab_id(cx) else {
            return false;
        };

        self.controllers.sftp.update(cx, |controller, cx| {
            controller.update_active_drag_selection(tab_id, position, cx)
        })
    }

    pub(in crate::ui::shell) fn finish_session_sftp_drag_selection(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab_id) = self.active_session_sftp_drag_tab_id(cx) else {
            return false;
        };

        self.controllers.sftp.update(cx, |controller, cx| {
            controller.finish_active_drag_selection(tab_id, position, cx)
        })
    }

    pub(in crate::ui::shell) fn should_sync_sftp_browser_for_tab(
        &self,
        tab_id: TabId,
        cx: &App,
    ) -> bool {
        let active_tab_matches = self
            .workspace
            .active_topbar_tab
            .and_then(|tab_id| self.workspace.tabs.get(tab_id))
            .is_some_and(|tab| tab.id == tab_id && tab.is_sftp());
        if active_tab_matches {
            return true;
        }

        self.controllers.session.read(cx).side_panel_open()
            && self.controllers.session.read(cx).side_panel_view() == SessionSidePanelView::Sftp
            && self.session_side_panel_sftp_tab_id(cx) == Some(tab_id)
    }

    pub(in crate::ui::shell) fn sync_sftp_path_inputs_for_tab(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        if !self.should_sync_sftp_browser_for_tab(tab_id, cx) {
            return;
        }
        let controller = self.controllers.sftp.clone();
        self.with_active_window(cx, move |window, cx| {
            controller.update(cx, |controller, cx| {
                controller.sync_path_inputs_for_tab(tab_id, window, cx);
            });
        });
    }

    pub(in crate::ui::shell) fn sync_active_sftp_path_inputs(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.workspace.active_topbar_tab else {
            return;
        };
        self.sync_sftp_path_inputs_for_tab(tab_id, cx);
    }

    pub(in crate::ui::shell) fn sync_sftp_tables_for_tab(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        if !self.should_sync_sftp_browser_for_tab(tab_id, cx) {
            return;
        }
        let prompt_download_destination = self.should_prompt_sftp_download_destination(tab_id, cx);
        self.controllers.sftp.update(cx, |controller, cx| {
            controller.set_download_destination_prompt_tab(tab_id, prompt_download_destination);
            controller.sync_tables_for_tab(tab_id, cx);
        });
    }

    pub(in crate::ui::shell) fn sync_active_sftp_tables(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.workspace.active_topbar_tab else {
            return;
        };
        self.sync_sftp_tables_for_tab(tab_id, cx);
    }

    pub(in crate::ui::shell) fn active_sftp_tab_id(&self) -> Option<TabId> {
        self.workspace.active_topbar_tab.filter(|tab_id| {
            self.workspace
                .tabs
                .get(*tab_id)
                .is_some_and(|tab| tab.is_sftp())
        })
    }

    pub(in crate::ui::shell) fn should_prompt_sftp_download_destination(
        &self,
        tab_id: TabId,
        cx: &App,
    ) -> bool {
        should_prompt_terminal_sftp_download(
            self.active_sftp_tab_id(),
            self.session_side_panel_sftp_tab_id(cx),
            tab_id,
        )
    }

    pub(in crate::ui::shell) fn insert_sftp_tab(&mut self, tab: TabState, cx: &App) {
        let tab_id = tab.id;
        debug_assert_eq!(tab.kind, TabKindTag::Sftp);
        assert!(
            self.workspace.tabs.get(tab_id).is_none(),
            "duplicate SFTP tab metadata for {tab_id}"
        );
        assert!(
            self.controllers.sftp.read(cx).contains_tab(tab_id),
            "missing SFTP tab payload for {tab_id}"
        );
        self.workspace.push_sftp_tab(tab);
    }

    fn session_port_snapshot(&self, cx: &App) -> SessionPortSnapshot {
        let sessions = self
            .workspace
            .tabs
            .iter()
            .filter_map(|tab| {
                self.controllers
                    .session
                    .read(cx)
                    .port_session(tab.id, tab.title.clone())
            })
            .collect();
        let active_profile_id = self
            .workspace
            .workspace
            .active_tab
            .and_then(|tab_id| self.session_tab(tab_id, cx))
            .map(|session| session.profile_id().to_string())
            .or_else(|| {
                self.workspace
                    .active_topbar_tab
                    .and_then(|tab_id| self.controllers.sftp.read(cx).profile_id(tab_id))
            });
        let active_terminal_tab_id = self
            .workspace
            .workspace
            .active_tab
            .filter(|&tab_id| {
                self.session_tab(tab_id, cx)
                    .is_some_and(|session| session.purpose() == SessionPurpose::Terminal)
            })
            .or_else(|| {
                self.workspace.active_topbar_tab.filter(|&tab_id| {
                    self.session_tab(tab_id, cx)
                        .is_some_and(|session| session.purpose() == SessionPurpose::Terminal)
                })
            });
        SessionPortSnapshot::new(
            self.controllers.session.read(cx).profiles().clone(),
            sessions,
            active_profile_id,
            active_terminal_tab_id,
        )
    }

    pub(in crate::ui::shell) fn sync_session_port_snapshot(&self, cx: &App) {
        self.controllers
            .session
            .read(cx)
            .sync_port_snapshot(self.session_port_snapshot(cx));
        self.controllers
            .session
            .read(cx)
            .sync_terminal_search_target(self.workspace.workspace.active_tab, cx);
    }

    pub(in crate::ui::shell) fn set_active_pane(
        &mut self,
        new_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if new_id == self.workspace.workspace.active_pane_id {
            self.workspace.workspace.active_pane.terminal_ime_anchor = None;
            self.workspace
                .workspace
                .active_pane
                .terminal_focus
                .focus(window, cx);
            self.sync_terminal_focus_reporting(window, cx);
            self.sync_window_overlays(window, cx);
            return;
        }
        self.clear_terminal_originated_selection_drag(cx);
        // Tabs are global; only per-pane state is parked.
        let outgoing = self.workspace.workspace.active_pane_id;
        let parked = ParkedPane {
            active_tab: self.workspace.workspace.active_tab.take(),
            terminal_focus: std::mem::replace(
                &mut self.workspace.workspace.active_pane.terminal_focus,
                cx.focus_handle(),
            ),
            terminal_bounds: self.workspace.workspace.active_pane.terminal_bounds.take(),
            terminal_cell_width: self.workspace.workspace.active_pane.terminal_cell_width,
            terminal_line_height: self.workspace.workspace.active_pane.terminal_line_height,
            terminal_mouse_gesture: self
                .workspace
                .workspace
                .active_pane
                .terminal_mouse_gesture
                .take(),
            terminal_keyboard: std::mem::take(
                &mut self.workspace.workspace.active_pane.terminal_keyboard,
            ),
            terminal_mouse_reporting: std::mem::take(
                &mut self
                    .workspace
                    .workspace
                    .active_pane
                    .terminal_mouse_reporting,
            ),
            terminal_pointer_target: self
                .workspace
                .workspace
                .active_pane
                .terminal_pointer_target
                .take(),
            terminal_pointer_position: self
                .workspace
                .workspace
                .active_pane
                .terminal_pointer_position
                .take(),
            terminal_link_query: self
                .workspace
                .workspace
                .active_pane
                .terminal_link_query
                .take(),
            terminal_hovered_link: self
                .workspace
                .workspace
                .active_pane
                .terminal_hovered_link
                .take(),
            terminal_link_open_modifier: std::mem::take(
                &mut self
                    .workspace
                    .workspace
                    .active_pane
                    .terminal_link_open_modifier,
            ),
            terminal_scrollbar_drag: self
                .workspace
                .workspace
                .active_pane
                .terminal_scrollbar_drag
                .take(),
            terminal_scrollbar_last_interaction_at: self
                .workspace
                .workspace
                .active_pane
                .terminal_scrollbar_last_interaction_at
                .take(),
        };
        self.workspace
            .workspace
            .parked_panes
            .insert(outgoing, parked);

        if let Some(incoming) = self.workspace.workspace.parked_panes.remove(&new_id) {
            self.workspace.workspace.active_tab = incoming.active_tab;
            self.workspace.workspace.active_pane.terminal_focus = incoming.terminal_focus;
            self.workspace.workspace.active_pane.terminal_bounds = incoming.terminal_bounds;
            self.workspace.workspace.active_pane.terminal_cell_width = incoming.terminal_cell_width;
            self.workspace.workspace.active_pane.terminal_line_height =
                incoming.terminal_line_height;
            self.workspace.workspace.active_pane.terminal_mouse_gesture =
                incoming.terminal_mouse_gesture;
            self.workspace.workspace.active_pane.terminal_keyboard = incoming.terminal_keyboard;
            self.workspace
                .workspace
                .active_pane
                .terminal_mouse_reporting = incoming.terminal_mouse_reporting;
            self.workspace.workspace.active_pane.terminal_pointer_target =
                incoming.terminal_pointer_target;
            self.workspace
                .workspace
                .active_pane
                .terminal_pointer_position = incoming.terminal_pointer_position;
            self.workspace.workspace.active_pane.terminal_link_query = incoming.terminal_link_query;
            self.workspace.workspace.active_pane.terminal_hovered_link =
                incoming.terminal_hovered_link;
            self.workspace
                .workspace
                .active_pane
                .terminal_link_open_modifier = incoming.terminal_link_open_modifier;
            self.workspace.workspace.active_pane.terminal_scrollbar_drag =
                incoming.terminal_scrollbar_drag;
            self.workspace
                .workspace
                .active_pane
                .terminal_scrollbar_last_interaction_at =
                incoming.terminal_scrollbar_last_interaction_at;
        }
        self.workspace.workspace.active_pane_id = new_id;
        self.workspace.workspace.active_pane.terminal_ime_anchor = None;
        if self.controllers.session.read(cx).side_panel_open()
            && self.controllers.session.read(cx).side_panel_view() == SessionSidePanelView::Sftp
            && let Some(session_tab_id) = self
                .active_terminal_session_index(cx)
                .and_then(|index| self.workspace.tabs.at(index))
                .map(|tab| tab.id)
        {
            self.ensure_session_side_panel_sftp_tab(session_tab_id, cx);
        }
        self.rebind_terminal_focus_reporting(window, cx);
        self.workspace
            .workspace
            .active_pane
            .terminal_focus
            .focus(window, cx);
        self.sync_terminal_focus_reporting(window, cx);
        self.sync_session_port_snapshot(cx);
        self.sync_window_overlays(window, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn split_active_pane(
        &mut self,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let outgoing_id = self.workspace.workspace.active_pane_id;
        let new_pane_id = self.allocate_pane_id();
        let active_profile = self.active_profile(cx);

        // Tabs are global; only per-pane state is parked.
        let parked = ParkedPane {
            active_tab: self.workspace.workspace.active_tab.take(),
            terminal_focus: std::mem::replace(
                &mut self.workspace.workspace.active_pane.terminal_focus,
                cx.focus_handle(),
            ),
            terminal_bounds: self.workspace.workspace.active_pane.terminal_bounds.take(),
            terminal_cell_width: self.workspace.workspace.active_pane.terminal_cell_width,
            terminal_line_height: self.workspace.workspace.active_pane.terminal_line_height,
            terminal_mouse_gesture: self
                .workspace
                .workspace
                .active_pane
                .terminal_mouse_gesture
                .take(),
            terminal_keyboard: std::mem::take(
                &mut self.workspace.workspace.active_pane.terminal_keyboard,
            ),
            terminal_mouse_reporting: std::mem::take(
                &mut self
                    .workspace
                    .workspace
                    .active_pane
                    .terminal_mouse_reporting,
            ),
            terminal_pointer_target: self
                .workspace
                .workspace
                .active_pane
                .terminal_pointer_target
                .take(),
            terminal_pointer_position: self
                .workspace
                .workspace
                .active_pane
                .terminal_pointer_position
                .take(),
            terminal_link_query: self
                .workspace
                .workspace
                .active_pane
                .terminal_link_query
                .take(),
            terminal_hovered_link: self
                .workspace
                .workspace
                .active_pane
                .terminal_hovered_link
                .take(),
            terminal_link_open_modifier: std::mem::take(
                &mut self
                    .workspace
                    .workspace
                    .active_pane
                    .terminal_link_open_modifier,
            ),
            terminal_scrollbar_drag: self
                .workspace
                .workspace
                .active_pane
                .terminal_scrollbar_drag
                .take(),
            terminal_scrollbar_last_interaction_at: self
                .workspace
                .workspace
                .active_pane
                .terminal_scrollbar_last_interaction_at
                .take(),
        };
        self.workspace
            .workspace
            .parked_panes
            .insert(outgoing_id, parked);
        self.workspace.workspace.active_pane.terminal_bounds = None;
        self.workspace.workspace.active_pane.terminal_cell_width = terminal_cell_width_default();
        self.workspace.workspace.active_pane.terminal_line_height = terminal_line_height_default();
        self.workspace.workspace.active_pane.terminal_ime_anchor = None;
        self.workspace.workspace.pane_split_drag = None;

        let split_animation =
            self.workspace
                .workspace
                .pane_layout
                .split(outgoing_id, direction, new_pane_id);
        if split_animation.is_none() {
            self.workspace.workspace.pane_layout = PaneLayout::Leaf(new_pane_id);
            self.workspace.workspace.pane_split_animation = None;
        } else if let Some(animation) = split_animation {
            let _ = self.set_split_flex_pair(
                &animation.path,
                animation.child_index,
                animation.from_flex_a,
                animation.from_flex_b,
            );
            self.workspace.workspace.pane_split_animation = Some(PaneSplitAnimation {
                kind: PaneSplitAnimationKind::Opening,
                path: animation.path,
                child_index: animation.child_index,
                new_child_index: animation.new_child_index,
                axis: animation.axis,
                from_flex_a: animation.from_flex_a,
                from_flex_b: animation.from_flex_b,
                to_flex_a: animation.to_flex_a,
                to_flex_b: animation.to_flex_b,
                started_at: Instant::now(),
                duration: support::CONTAINER_TRANSITION_DURATION,
                pending_close: None,
            });
        }
        self.workspace.workspace.active_pane_id = new_pane_id;
        self.rebind_terminal_focus_reporting(window, cx);

        if let Some(profile) = active_profile {
            let (mut tab, session) = self.build_session_tab(profile, cx);
            tab.placement = self
                .workspace
                .active_topbar_tab
                .map(|owner| TabPlacement::WorkspacePane {
                    owner,
                    pane: new_pane_id,
                })
                .unwrap_or(TabPlacement::Background);
            self.insert_session_tab(tab, session, cx);
            self.workspace.workspace.active_tab =
                self.workspace.tabs.id_at(self.workspace.tabs.len() - 1);
        }
        self.workspace
            .workspace
            .active_pane
            .terminal_focus
            .focus(window, cx);
        self.sync_terminal_focus_reporting(window, cx);
        self.sync_session_port_snapshot(cx);
        self.sync_window_overlays(window, cx);
        self.schedule_pane_split_animation(window, cx);
        cx.notify();
    }

    /// Tabs are global, so closing a pane usually leaves the underlying tabs alive.
    pub(in crate::ui::shell) fn close_active_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.workspace.workspace.pane_layout, PaneLayout::Leaf(id) if id == self.workspace.workspace.active_pane_id)
            && self.workspace.workspace.parked_panes.is_empty()
        {
            if let Some(active) = self.workspace.workspace.active_tab
                && let Some(index) = self.workspace.tabs.index_of(active)
            {
                self.close_tab(index, window, cx);
            }
            return;
        }

        let removed_id = self.workspace.workspace.active_pane_id;
        let hidden_tab_id = self
            .workspace
            .workspace
            .active_tab
            .and_then(|index| self.workspace.tabs.get(index))
            .and_then(|tab| (!tab.is_top_level()).then_some(tab.id));

        if let Some(animation) = self
            .workspace
            .workspace
            .pane_layout
            .close_animation_target(removed_id)
        {
            self.workspace.workspace.pane_split_drag = None;
            let _ = self.set_split_flex_pair(
                &animation.path,
                animation.child_index,
                animation.from_flex_a,
                animation.from_flex_b,
            );
            self.workspace.workspace.pane_split_animation = Some(PaneSplitAnimation {
                kind: PaneSplitAnimationKind::Closing,
                path: animation.path,
                child_index: animation.child_index,
                new_child_index: animation.new_child_index,
                axis: animation.axis,
                from_flex_a: animation.from_flex_a,
                from_flex_b: animation.from_flex_b,
                to_flex_a: animation.to_flex_a,
                to_flex_b: animation.to_flex_b,
                started_at: Instant::now(),
                duration: support::CONTAINER_TRANSITION_DURATION,
                pending_close: Some(PaneCloseAnimation {
                    removed_pane_id: removed_id,
                    hidden_tab_id,
                }),
            });
            self.schedule_pane_split_animation(window, cx);
            cx.notify();
            return;
        }

        // Tabs created only for split panes are hidden from the top bar and
        // should be torn down with that pane.
        if let Some(active_idx) = self.workspace.workspace.active_tab.take()
            && let Some(tab) = self.workspace.tabs.get(active_idx)
            && !tab.is_top_level()
            && let Some(index) = self.workspace.tabs.index_of(active_idx)
        {
            self.close_tab(index, window, cx);
        }

        let removed_id = self.workspace.workspace.active_pane_id;
        self.workspace.workspace.pane_layout.remove(removed_id);

        let next_id = self.workspace.workspace.pane_layout.first_leaf();
        // set_active_pane swaps the current AppView fields through parked_panes,
        // so it needs a temporary entry here.
        self.workspace.workspace.parked_panes.insert(
            removed_id,
            ParkedPane::empty(self.workspace.workspace.active_pane.terminal_focus.clone()),
        );
        self.set_active_pane(next_id, window, cx);
        self.workspace.workspace.parked_panes.remove(&removed_id);
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_split_flex_pair(
        &mut self,
        path: &[usize],
        child_index: usize,
        flex_a: f32,
        flex_b: f32,
    ) -> bool {
        let mut node = &mut self.workspace.workspace.pane_layout;
        for &i in path {
            match node {
                PaneLayout::Split { children, .. } => {
                    let Some(next) = children.get_mut(i) else {
                        return false;
                    };
                    node = next;
                }
                _ => return false,
            }
        }
        if let PaneLayout::Split { flexes, .. } = node {
            let Some(slot_a) = flexes.get_mut(child_index) else {
                return false;
            };
            *slot_a = flex_a.max(0.001);
            let Some(slot_b) = flexes.get_mut(child_index + 1) else {
                return false;
            };
            *slot_b = flex_b.max(0.001);
            return true;
        }

        false
    }

    pub(in crate::ui::shell) fn apply_split_flex_delta(
        &mut self,
        path: &[usize],
        child_index: usize,
        new_flex_a: f32,
        new_flex_b: f32,
    ) {
        let _ = self.set_split_flex_pair(
            path,
            child_index,
            new_flex_a.clamp(0.05, 0.95),
            new_flex_b.clamp(0.05, 0.95),
        );
    }

    fn finish_pane_close_animation(
        &mut self,
        close: PaneCloseAnimation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self
            .workspace
            .workspace
            .pane_layout
            .remove(close.removed_pane_id)
        {
            return;
        }

        if self.workspace.workspace.active_pane_id == close.removed_pane_id {
            let next_id = self.workspace.workspace.pane_layout.first_leaf();
            self.set_active_pane(next_id, window, cx);
        }
        self.workspace
            .workspace
            .parked_panes
            .remove(&close.removed_pane_id);

        if let Some(hidden_tab_id) = close.hidden_tab_id
            && let Some(index) = self.workspace.tabs.index_of(hidden_tab_id)
        {
            self.close_tab(index, window, cx);
        }
    }

    pub(in crate::ui::shell) fn schedule_pane_split_animation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shell.pane_animation_frame_pending {
            return;
        }
        let Some(started_at) = self
            .workspace
            .workspace
            .pane_split_animation
            .as_ref()
            .map(|animation| animation.started_at)
        else {
            return;
        };
        self.shell.pane_animation_frame_pending = true;
        window.request_animation_frame();
        cx.on_next_frame(window, move |this, window, cx| {
            this.shell.pane_animation_frame_pending = false;
            if this
                .workspace
                .workspace
                .pane_split_animation
                .as_ref()
                .is_some_and(|animation| animation.started_at == started_at)
            {
                this.advance_pane_split_animation(window, cx);
                cx.notify();
            }
            this.schedule_pane_split_animation(window, cx);
        });
    }

    fn advance_pane_split_animation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace.workspace.pane_split_drag.is_some() {
            self.workspace.workspace.pane_split_animation = None;
            return;
        }

        let Some(animation) = self.workspace.workspace.pane_split_animation.clone() else {
            return;
        };

        let duration_seconds = animation.duration.as_secs_f32();
        if duration_seconds <= f32::EPSILON {
            self.workspace.workspace.pane_split_animation = None;
            if let Some(close) = animation.pending_close {
                self.finish_pane_close_animation(close, window, cx);
            } else {
                let _ = self.set_split_flex_pair(
                    &animation.path,
                    animation.child_index,
                    animation.to_flex_a,
                    animation.to_flex_b,
                );
            }
            return;
        }

        let elapsed = Instant::now().saturating_duration_since(animation.started_at);
        let progress = (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0);
        let eased = progress * progress * (3.0 - 2.0 * progress);
        let current_flex_a =
            animation.from_flex_a + (animation.to_flex_a - animation.from_flex_a) * eased;
        let current_flex_b =
            animation.from_flex_b + (animation.to_flex_b - animation.from_flex_b) * eased;

        if !self.set_split_flex_pair(
            &animation.path,
            animation.child_index,
            current_flex_a,
            current_flex_b,
        ) {
            self.workspace.workspace.pane_split_animation = None;
            return;
        }

        if progress >= 1.0 {
            self.workspace.workspace.pane_split_animation = None;
            if let Some(close) = animation.pending_close {
                self.finish_pane_close_animation(close, window, cx);
            } else {
                let _ = self.set_split_flex_pair(
                    &animation.path,
                    animation.child_index,
                    animation.to_flex_a,
                    animation.to_flex_b,
                );
            }
            return;
        }

        window.request_animation_frame();
    }

    pub(in crate::ui::shell) fn split_container_size(
        &self,
        path: &[usize],
        axis: SplitAxis,
        window: &Window,
    ) -> f32 {
        let bounds = window.bounds().size;
        let mut available = match axis {
            SplitAxis::Horizontal => f32::from(bounds.width),
            SplitAxis::Vertical => {
                f32::from(bounds.height) - top_bar_height() - FOOTER_HEIGHT - TERMINAL_PANEL_BORDER
            }
        }
        .max(1.0);

        let mut node = &self.workspace.workspace.pane_layout;
        for &i in path {
            match node {
                PaneLayout::Split {
                    axis: node_axis,
                    children,
                    flexes,
                } => {
                    if *node_axis == axis
                        && let Some(flex) = flexes.get(i)
                    {
                        available *= flex;
                    }
                    let Some(next) = children.get(i) else {
                        return available;
                    };
                    node = next;
                }
                _ => return available,
            }
        }
        available
    }

    pub(in crate::ui::shell) fn active_profile(&self, cx: &App) -> Option<SessionProfile> {
        let profile_id = self
            .workspace
            .workspace
            .active_tab
            .and_then(|tab_id| self.session_tab(tab_id, cx))
            .map(|session| session.profile_id().to_string())
            .or_else(|| {
                self.workspace
                    .active_topbar_tab
                    .and_then(|tab_id| self.controllers.sftp.read(cx).profile_id(tab_id))
            })?;
        self.controllers
            .session
            .read(cx)
            .profiles()
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
    }

    pub(in crate::ui::shell) fn has_active_session(&self) -> bool {
        self.workspace
            .workspace
            .active_tab
            .and_then(|tab_id| self.workspace.tabs.get(tab_id))
            .is_some_and(|tab| tab.is_session())
    }

    pub(in crate::ui::shell) fn active_terminal_session_index(&self, cx: &App) -> Option<usize> {
        let tab_id = self
            .workspace
            .workspace
            .active_tab
            .filter(|&tab_id| {
                self.session_tab(tab_id, cx)
                    .is_some_and(|session| session.purpose() == SessionPurpose::Terminal)
            })
            .or_else(|| {
                self.workspace.active_topbar_tab.filter(|&tab_id| {
                    self.session_tab(tab_id, cx)
                        .is_some_and(|session| session.purpose() == SessionPurpose::Terminal)
                })
            })?;
        self.workspace.tabs.index_of(tab_id)
    }

    pub(in crate::ui::shell) fn toggle_session_side_panel(&mut self, cx: &App) {
        self.controllers.session.read(cx).toggle_side_panel();
    }

    pub(in crate::ui::shell) fn toggle_session_agent_panel(&mut self, cx: &mut Context<Self>) {
        self.controllers.agent.update(cx, |controller, cx| {
            if controller.panel_open() {
                controller.finish_text_drag(cx);
            }
            controller.toggle_panel();
            cx.notify();
        });
    }

    pub(in crate::ui::shell) fn active_tab_is_hosts(&self) -> bool {
        self.workspace
            .active_topbar_tab
            .and_then(|index| self.workspace.tabs.get(index))
            .is_some_and(|tab| tab.is_hosts())
    }

    pub(in crate::ui::shell) fn active_username(&self, cx: &App) -> String {
        if let Some(profile) = self.active_profile(cx)
            && !profile.username.trim().is_empty()
        {
            return profile.username.clone();
        }

        std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "admin".into())
    }

    pub(in crate::ui::shell) fn window_title(&self, cx: &App) -> String {
        let active_index = self
            .workspace
            .workspace
            .active_tab
            .or(self.workspace.active_topbar_tab);

        let Some(active_index) = active_index else {
            return APP_TITLE.to_string();
        };

        let Some(tab) = self.workspace.tabs.get(active_index) else {
            return APP_TITLE.to_string();
        };

        if tab.is_hosts() {
            return APP_TITLE.to_string();
        }

        let title = tab.title.trim();
        if !title.is_empty() {
            return format!("{title} - {APP_TITLE}");
        }

        self.active_profile(cx)
            .map(|profile| format!("{} - {APP_TITLE}", profile.summary()))
            .unwrap_or_else(|| APP_TITLE.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_download_prompts_only_when_target_is_the_session_sftp_tab() {
        let target = TabId::new(4);
        assert!(should_prompt_terminal_sftp_download(
            Some(TabId::new(2)),
            Some(target),
            target,
        ));
        assert!(!should_prompt_terminal_sftp_download(
            Some(target),
            Some(target),
            target,
        ));
        assert!(!should_prompt_terminal_sftp_download(
            Some(TabId::new(2)),
            Some(TabId::new(3)),
            target,
        ));
    }
}
