use super::state::{
    PendingAiProviderPopupState, PendingLocalDataResetConfirmState,
    PendingLocalDataResetConfirmationPopupState, PendingSyncPassphraseClearConfirmPopupState,
};
use miaominal_core::keychain::ManagedKeySource;

use super::*;

pub struct AppView {
    pub(in crate::ui::shell) services: AppServices,
    pub(in crate::ui::shell) data: AppDataState,
    pub(in crate::ui::shell) host_editor_forms: HostEditorForms,
    pub(in crate::ui::shell) workspace_forms: WorkspaceForms,
    pub(in crate::ui::shell) panel_forms: PanelForms,
    pub(in crate::ui::shell) keychain_page_view: KeychainPageView,
    pub(in crate::ui::shell) keychain_editor_mode: KeychainEditorMode,
    pub(in crate::ui::shell) keychain_deploy_in_progress: bool,
    pub(in crate::ui::shell) keychain_editor_draft_source: Option<ManagedKeySource>,
    pub(in crate::ui::shell) keychain_deploy_key_id: Option<String>,
    pub(in crate::ui::shell) workspace_state: WorkspaceState,
    pub(in crate::ui::shell) session_agent_focus: FocusHandle,
    pub(in crate::ui::shell) panel_view: PanelViewState,
    pub(in crate::ui::shell) editors: EditorOverlayState,
    pub(in crate::ui::shell) shell_state: ShellState,
    pub(in crate::ui::shell) panels: PanelState,
    pub(in crate::ui::shell) session_agent: SessionAgentState,
    pub(in crate::ui::shell) session_agent_sessions: HashMap<String, SessionAgentState>,
    pub(in crate::ui::shell) kbi_inputs: Vec<Entity<InputState>>,
    pub(in crate::ui::shell) dialogs: DialogState,
    pub(in crate::ui::shell) onboarding: OnboardingState,
    pub(in crate::ui::shell) status_message: String,
    pub(in crate::ui::shell) settings_store: SettingsStore,
    pub(in crate::ui::shell) local_vault_status: LocalVaultStatus,
    pub(in crate::ui::shell) sync_passphrase_popup: Option<PendingSyncPassphrasePopupState>,
    pub(in crate::ui::shell) ai_provider_popup: Option<PendingAiProviderPopupState>,
    pub(in crate::ui::shell) local_vault_passphrase_popup: Option<LocalVaultPassphrasePopupMode>,
    pub(in crate::ui::shell) pending_local_vault_unlock_action:
        Option<PendingLocalVaultUnlockAction>,
    pub(in crate::ui::shell) local_vault_unlock_in_progress: bool,
    pub(in crate::ui::shell) local_vault_disable_in_progress: bool,
    pub(in crate::ui::shell) local_data_reset_in_progress: bool,
    pub(in crate::ui::shell) ai_provider_save_in_progress: bool,
    pub(in crate::ui::shell) web_search_save_in_progress: bool,
    pub(in crate::ui::shell) ai_provider_api_key_load_in_progress: Option<String>,
    pub(in crate::ui::shell) local_vault_session_passphrase: Option<String>,
    pub(in crate::ui::shell) local_vault_auto_lock_task: Option<gpui::Task<()>>,
    pub(in crate::ui::shell) sync: SyncUiState,
    pub(in crate::ui::shell) secret_visibility: SecretVisibilityState,
    #[allow(dead_code)]
    pub(in crate::ui::shell) controllers: ControllerSet,
    pub(in crate::ui::shell) _subscriptions: AppViewSubscriptions,
}

impl AppView {
    pub(in crate::ui::shell) fn start_hosts_to_terminal_transition(
        &mut self,
        active_tab_id: usize,
        terminal_tab_id: usize,
        direction: HostsToTerminalTransitionDirection,
        show_host_editor_sidebar: bool,
    ) {
        self.workspace_state.hosts_to_terminal_transition = Some(HostsToTerminalTransition {
            started_at: Instant::now(),
            duration: support::OVERLAY_ENTER_DURATION,
            active_tab_id,
            terminal_tab_id,
            direction,
            show_host_editor_sidebar,
        });
    }
}

impl AppView {
    pub(in crate::ui::shell) fn set_active_pane(
        &mut self,
        new_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if new_id == self.workspace_state.workspace.active_pane_id {
            self.workspace_state
                .workspace
                .active_pane
                .terminal_focus
                .focus(window, cx);
            self.sync_terminal_focus_reporting(window, cx);
            return;
        }
        // Tabs are global; only per-pane state is parked.
        let outgoing = self.workspace_state.workspace.active_pane_id;
        let parked = ParkedPane {
            active_tab: self.workspace_state.workspace.active_tab.take(),
            terminal_focus: std::mem::replace(
                &mut self.workspace_state.workspace.active_pane.terminal_focus,
                cx.focus_handle(),
            ),
            terminal_bounds: self
                .workspace_state
                .workspace
                .active_pane
                .terminal_bounds
                .take(),
            terminal_cell_width: self
                .workspace_state
                .workspace
                .active_pane
                .terminal_cell_width,
            terminal_line_height: self
                .workspace_state
                .workspace
                .active_pane
                .terminal_line_height,
            terminal_dragging: std::mem::take(
                &mut self.workspace_state.workspace.active_pane.terminal_dragging,
            ),
            terminal_mouse_reporting_active: std::mem::take(
                &mut self
                    .workspace_state
                    .workspace
                    .active_pane
                    .terminal_mouse_reporting_active,
            ),
            last_reported_mouse_cell: self
                .workspace_state
                .workspace
                .active_pane
                .last_reported_mouse_cell
                .take(),
            terminal_pointer_position: self
                .workspace_state
                .workspace
                .active_pane
                .terminal_pointer_position
                .take(),
            terminal_hovered_link: self
                .workspace_state
                .workspace
                .active_pane
                .terminal_hovered_link
                .take(),
            terminal_link_open_modifier: std::mem::take(
                &mut self
                    .workspace_state
                    .workspace
                    .active_pane
                    .terminal_link_open_modifier,
            ),
            terminal_scrollbar_drag: self
                .workspace_state
                .workspace
                .active_pane
                .terminal_scrollbar_drag
                .take(),
            terminal_scrollbar_last_interaction_at: self
                .workspace_state
                .workspace
                .active_pane
                .terminal_scrollbar_last_interaction_at
                .take(),
        };
        self.workspace_state
            .workspace
            .parked_panes
            .insert(outgoing, parked);

        if let Some(incoming) = self.workspace_state.workspace.parked_panes.remove(&new_id) {
            self.workspace_state.workspace.active_tab = incoming.active_tab;
            self.workspace_state.workspace.active_pane.terminal_focus = incoming.terminal_focus;
            self.workspace_state.workspace.active_pane.terminal_bounds = incoming.terminal_bounds;
            self.workspace_state
                .workspace
                .active_pane
                .terminal_cell_width = incoming.terminal_cell_width;
            self.workspace_state
                .workspace
                .active_pane
                .terminal_line_height = incoming.terminal_line_height;
            self.workspace_state.workspace.active_pane.terminal_dragging =
                incoming.terminal_dragging;
            self.workspace_state
                .workspace
                .active_pane
                .terminal_mouse_reporting_active = incoming.terminal_mouse_reporting_active;
            self.workspace_state
                .workspace
                .active_pane
                .last_reported_mouse_cell = incoming.last_reported_mouse_cell;
            self.workspace_state
                .workspace
                .active_pane
                .terminal_pointer_position = incoming.terminal_pointer_position;
            self.workspace_state
                .workspace
                .active_pane
                .terminal_hovered_link = incoming.terminal_hovered_link;
            self.workspace_state
                .workspace
                .active_pane
                .terminal_link_open_modifier = incoming.terminal_link_open_modifier;
            self.workspace_state
                .workspace
                .active_pane
                .terminal_scrollbar_drag = incoming.terminal_scrollbar_drag;
            self.workspace_state
                .workspace
                .active_pane
                .terminal_scrollbar_last_interaction_at =
                incoming.terminal_scrollbar_last_interaction_at;
        }
        self.workspace_state.workspace.active_pane_id = new_id;
        self.rebind_terminal_focus_reporting(window, cx);
        self.workspace_state
            .workspace
            .active_pane
            .terminal_focus
            .focus(window, cx);
        self.sync_terminal_focus_reporting(window, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn split_active_pane(
        &mut self,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let outgoing_id = self.workspace_state.workspace.active_pane_id;
        let new_pane_id = self.allocate_pane_id();
        let active_profile = self.active_profile().cloned();

        // Tabs are global; only per-pane state is parked.
        let parked = ParkedPane {
            active_tab: self.workspace_state.workspace.active_tab.take(),
            terminal_focus: std::mem::replace(
                &mut self.workspace_state.workspace.active_pane.terminal_focus,
                cx.focus_handle(),
            ),
            terminal_bounds: self
                .workspace_state
                .workspace
                .active_pane
                .terminal_bounds
                .take(),
            terminal_cell_width: self
                .workspace_state
                .workspace
                .active_pane
                .terminal_cell_width,
            terminal_line_height: self
                .workspace_state
                .workspace
                .active_pane
                .terminal_line_height,
            terminal_dragging: std::mem::take(
                &mut self.workspace_state.workspace.active_pane.terminal_dragging,
            ),
            terminal_mouse_reporting_active: std::mem::take(
                &mut self
                    .workspace_state
                    .workspace
                    .active_pane
                    .terminal_mouse_reporting_active,
            ),
            last_reported_mouse_cell: self
                .workspace_state
                .workspace
                .active_pane
                .last_reported_mouse_cell
                .take(),
            terminal_pointer_position: self
                .workspace_state
                .workspace
                .active_pane
                .terminal_pointer_position
                .take(),
            terminal_hovered_link: self
                .workspace_state
                .workspace
                .active_pane
                .terminal_hovered_link
                .take(),
            terminal_link_open_modifier: std::mem::take(
                &mut self
                    .workspace_state
                    .workspace
                    .active_pane
                    .terminal_link_open_modifier,
            ),
            terminal_scrollbar_drag: self
                .workspace_state
                .workspace
                .active_pane
                .terminal_scrollbar_drag
                .take(),
            terminal_scrollbar_last_interaction_at: self
                .workspace_state
                .workspace
                .active_pane
                .terminal_scrollbar_last_interaction_at
                .take(),
        };
        self.workspace_state
            .workspace
            .parked_panes
            .insert(outgoing_id, parked);
        self.workspace_state.workspace.active_pane.terminal_bounds = None;
        self.workspace_state
            .workspace
            .active_pane
            .terminal_cell_width = terminal_cell_width_default();
        self.workspace_state
            .workspace
            .active_pane
            .terminal_line_height = terminal_line_height_default();
        self.workspace_state.workspace.pane_split_drag = None;

        let split_animation =
            self.workspace_state
                .workspace
                .pane_layout
                .split(outgoing_id, direction, new_pane_id);
        if split_animation.is_none() {
            self.workspace_state.workspace.pane_layout = PaneLayout::Leaf(new_pane_id);
            self.workspace_state.workspace.pane_split_animation = None;
        } else if let Some(animation) = split_animation {
            let _ = self.set_split_flex_pair(
                &animation.path,
                animation.child_index,
                animation.from_flex_a,
                animation.from_flex_b,
            );
            self.workspace_state.workspace.pane_split_animation = Some(PaneSplitAnimation {
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
        self.workspace_state.workspace.active_pane_id = new_pane_id;
        self.rebind_terminal_focus_reporting(window, cx);

        if let Some(profile) = active_profile {
            let mut tab = self.build_session_tab(profile);
            tab.hidden_from_topbar = true;
            self.workspace_state.tabs.push(tab);
            self.workspace_state.workspace.active_tab = Some(self.workspace_state.tabs.len() - 1);
        }
        self.workspace_state
            .workspace
            .active_pane
            .terminal_focus
            .focus(window, cx);
        self.sync_terminal_focus_reporting(window, cx);
        cx.notify();
    }

    /// Tabs are global, so closing a pane usually leaves the underlying tabs alive.
    pub(in crate::ui::shell) fn close_active_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.workspace_state.workspace.pane_layout, PaneLayout::Leaf(id) if id == self.workspace_state.workspace.active_pane_id)
            && self.workspace_state.workspace.parked_panes.is_empty()
        {
            if let Some(active) = self.workspace_state.workspace.active_tab {
                self.close_tab(active, window, cx);
            }
            return;
        }

        let removed_id = self.workspace_state.workspace.active_pane_id;
        let hidden_tab_id = self
            .workspace_state
            .workspace
            .active_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(|tab| tab.hidden_from_topbar.then_some(tab.id));

        if let Some(animation) = self
            .workspace_state
            .workspace
            .pane_layout
            .close_animation_target(removed_id)
        {
            self.workspace_state.workspace.pane_split_drag = None;
            let _ = self.set_split_flex_pair(
                &animation.path,
                animation.child_index,
                animation.from_flex_a,
                animation.from_flex_b,
            );
            self.workspace_state.workspace.pane_split_animation = Some(PaneSplitAnimation {
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
            cx.notify();
            return;
        }

        // Tabs created only for split panes are hidden from the top bar and
        // should be torn down with that pane.
        if let Some(active_idx) = self.workspace_state.workspace.active_tab.take()
            && let Some(tab) = self.workspace_state.tabs.get(active_idx)
            && tab.hidden_from_topbar
        {
            self.close_tab(active_idx, window, cx);
        }

        let removed_id = self.workspace_state.workspace.active_pane_id;
        self.workspace_state
            .workspace
            .pane_layout
            .remove(removed_id);

        let next_id = self.workspace_state.workspace.pane_layout.first_leaf();
        // set_active_pane swaps the current AppView fields through parked_panes,
        // so it needs a temporary entry here.
        self.workspace_state.workspace.parked_panes.insert(
            removed_id,
            ParkedPane::empty(
                self.workspace_state
                    .workspace
                    .active_pane
                    .terminal_focus
                    .clone(),
            ),
        );
        self.set_active_pane(next_id, window, cx);
        self.workspace_state
            .workspace
            .parked_panes
            .remove(&removed_id);
        cx.notify();
    }

    pub(in crate::ui::shell) fn write_pane_terminal_metrics(
        &mut self,
        pane_id: PaneId,
        bounds: Bounds<Pixels>,
        cell_width: f32,
        line_height: f32,
        cx: &mut Context<Self>,
    ) {
        if pane_id == self.workspace_state.workspace.active_pane_id {
            let changed = self.workspace_state.workspace.active_pane.terminal_bounds
                != Some(bounds)
                || self
                    .workspace_state
                    .workspace
                    .active_pane
                    .terminal_cell_width
                    != cell_width
                || self
                    .workspace_state
                    .workspace
                    .active_pane
                    .terminal_line_height
                    != line_height;
            self.workspace_state.workspace.active_pane.terminal_bounds = Some(bounds);
            self.workspace_state
                .workspace
                .active_pane
                .terminal_cell_width = cell_width;
            self.workspace_state
                .workspace
                .active_pane
                .terminal_line_height = line_height;
            if changed {
                cx.notify();
            }
        } else if let Some(parked) = self
            .workspace_state
            .workspace
            .parked_panes
            .get_mut(&pane_id)
        {
            let changed = parked.terminal_bounds != Some(bounds)
                || parked.terminal_cell_width != cell_width
                || parked.terminal_line_height != line_height;
            let tab_index = parked.active_tab;
            parked.terminal_bounds = Some(bounds);
            parked.terminal_cell_width = cell_width;
            parked.terminal_line_height = line_height;

            let resized = tab_index.is_some_and(|index| {
                self.sync_session_terminal_size_from_metrics(
                    index,
                    bounds,
                    cell_width,
                    line_height,
                    !changed,
                    cx,
                )
            });

            if changed || resized {
                cx.notify();
            }
        }
    }

    pub(in crate::ui::shell) fn set_split_flex_pair(
        &mut self,
        path: &[usize],
        child_index: usize,
        flex_a: f32,
        flex_b: f32,
    ) -> bool {
        let mut node = &mut self.workspace_state.workspace.pane_layout;
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
            .workspace_state
            .workspace
            .pane_layout
            .remove(close.removed_pane_id)
        {
            return;
        }

        let next_id = self.workspace_state.workspace.pane_layout.first_leaf();
        self.workspace_state.workspace.parked_panes.insert(
            close.removed_pane_id,
            ParkedPane::empty(
                self.workspace_state
                    .workspace
                    .active_pane
                    .terminal_focus
                    .clone(),
            ),
        );
        self.set_active_pane(next_id, window, cx);
        self.workspace_state
            .workspace
            .parked_panes
            .remove(&close.removed_pane_id);

        if let Some(hidden_tab_id) = close.hidden_tab_id
            && let Some(index) = self
                .workspace_state
                .tabs
                .iter()
                .position(|tab| tab.id == hidden_tab_id)
        {
            self.close_tab(index, window, cx);
        }
    }

    pub(in crate::ui::shell) fn advance_pane_split_animation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace_state.workspace.pane_split_drag.is_some() {
            self.workspace_state.workspace.pane_split_animation = None;
            return;
        }

        let Some(animation) = self.workspace_state.workspace.pane_split_animation.clone() else {
            return;
        };

        let duration_seconds = animation.duration.as_secs_f32();
        if duration_seconds <= f32::EPSILON {
            self.workspace_state.workspace.pane_split_animation = None;
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
            self.workspace_state.workspace.pane_split_animation = None;
            return;
        }

        if progress >= 1.0 {
            self.workspace_state.workspace.pane_split_animation = None;
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
                f32::from(bounds.height) - TOP_BAR_HEIGHT - FOOTER_HEIGHT - TERMINAL_PANEL_BORDER
            }
        }
        .max(1.0);

        let mut node = &self.workspace_state.workspace.pane_layout;
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

    pub(in crate::ui::shell) fn active_profile(&self) -> Option<&SessionProfile> {
        let profile_id = self
            .workspace_state
            .workspace
            .active_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(TabState::as_session)
            .map(|session| session.profile_id.as_str())
            .or_else(|| {
                self.workspace_state
                    .active_topbar_tab
                    .and_then(|index| self.workspace_state.tabs.get(index))
                    .and_then(TabState::as_sftp)
                    .map(|sftp| sftp.profile_id.as_str())
            })?;
        self.data
            .sessions
            .iter()
            .find(|profile| profile.id == profile_id)
    }

    pub(in crate::ui::shell) fn has_active_session(&self) -> bool {
        self.workspace_state
            .workspace
            .active_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(TabState::as_session)
            .is_some()
    }

    pub(in crate::ui::shell) fn active_terminal_session_index(&self) -> Option<usize> {
        self.workspace_state
            .workspace
            .active_tab
            .filter(|&index| {
                self.workspace_state
                    .tabs
                    .get(index)
                    .and_then(TabState::as_session)
                    .is_some_and(|session| session.purpose == SessionPurpose::Terminal)
            })
            .or_else(|| {
                self.workspace_state.active_topbar_tab.filter(|&index| {
                    self.workspace_state
                        .tabs
                        .get(index)
                        .and_then(TabState::as_session)
                        .is_some_and(|session| session.purpose == SessionPurpose::Terminal)
                })
            })
    }

    pub(in crate::ui::shell) fn set_active_session_pty_tap(
        &mut self,
        tap: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
    ) {
        let Some(index) = self.active_terminal_session_index() else {
            return;
        };
        let Some(session) = self
            .workspace_state
            .tabs
            .get_mut(index)
            .and_then(TabState::as_session_mut)
        else {
            return;
        };
        session.pty_output_tap = tap;
    }

    pub(in crate::ui::shell) fn set_session_pty_tap_by_tab_id(
        &mut self,
        tab_id: usize,
        tap: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
    ) {
        let Some(session) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_session_mut)
        else {
            return;
        };
        session.pty_output_tap = tap;
    }

    pub(in crate::ui::shell) fn clear_session_pty_taps_by_tab_id(&mut self, tab_ids: &[usize]) {
        for tab_id in tab_ids {
            self.set_session_pty_tap_by_tab_id(*tab_id, None);
        }
    }

    pub(in crate::ui::shell) fn toggle_session_side_panel(&mut self) {
        self.panels.session_side_panel_open = !self.panels.session_side_panel_open;
    }

    pub(in crate::ui::shell) fn toggle_session_agent_panel(&mut self) {
        self.panels.session_agent_panel_open = !self.panels.session_agent_panel_open;
    }

    pub(in crate::ui::shell) fn pending_host_key_session_index(&self) -> Option<usize> {
        self.workspace_state
            .workspace
            .active_tab
            .filter(|&index| {
                self.workspace_state
                    .tabs
                    .get(index)
                    .and_then(TabState::as_session)
                    .is_some_and(|session| session.pending_host_key.is_some())
            })
            .or_else(|| {
                self.workspace_state
                    .tabs
                    .iter()
                    .enumerate()
                    .find_map(|(index, tab)| {
                        tab.as_session()
                            .and_then(|session| session.pending_host_key.as_ref().map(|_| index))
                    })
            })
    }

    pub(in crate::ui::shell) fn pending_host_key_prompt(&self) -> Option<HostKeyPrompt> {
        self.pending_host_key_session_index()
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(TabState::as_session)
            .and_then(|session| session.pending_host_key.clone())
    }

    pub(in crate::ui::shell) fn pending_keyboard_interactive_prompt(&self) -> Option<KbiChallenge> {
        self.workspace_state
            .workspace
            .active_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(TabState::as_session)
            .and_then(|session| session.pending_keyboard_interactive.clone())
            .or_else(|| {
                self.workspace_state.tabs.iter().find_map(|tab| {
                    tab.as_session()
                        .and_then(|session| session.pending_keyboard_interactive.clone())
                })
            })
    }

    pub(in crate::ui::shell) fn pending_profile_delete_prompt(
        &self,
    ) -> Option<PendingProfileDeleteState> {
        self.dialogs.pending_profile_delete.clone()
    }

    pub(in crate::ui::shell) fn pending_managed_key_delete_prompt(
        &self,
    ) -> Option<PendingManagedKeyDeleteState> {
        self.dialogs.pending_managed_key_delete.clone()
    }

    pub(in crate::ui::shell) fn pending_known_host_delete_prompt(
        &self,
    ) -> Option<PendingKnownHostDeleteState> {
        self.dialogs.pending_known_host_delete.clone()
    }

    pub(in crate::ui::shell) fn pending_snippet_delete_prompt(
        &self,
    ) -> Option<PendingSnippetDeleteState> {
        self.dialogs.pending_snippet_delete.clone()
    }

    pub(in crate::ui::shell) fn pending_port_forward_rule_delete_prompt(
        &self,
    ) -> Option<PendingPortForwardRuleDeleteState> {
        self.dialogs.pending_port_forward_rule_delete.clone()
    }

    pub(in crate::ui::shell) fn pending_chat_session_delete_prompt(
        &self,
    ) -> Option<PendingChatSessionDeleteState> {
        self.dialogs.pending_chat_session_delete.clone()
    }

    pub(in crate::ui::shell) fn pending_sync_direction_prompt(
        &self,
    ) -> Option<PendingSyncDirectionState> {
        self.dialogs.pending_sync_direction
    }

    pub(in crate::ui::shell) fn pending_sync_pull_confirm_prompt(
        &self,
    ) -> Option<PendingSyncPullConfirmState> {
        self.dialogs.pending_sync_pull_confirm
    }

    pub(in crate::ui::shell) fn pending_local_vault_disable_confirm_prompt(
        &self,
    ) -> Option<PendingLocalVaultDisableConfirmState> {
        self.dialogs.pending_local_vault_disable_confirm
    }

    pub(in crate::ui::shell) fn pending_local_data_reset_confirm_prompt(
        &self,
    ) -> Option<PendingLocalDataResetConfirmState> {
        self.dialogs.pending_local_data_reset_confirm
    }

    pub(in crate::ui::shell) fn pending_local_data_reset_confirmation_popup(
        &self,
    ) -> Option<PendingLocalDataResetConfirmationPopupState> {
        self.dialogs.pending_local_data_reset_confirmation_popup
    }

    pub(in crate::ui::shell) fn pending_sync_passphrase_clear_confirm_popup(
        &self,
    ) -> Option<PendingSyncPassphraseClearConfirmPopupState> {
        self.dialogs.pending_sync_passphrase_clear_confirm_popup
    }

    pub(in crate::ui::shell) fn pending_sync_passphrase_popup(
        &self,
    ) -> Option<PendingSyncPassphrasePopupState> {
        self.sync_passphrase_popup
    }

    pub(in crate::ui::shell) fn pending_ai_provider_popup(
        &self,
    ) -> Option<PendingAiProviderPopupState> {
        self.ai_provider_popup
    }

    pub(in crate::ui::shell) fn pending_local_vault_passphrase_popup(
        &self,
    ) -> Option<LocalVaultPassphrasePopupMode> {
        self.local_vault_passphrase_popup
    }

    pub(in crate::ui::shell) fn pending_sftp_prompt(&self) -> Option<(usize, SftpPromptState)> {
        self.workspace_state
            .active_topbar_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .filter(|tab| !tab.hidden_from_topbar)
            .and_then(|tab| {
                tab.as_sftp()
                    .and_then(|sftp| sftp.prompt.clone().map(|prompt| (tab.id, prompt)))
            })
    }

    pub(in crate::ui::shell) fn start_dialog_exit(
        &mut self,
        snapshot: DialogOverlaySnapshot,
        cx: &mut Context<Self>,
    ) {
        let stable_key = snapshot.stable_key();
        self.dialogs
            .exiting_dialogs
            .retain(|dialog| dialog.snapshot.stable_key() != stable_key);
        self.dialogs.exiting_dialogs.push(ExitingDialogState {
            snapshot,
            started_at: Instant::now(),
        });
        cx.notify();
    }

    pub(in crate::ui::shell) fn active_exiting_dialogs(
        &mut self,
        window: &mut Window,
    ) -> Vec<(DialogOverlaySnapshot, f32)> {
        let now = Instant::now();
        let duration = support::OVERLAY_ENTER_DURATION;
        let mut render_states = Vec::new();

        self.dialogs.exiting_dialogs.retain(|dialog| {
            let elapsed = now.saturating_duration_since(dialog.started_at);
            if elapsed >= duration {
                return false;
            }

            let progress = (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0);
            render_states.push((dialog.snapshot.clone(), progress));
            true
        });

        if !render_states.is_empty() {
            window.request_animation_frame();
        }

        render_states
    }

    pub(in crate::ui::shell) fn active_tab_is_hosts(&self) -> bool {
        self.workspace_state
            .active_topbar_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .is_some_and(TabState::is_hosts)
    }

    pub(in crate::ui::shell) fn active_username(&self) -> String {
        if let Some(profile) = self.active_profile()
            && !profile.username.trim().is_empty()
        {
            return profile.username.clone();
        }

        std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "admin".into())
    }

    pub(in crate::ui::shell) fn window_title(&self) -> String {
        let active_index = self
            .workspace_state
            .workspace
            .active_tab
            .or(self.workspace_state.active_topbar_tab);

        let Some(active_index) = active_index else {
            return APP_TITLE.to_string();
        };

        let Some(tab) = self.workspace_state.tabs.get(active_index) else {
            return APP_TITLE.to_string();
        };

        if tab.is_hosts() {
            return APP_TITLE.to_string();
        }

        let title = tab.title.trim();
        if !title.is_empty() {
            return format!("{title} - {APP_TITLE}");
        }

        self.active_profile()
            .map(|profile| format!("{} - {APP_TITLE}", profile.summary()))
            .unwrap_or_else(|| APP_TITLE.to_string())
    }
}
