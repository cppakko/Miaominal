use super::state::{
    PendingLocalDataResetConfirmState, PendingLocalDataResetConfirmationPopupState,
    PendingSyncPassphraseClearConfirmPopupState, PendingSyncPassphrasePopupState,
};
use super::*;
use crate::ui::i18n;
use gpui_component::Disableable;

#[derive(Clone, Copy)]
struct PageEditorSidebarRenderState {
    kind: PageEditorSidebarKind,
    visibility: f32,
}

#[derive(Clone, Copy)]
struct TerminalViewRenderState {
    tab_id: usize,
    phase: TerminalViewTransitionPhase,
    visibility: f32,
    animating: bool,
}

#[derive(Clone, Copy)]
struct HostsToTerminalTransitionRenderState {
    terminal_tab_id: usize,
    visibility: f32,
    show_host_editor_sidebar: bool,
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let window_title = self.window_title();
        if window.window_title() != window_title {
            window.set_window_title(&window_title);
        }

        let entity = cx.entity();
        let roles = settings::current_theme().material.roles;
        if self.onboarding.show_onboarding {
            return self.render_onboarding_page(entity, window, cx);
        }
        let has_active_session = self.has_active_session();
        let has_active_sftp_tab = self
            .workspace_state
            .active_topbar_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(TabState::as_sftp)
            .is_some();
        let hosts_to_terminal_transition = self.hosts_to_terminal_transition_render_state(window);
        let active_terminal_tab_id = self.active_terminal_tab_id();
        let terminal_view_transition = if hosts_to_terminal_transition.is_some() {
            self.clear_terminal_view_transition_state();
            None
        } else {
            self.terminal_view_render_state(active_terminal_tab_id, window)
        };
        let show_sidebar = !has_active_session && !has_active_sftp_tab;

        let pending_host_key = self.pending_host_key_prompt();
        let pending_kbi = self.pending_keyboard_interactive_prompt();
        let pending_profile_delete = self.pending_profile_delete_prompt();
        let pending_managed_key_delete = self.pending_managed_key_delete_prompt();
        let pending_known_host_delete = self.pending_known_host_delete_prompt();
        let pending_snippet_delete = self.pending_snippet_delete_prompt();
        let pending_port_forward_rule_delete = self.pending_port_forward_rule_delete_prompt();
        let pending_sync_direction = self.pending_sync_direction_prompt();
        let pending_sync_pull_confirm = self.pending_sync_pull_confirm_prompt();
        let pending_local_vault_disable_confirm = self.pending_local_vault_disable_confirm_prompt();
        let pending_local_data_reset_confirm = self.pending_local_data_reset_confirm_prompt();
        let pending_local_data_reset_confirmation_popup =
            self.pending_local_data_reset_confirmation_popup();
        let pending_sync_passphrase_clear_confirm_popup =
            self.pending_sync_passphrase_clear_confirm_popup();
        let pending_sync_passphrase_popup = self.pending_sync_passphrase_popup();
        let pending_local_vault_passphrase_popup = self.pending_local_vault_passphrase_popup();
        let pending_sftp_prompt = self.pending_sftp_prompt();
        let exiting_dialogs = self.active_exiting_dialogs(window);
        let has_exiting_kbi = exiting_dialogs
            .iter()
            .any(|(snapshot, _)| matches!(snapshot, DialogOverlaySnapshot::KeyboardInteractive(_)));

        if let Some(challenge) = &pending_kbi {
            if self.kbi_inputs.len() != challenge.prompts.len() {
                self.kbi_inputs = challenge
                    .prompts
                    .iter()
                    .map(|prompt| new_input_state("", "", !prompt.echo, window, cx))
                    .collect();
            }
        } else if !has_exiting_kbi && !self.kbi_inputs.is_empty() {
            self.kbi_inputs.clear();
        }

        let show_host_editor_sidebar = self.editors.host_editor_open
            && !has_active_session
            && self.panel_view.sidebar_section == SidebarSection::Hosts;
        let show_port_forward_editor_sidebar = self.editors.port_forward_editor_open
            && !has_active_session
            && !has_active_sftp_tab
            && self.panel_view.sidebar_section == SidebarSection::PortForwarding;
        let show_snippets_editor_sidebar = self.editors.snippets_editor_open
            && !has_active_session
            && !has_active_sftp_tab
            && self.panel_view.sidebar_section == SidebarSection::Snippets;
        let show_keychain_editor_sidebar = self.editors.keychain_editor_open
            && !has_active_session
            && !has_active_sftp_tab
            && self.panel_view.sidebar_section == SidebarSection::Keychain;
        let page_editor_sidebar = if hosts_to_terminal_transition.is_some()
            || has_active_session
            || has_active_sftp_tab
        {
            self.clear_page_editor_sidebar_transition_state();
            None
        } else {
            self.page_editor_sidebar_render_state(
                self.desired_page_editor_sidebar_kind(
                    show_host_editor_sidebar,
                    show_port_forward_editor_sidebar,
                    show_snippets_editor_sidebar,
                    show_keychain_editor_sidebar,
                ),
                window,
            )
        };
        let show_host_editor_sidebar = matches!(
            page_editor_sidebar,
            Some(sidebar) if sidebar.kind == PageEditorSidebarKind::Hosts
        );
        let show_port_forward_editor_sidebar = matches!(
            page_editor_sidebar,
            Some(sidebar) if sidebar.kind == PageEditorSidebarKind::PortForwarding
        );
        let show_snippets_editor_sidebar = matches!(
            page_editor_sidebar,
            Some(sidebar) if sidebar.kind == PageEditorSidebarKind::Snippets
        );
        let show_keychain_editor_sidebar = matches!(
            page_editor_sidebar,
            Some(sidebar) if sidebar.kind == PageEditorSidebarKind::Keychain
        );
        let shell_body = if let Some(transition) = hosts_to_terminal_transition {
            self.render_hosts_to_terminal_transition(
                transition,
                entity.clone(),
                show_host_editor_sidebar,
                window,
                cx,
            )
        } else if let Some(transition) = terminal_view_transition.filter(|state| state.animating) {
            self.render_terminal_view_transition(
                transition,
                entity.clone(),
                has_active_sftp_tab,
                show_sidebar,
                window,
                cx,
            )
        } else {
            let primary_panel = if has_active_session {
                self.render_terminal_page(window, cx)
            } else if has_active_sftp_tab {
                self.render_sftp_page(entity.clone(), cx)
            } else if self.panel_view.sidebar_section == SidebarSection::Hosts {
                self.render_hosts_page(entity.clone(), cx)
            } else if self.panel_view.sidebar_section == SidebarSection::Keychain {
                self.render_keychain_page(entity.clone(), cx)
            } else if self.panel_view.sidebar_section == SidebarSection::PortForwarding {
                self.render_forward_page(entity.clone(), cx)
            } else if self.panel_view.sidebar_section == SidebarSection::KnownHosts {
                self.render_trusted_page(entity.clone())
            } else if self.panel_view.sidebar_section == SidebarSection::Settings {
                self.render_settings_page(entity.clone())
            } else {
                self.render_snippets_page(entity.clone(), cx)
            };
            let animate_sidebar_page = self.should_animate_sidebar_page_container(
                self.panel_view.sidebar_section,
                show_sidebar,
            );
            let primary_panel = super::support::render_sidebar_page_container(
                primary_panel,
                self.panel_view.sidebar_section,
                animate_sidebar_page,
            );

            self.render_shell_body(
                entity.clone(),
                primary_panel,
                has_active_session,
                has_active_sftp_tab,
                show_sidebar,
                page_editor_sidebar,
                show_host_editor_sidebar,
                show_port_forward_editor_sidebar,
                show_snippets_editor_sidebar,
                show_keychain_editor_sidebar,
                cx,
            )
        };

        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(rgb(roles.surface_container))
            .when(!has_active_session && !has_active_sftp_tab, |this| {
                this.pr_2()
            })
            .child(self.render_top_bar(entity.clone(), window))
            .child(shell_body)
            .child(self.render_status_footer(entity.clone()))
            .when_some(pending_host_key, |this, prompt| {
                this.child(self.render_trusted_host_key_prompt(entity.clone(), &prompt, None))
            })
            .when_some(pending_kbi, |this, challenge| {
                this.child(self.render_keyboard_interactive_prompt(
                    entity.clone(),
                    &challenge,
                    None,
                ))
            })
            .when_some(pending_profile_delete, |this, prompt| {
                this.child(self.render_profile_delete_prompt(entity.clone(), &prompt, None))
            })
            .when_some(pending_managed_key_delete, |this, prompt| {
                this.child(self.render_managed_key_delete_prompt(entity.clone(), &prompt, None))
            })
            .when_some(pending_known_host_delete, |this, prompt| {
                this.child(self.render_trusted_known_host_delete_prompt(
                    entity.clone(),
                    &prompt,
                    None,
                ))
            })
            .when_some(pending_snippet_delete, |this, prompt| {
                this.child(self.render_snippet_delete_prompt(entity.clone(), &prompt, None))
            })
            .when_some(pending_port_forward_rule_delete, |this, prompt| {
                this.child(self.render_port_forward_rule_delete_prompt(
                    entity.clone(),
                    &prompt,
                    None,
                ))
            })
            .when_some(pending_sync_direction, |this, prompt| {
                this.child(self.render_sync_direction_prompt(entity.clone(), &prompt, None))
            })
            .when_some(pending_sync_pull_confirm, |this, prompt| {
                this.child(self.render_sync_pull_confirm_prompt(entity.clone(), &prompt, None))
            })
            .when_some(pending_local_vault_disable_confirm, |this, prompt| {
                this.child(self.render_local_vault_disable_confirm_prompt(
                    entity.clone(),
                    &prompt,
                    None,
                ))
            })
            .when_some(pending_local_data_reset_confirm, |this, prompt| {
                this.child(self.render_local_data_reset_confirm_prompt(
                    entity.clone(),
                    &prompt,
                    None,
                ))
            })
            .when_some(
                pending_local_data_reset_confirmation_popup,
                |this, prompt| {
                    this.child(self.render_local_data_reset_confirmation_popup(
                        entity.clone(),
                        prompt,
                        None,
                    ))
                },
            )
            .when_some(pending_sync_passphrase_clear_confirm_popup, |this, prompt| {
                this.child(self.render_sync_passphrase_clear_confirm_popup(
                    entity.clone(),
                    prompt,
                    None,
                ))
            })
            .when_some(pending_sync_passphrase_popup, |this, prompt| {
                this.child(self.render_sync_passphrase_popup(entity.clone(), prompt, None))
            })
            .when_some(pending_local_vault_passphrase_popup, |this, mode| {
                this.child(self.render_local_vault_passphrase_popup(entity.clone(), mode, None))
            })
            .when_some(pending_sftp_prompt, |this, prompt| {
                let (tab_id, prompt) = prompt;
                this.child(self.render_sftp_prompt_overlay(entity.clone(), tab_id, &prompt, None))
            })
            .children(exiting_dialogs.into_iter().map(|(snapshot, progress)| {
                self.render_exiting_dialog_overlay(entity.clone(), snapshot, progress)
            }))
            .when_some(
                Root::render_notification_layer(window, cx),
                |this, layer| this.child(layer),
            )
            .into_any_element()
    }
}

impl AppView {
    fn active_terminal_tab_id(&self) -> Option<usize> {
        self.workspace_state
            .active_topbar_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(|tab| {
                tab.as_session()
                    .filter(|session| session.purpose == SessionPurpose::Terminal)
                    .map(|_| tab.id)
            })
    }

    fn clear_hosts_transition_editor_state(&mut self, transition: HostsToTerminalTransition) {
        self.workspace_state.hosts_to_terminal_transition = None;
        if transition.show_host_editor_sidebar {
            self.editors.host_editor_open = false;
            self.editors.host_editor_is_new = false;
        }
    }

    fn finish_hosts_transition(&mut self, transition: HostsToTerminalTransition) {
        if matches!(
            transition.direction,
            HostsToTerminalTransitionDirection::ToHosts
        ) {
            self.shell_state.suppressed_page_container_animation_section =
                Some(SidebarSection::Hosts);
        }

        self.clear_hosts_transition_editor_state(transition);
    }

    fn clear_terminal_view_transition_state(&mut self) {
        self.workspace_state.terminal_view_transition = None;
        self.workspace_state.visible_terminal_view_tab_id = self.active_terminal_tab_id();
    }

    fn hosts_to_terminal_transition_render_state(
        &mut self,
        window: &mut Window,
    ) -> Option<HostsToTerminalTransitionRenderState> {
        let transition = self.workspace_state.hosts_to_terminal_transition?;
        let active_tab_id = self
            .workspace_state
            .active_topbar_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .map(|tab| tab.id);
        let active_tab_is_hosts = self
            .workspace_state
            .active_topbar_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .is_some_and(TabState::is_hosts);
        let active_tab_is_terminal = self
            .workspace_state
            .active_topbar_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(TabState::as_session)
            .is_some_and(|session| session.purpose == SessionPurpose::Terminal);
        let terminal_tab_exists = self
            .workspace_state
            .tabs
            .iter()
            .any(|tab| tab.id == transition.terminal_tab_id);

        let transition_still_valid = active_tab_id == Some(transition.active_tab_id)
            && self.panel_view.sidebar_section == SidebarSection::Hosts
            && terminal_tab_exists
            && match transition.direction {
                HostsToTerminalTransitionDirection::ToTerminal => active_tab_is_terminal,
                HostsToTerminalTransitionDirection::ToHosts => active_tab_is_hosts,
            };

        if !transition_still_valid {
            self.clear_hosts_transition_editor_state(transition);
            return None;
        }

        let duration_seconds = transition.duration.as_secs_f32();
        if duration_seconds <= f32::EPSILON {
            self.finish_hosts_transition(transition);
            return None;
        }

        let elapsed = Instant::now().saturating_duration_since(transition.started_at);
        let progress = (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0);

        if progress >= 1.0 {
            self.finish_hosts_transition(transition);
            return None;
        }

        window.request_animation_frame();
        let eased = progress * progress * (3.0 - 2.0 * progress);
        let visibility = match transition.direction {
            HostsToTerminalTransitionDirection::ToTerminal => eased,
            HostsToTerminalTransitionDirection::ToHosts => 1.0 - eased,
        };

        Some(HostsToTerminalTransitionRenderState {
            terminal_tab_id: transition.terminal_tab_id,
            visibility,
            show_host_editor_sidebar: transition.show_host_editor_sidebar,
        })
    }

    fn clear_page_editor_sidebar_transition_state(&mut self) {
        self.shell_state.page_editor_sidebar_transition = None;
        self.shell_state.visible_page_editor_sidebar = None;
    }

    fn should_animate_sidebar_page_container(
        &mut self,
        section: SidebarSection,
        show_sidebar: bool,
    ) -> bool {
        if !show_sidebar {
            self.shell_state.suppressed_page_container_animation_section = None;
            return false;
        }

        match self.shell_state.suppressed_page_container_animation_section {
            Some(suppressed_section) if suppressed_section == section => false,
            Some(_) => {
                self.shell_state.suppressed_page_container_animation_section = None;
                true
            }
            None => true,
        }
    }

    fn terminal_view_render_state(
        &mut self,
        desired: Option<usize>,
        window: &mut Window,
    ) -> Option<TerminalViewRenderState> {
        let now = Instant::now();
        let duration = super::support::CONTAINER_TRANSITION_DURATION;

        match (self.workspace_state.visible_terminal_view_tab_id, desired) {
            (None, Some(tab_id)) => {
                self.workspace_state.visible_terminal_view_tab_id = Some(tab_id);
                self.workspace_state.terminal_view_transition = Some(TerminalViewTransition {
                    tab_id,
                    phase: TerminalViewTransitionPhase::Entering,
                    started_at: now,
                    duration,
                });
            }
            (Some(tab_id), None) => match self.workspace_state.terminal_view_transition {
                Some(transition) if transition.tab_id == tab_id => {
                    if transition.phase == TerminalViewTransitionPhase::Entering {
                        self.workspace_state.terminal_view_transition =
                            Some(TerminalViewTransition {
                                phase: TerminalViewTransitionPhase::Exiting,
                                started_at: now,
                                ..transition
                            });
                    }
                }
                _ => {
                    self.workspace_state.terminal_view_transition = Some(TerminalViewTransition {
                        tab_id,
                        phase: TerminalViewTransitionPhase::Exiting,
                        started_at: now,
                        duration,
                    });
                }
            },
            (Some(tab_id), Some(desired_tab_id)) if tab_id == desired_tab_id => {
                if let Some(transition) = self.workspace_state.terminal_view_transition
                    && transition.tab_id == tab_id
                    && transition.phase == TerminalViewTransitionPhase::Exiting
                {
                    self.workspace_state.terminal_view_transition = Some(TerminalViewTransition {
                        phase: TerminalViewTransitionPhase::Entering,
                        started_at: now,
                        ..transition
                    });
                }
            }
            (Some(current), Some(desired_tab_id)) if current != desired_tab_id => {
                self.workspace_state.visible_terminal_view_tab_id = Some(desired_tab_id);
                self.workspace_state.terminal_view_transition = None;
            }
            _ => {}
        }

        if let Some(transition) = self.workspace_state.terminal_view_transition {
            let duration_seconds = transition.duration.as_secs_f32();
            if duration_seconds <= f32::EPSILON {
                self.workspace_state.terminal_view_transition = None;
                self.workspace_state.visible_terminal_view_tab_id = match transition.phase {
                    TerminalViewTransitionPhase::Entering => Some(transition.tab_id),
                    TerminalViewTransitionPhase::Exiting => {
                        self.shell_state.suppressed_page_container_animation_section =
                            Some(self.panel_view.sidebar_section);
                        None
                    }
                };

                return self
                    .workspace_state
                    .visible_terminal_view_tab_id
                    .map(|tab_id| TerminalViewRenderState {
                        tab_id,
                        phase: TerminalViewTransitionPhase::Entering,
                        visibility: 1.0,
                        animating: false,
                    });
            }

            let elapsed = now.saturating_duration_since(transition.started_at);
            let progress = (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0);
            let eased = progress * progress * (3.0 - 2.0 * progress);

            if progress >= 1.0 {
                self.workspace_state.terminal_view_transition = None;
                self.workspace_state.visible_terminal_view_tab_id = match transition.phase {
                    TerminalViewTransitionPhase::Entering => Some(transition.tab_id),
                    TerminalViewTransitionPhase::Exiting => {
                        self.shell_state.suppressed_page_container_animation_section =
                            Some(self.panel_view.sidebar_section);
                        None
                    }
                };

                return self
                    .workspace_state
                    .visible_terminal_view_tab_id
                    .map(|tab_id| TerminalViewRenderState {
                        tab_id,
                        phase: TerminalViewTransitionPhase::Entering,
                        visibility: 1.0,
                        animating: false,
                    });
            }

            window.request_animation_frame();

            return Some(TerminalViewRenderState {
                tab_id: transition.tab_id,
                phase: transition.phase,
                visibility: match transition.phase {
                    TerminalViewTransitionPhase::Entering => eased,
                    TerminalViewTransitionPhase::Exiting => 1.0 - eased,
                },
                animating: true,
            });
        }

        self.workspace_state.visible_terminal_view_tab_id = desired;
        desired.map(|tab_id| TerminalViewRenderState {
            tab_id,
            phase: TerminalViewTransitionPhase::Entering,
            visibility: 1.0,
            animating: false,
        })
    }

    fn render_non_terminal_primary_panel(
        &mut self,
        entity: Entity<Self>,
        has_active_sftp_tab: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if has_active_sftp_tab {
            self.render_sftp_page(entity, cx)
        } else if self.panel_view.sidebar_section == SidebarSection::Hosts {
            self.render_hosts_page(entity, cx)
        } else if self.panel_view.sidebar_section == SidebarSection::Keychain {
            self.render_keychain_page(entity, cx)
        } else if self.panel_view.sidebar_section == SidebarSection::PortForwarding {
            self.render_forward_page(entity, cx)
        } else if self.panel_view.sidebar_section == SidebarSection::KnownHosts {
            self.render_trusted_page(entity)
        } else if self.panel_view.sidebar_section == SidebarSection::Settings {
            self.render_settings_page(entity)
        } else {
            self.render_snippets_page(entity, cx)
        }
    }

    fn render_terminal_page_for_tab(
        &mut self,
        tab_id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let index = self
            .workspace_state
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)?;

        if self.workspace_state.active_topbar_tab == Some(index)
            && self
                .workspace_state
                .tabs
                .get(index)
                .and_then(TabState::as_session)
                .is_some()
        {
            return Some(self.render_terminal_page(window, cx));
        }

        let parked_workspace = self.workspace_state.tabs.get_mut(index)?.workspace.take()?;
        let live_workspace =
            std::mem::replace(&mut self.workspace_state.workspace, parked_workspace);
        let rendered = self.render_workspace_surface(window, cx);
        let parked_workspace =
            std::mem::replace(&mut self.workspace_state.workspace, live_workspace);
        if let Some(tab) = self.workspace_state.tabs.get_mut(index) {
            tab.workspace = Some(parked_workspace);
        }

        Some(rendered)
    }

    fn desired_page_editor_sidebar_kind(
        &self,
        show_host_editor_sidebar: bool,
        show_port_forward_editor_sidebar: bool,
        show_snippets_editor_sidebar: bool,
        show_keychain_editor_sidebar: bool,
    ) -> Option<PageEditorSidebarKind> {
        if show_host_editor_sidebar {
            Some(PageEditorSidebarKind::Hosts)
        } else if show_port_forward_editor_sidebar {
            Some(PageEditorSidebarKind::PortForwarding)
        } else if show_snippets_editor_sidebar {
            Some(PageEditorSidebarKind::Snippets)
        } else if show_keychain_editor_sidebar {
            Some(PageEditorSidebarKind::Keychain)
        } else {
            None
        }
    }

    fn page_editor_sidebar_render_state(
        &mut self,
        desired: Option<PageEditorSidebarKind>,
        window: &mut Window,
    ) -> Option<PageEditorSidebarRenderState> {
        let now = Instant::now();
        let duration = super::support::OVERLAY_ENTER_DURATION;

        match desired {
            Some(kind) => match self.shell_state.page_editor_sidebar_transition {
                Some(transition) if transition.kind == kind => {
                    if transition.phase == PageEditorSidebarTransitionPhase::Exiting {
                        self.shell_state.page_editor_sidebar_transition =
                            Some(PageEditorSidebarTransition {
                                phase: PageEditorSidebarTransitionPhase::Entering,
                                started_at: now,
                                ..transition
                            });
                    }
                }
                _ => {
                    if self.shell_state.visible_page_editor_sidebar != Some(kind)
                        || self.shell_state.page_editor_sidebar_transition.is_some()
                    {
                        self.shell_state.visible_page_editor_sidebar = Some(kind);
                        self.shell_state.page_editor_sidebar_transition =
                            Some(PageEditorSidebarTransition {
                                kind,
                                phase: PageEditorSidebarTransitionPhase::Entering,
                                started_at: now,
                                duration,
                            });
                    }
                }
            },
            None => {
                if let Some(kind) = self.shell_state.visible_page_editor_sidebar {
                    match self.shell_state.page_editor_sidebar_transition {
                        Some(transition) if transition.kind == kind => {
                            if transition.phase == PageEditorSidebarTransitionPhase::Entering {
                                self.shell_state.page_editor_sidebar_transition =
                                    Some(PageEditorSidebarTransition {
                                        phase: PageEditorSidebarTransitionPhase::Exiting,
                                        started_at: now,
                                        ..transition
                                    });
                            }
                        }
                        _ => {
                            self.shell_state.page_editor_sidebar_transition =
                                Some(PageEditorSidebarTransition {
                                    kind,
                                    phase: PageEditorSidebarTransitionPhase::Exiting,
                                    started_at: now,
                                    duration,
                                });
                        }
                    }
                }
            }
        }

        if let Some(transition) = self.shell_state.page_editor_sidebar_transition {
            let duration_seconds = transition.duration.as_secs_f32();
            if duration_seconds <= f32::EPSILON {
                self.shell_state.page_editor_sidebar_transition = None;
                self.shell_state.visible_page_editor_sidebar = match transition.phase {
                    PageEditorSidebarTransitionPhase::Entering => Some(transition.kind),
                    PageEditorSidebarTransitionPhase::Exiting => None,
                };
                return self.shell_state.visible_page_editor_sidebar.map(|kind| {
                    PageEditorSidebarRenderState {
                        kind,
                        visibility: 1.0,
                    }
                });
            }

            let elapsed = now.saturating_duration_since(transition.started_at);
            let progress = (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0);
            let eased = progress * progress * (3.0 - 2.0 * progress);

            if progress >= 1.0 {
                self.shell_state.page_editor_sidebar_transition = None;
                self.shell_state.visible_page_editor_sidebar = match transition.phase {
                    PageEditorSidebarTransitionPhase::Entering => Some(transition.kind),
                    PageEditorSidebarTransitionPhase::Exiting => None,
                };

                return self.shell_state.visible_page_editor_sidebar.map(|kind| {
                    PageEditorSidebarRenderState {
                        kind,
                        visibility: 1.0,
                    }
                });
            }

            window.request_animation_frame();

            return Some(PageEditorSidebarRenderState {
                kind: transition.kind,
                visibility: match transition.phase {
                    PageEditorSidebarTransitionPhase::Entering => eased,
                    PageEditorSidebarTransitionPhase::Exiting => 1.0 - eased,
                },
            });
        }

        self.shell_state.visible_page_editor_sidebar = desired;
        desired.map(|kind| PageEditorSidebarRenderState {
            kind,
            visibility: 1.0,
        })
    }

    fn render_shell_body(
        &mut self,
        entity: Entity<Self>,
        primary_panel: AnyElement,
        has_active_session: bool,
        has_active_sftp_tab: bool,
        show_sidebar: bool,
        page_editor_sidebar: Option<PageEditorSidebarRenderState>,
        show_host_editor_sidebar: bool,
        show_port_forward_editor_sidebar: bool,
        show_snippets_editor_sidebar: bool,
        show_keychain_editor_sidebar: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let roles = settings::current_theme().material.roles;

        div()
            .flex_1()
            .w_full()
            .relative()
            .flex()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .rounded(px(16.0))
            .overflow_hidden()
            .when(show_sidebar, |this| {
                this.child(self.render_sidebar(entity.clone()))
            })
            .when(has_active_session || has_active_sftp_tab, |this| {
                this.pr_2().pl_2()
            })
            .child(
                div()
                    .flex_1()
                    .flex()
                    .rounded(px(16.0))
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .bg(rgb(roles.surface_container))
                    .child(self.render_primary_surface_layer(
                        entity.clone(),
                        primary_panel,
                        has_active_session,
                        has_active_sftp_tab,
                        show_host_editor_sidebar,
                        show_port_forward_editor_sidebar,
                        show_keychain_editor_sidebar,
                        show_snippets_editor_sidebar,
                    ))
                    .when_some(page_editor_sidebar, |this, sidebar| {
                        this.child(self.render_page_editor_sidebar(sidebar, entity.clone(), cx))
                    }),
            )
            .into_any_element()
    }

    fn render_terminal_view_transition(
        &mut self,
        transition: TerminalViewRenderState,
        entity: Entity<Self>,
        has_active_sftp_tab: bool,
        show_sidebar: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let roles = settings::current_theme().material.roles;
        let non_terminal_section = self.panel_view.sidebar_section;
        let non_terminal_panel =
            self.render_non_terminal_primary_panel(entity.clone(), has_active_sftp_tab, cx);
        let terminal_panel = self
            .render_terminal_page_for_tab(transition.tab_id, window, cx)
            .unwrap_or_else(|| self.render_terminal_page(window, cx));
        let (
            incoming_surface,
            outgoing_surface,
            incoming_opacity,
            outgoing_opacity,
            incoming_shift,
            outgoing_shift,
        ) = match transition.phase {
            TerminalViewTransitionPhase::Entering => (
                self.render_primary_surface_layer(
                    entity.clone(),
                    terminal_panel,
                    true,
                    false,
                    false,
                    false,
                    false,
                    false,
                ),
                self.render_primary_surface_layer(
                    entity.clone(),
                    super::support::render_sidebar_page_container(
                        non_terminal_panel,
                        non_terminal_section,
                        false,
                    ),
                    false,
                    has_active_sftp_tab,
                    false,
                    false,
                    false,
                    false,
                ),
                0.82 + transition.visibility * 0.18,
                1.0 - transition.visibility,
                (1.0 - transition.visibility) * 8.0,
                transition.visibility * -10.0,
            ),
            TerminalViewTransitionPhase::Exiting => {
                let outgoing_terminal_panel = self
                    .render_terminal_page_for_tab(transition.tab_id, window, cx)
                    .unwrap_or_else(|| self.render_terminal_page(window, cx));
                (
                    self.render_primary_surface_layer(
                        entity.clone(),
                        super::support::render_sidebar_page_container(
                            non_terminal_panel,
                            non_terminal_section,
                            false,
                        ),
                        false,
                        has_active_sftp_tab,
                        false,
                        false,
                        false,
                        false,
                    ),
                    self.render_primary_surface_layer(
                        entity.clone(),
                        outgoing_terminal_panel,
                        true,
                        false,
                        false,
                        false,
                        false,
                        false,
                    ),
                    0.82 + transition.visibility * 0.18,
                    transition.visibility,
                    0.0,
                    (1.0 - transition.visibility) * -10.0,
                )
            }
        };

        div()
            .flex_1()
            .w_full()
            .relative()
            .flex()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .rounded(px(16.0))
            .overflow_hidden()
            .when(show_sidebar, |this| {
                this.child(self.render_sidebar(entity.clone()))
            })
            .when(!show_sidebar, |this| this.pr_2().pl_2())
            .child(
                div()
                    .flex_1()
                    .flex()
                    .rounded(px(16.0))
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .bg(rgb(roles.surface_container))
                    .child(
                        div()
                            .size_full()
                            .relative()
                            .child(
                                div()
                                    .absolute()
                                    .top(px(incoming_shift))
                                    .right(px(0.0))
                                    .bottom(px(-incoming_shift))
                                    .left(px(0.0))
                                    .opacity(incoming_opacity)
                                    .child(incoming_surface),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top(px(outgoing_shift))
                                    .right(px(0.0))
                                    .bottom(px(-outgoing_shift))
                                    .left(px(0.0))
                                    .opacity(outgoing_opacity)
                                    .child(outgoing_surface),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_page_editor_sidebar_content(
        &self,
        kind: PageEditorSidebarKind,
        entity: Entity<Self>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match kind {
            PageEditorSidebarKind::Hosts => self
                .render_hosts_editor_sidebar(entity, cx)
                .into_any_element(),
            PageEditorSidebarKind::PortForwarding => self
                .render_port_forward_editor_sidebar(entity, cx)
                .into_any_element(),
            PageEditorSidebarKind::Snippets => self
                .render_snippets_editor_sidebar(entity)
                .into_any_element(),
            PageEditorSidebarKind::Keychain => self
                .render_keychain_editor_sidebar(entity)
                .into_any_element(),
        }
    }

    fn render_page_editor_sidebar(
        &self,
        sidebar: PageEditorSidebarRenderState,
        entity: Entity<Self>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let visible_width = EDITOR_DRAWER_WIDTH * sidebar.visibility;
        let slide_offset = 18.0 * (1.0 - sidebar.visibility);
        let opacity = 0.28 + sidebar.visibility * 0.72;
        let content = self.render_page_editor_sidebar_content(sidebar.kind, entity, cx);

        div()
            .relative()
            .h_full()
            .w(px(visible_width))
            .min_w(px(0.0))
            .flex_shrink_0()
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .top(px(0.0))
                    .right(px(-slide_offset))
                    .bottom(px(0.0))
                    .opacity(opacity)
                    .child(content),
            )
            .into_any_element()
    }

    fn render_hosts_to_terminal_transition(
        &mut self,
        transition: HostsToTerminalTransitionRenderState,
        entity: Entity<Self>,
        show_host_editor_sidebar: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let progress = transition.visibility;
        let show_transition_host_editor_sidebar =
            transition.show_host_editor_sidebar || show_host_editor_sidebar;
        let roles = settings::current_theme().material.roles;
        let expansion_left = LEFT_RAIL_WIDTH * (1.0 - progress);
        let terminal_side_inset = 8.0 * progress;
        let host_editor_visible_width = EDITOR_DRAWER_WIDTH * (1.0 - progress);
        let sidebar_opacity = 1.0 - progress;
        let hosts_opacity = 1.0 - progress;
        let terminal_opacity = 0.2 + progress * 0.8;
        let hosts_panel = self.render_hosts_page(entity.clone(), cx);
        let terminal_panel = self
            .render_terminal_page_for_tab(transition.terminal_tab_id, window, cx)
            .unwrap_or_else(|| self.render_terminal_page(window, cx));
        let hosts_surface = self.render_primary_surface_layer(
            entity.clone(),
            hosts_panel,
            false,
            false,
            show_transition_host_editor_sidebar,
            false,
            false,
            false,
        );
        let hosts_layer = div()
            .size_full()
            .flex()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .child(hosts_surface)
            .when(show_transition_host_editor_sidebar, |this| {
                this.child(
                    div()
                        .relative()
                        .h_full()
                        .w(px(host_editor_visible_width))
                        .min_w(px(0.0))
                        .flex_shrink_0()
                        .overflow_hidden()
                        .child(
                            div()
                                .absolute()
                                .top(px(0.0))
                                .right(px(0.0))
                                .bottom(px(0.0))
                                .child(self.render_hosts_editor_sidebar(entity.clone(), cx)),
                        ),
                )
            });
        let terminal_surface = self.render_primary_surface_layer(
            entity.clone(),
            terminal_panel,
            true,
            false,
            false,
            false,
            false,
            false,
        );

        div()
            .flex_1()
            .w_full()
            .relative()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .rounded(px(16.0))
            .overflow_hidden()
            .child(
                div()
                    .relative()
                    .size_full()
                    .rounded(px(16.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .absolute()
                            .left(px(0.0))
                            .top(px(0.0))
                            .bottom(px(0.0))
                            .opacity(sidebar_opacity)
                            .child(self.render_sidebar(entity.clone())),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(expansion_left + terminal_side_inset))
                            .top(px(0.0))
                            .right(px(terminal_side_inset))
                            .bottom(px(0.0))
                            .bg(rgb(roles.surface_container))
                            .child(
                                div()
                                    .size_full()
                                    .opacity(terminal_opacity)
                                    .child(terminal_surface),
                            ),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(LEFT_RAIL_WIDTH))
                            .top(px(0.0))
                            .right(px(0.0))
                            .bottom(px(0.0))
                            .opacity(hosts_opacity)
                            .child(hosts_layer),
                    ),
            )
            .into_any_element()
    }

    fn render_primary_surface_layer(
        &self,
        entity: Entity<Self>,
        panel: AnyElement,
        has_active_session: bool,
        has_active_sftp_tab: bool,
        show_host_editor_sidebar: bool,
        show_port_forward_editor_sidebar: bool,
        show_keychain_editor_sidebar: bool,
        show_snippets_editor_sidebar: bool,
    ) -> AnyElement {
        let roles = settings::current_theme().material.roles;

        div()
            .size_full()
            .flex()
            .flex_col()
            .rounded(px(16.0))
            .relative()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .when(has_active_session, |this| {
                this.bg(rgb(roles.surface_container))
            })
            .when(!has_active_session, |this| this.bg(rgb(roles.background)))
            .child(panel)
            .when(
                self.panel_view.sidebar_section == SidebarSection::Hosts
                    && !has_active_session
                    && !has_active_sftp_tab
                    && !show_host_editor_sidebar,
                |this| {
                    this.child(
                        div()
                            .absolute()
                            .right(px(28.0))
                            .bottom(px(28.0))
                            .child(self.render_fab(entity.clone())),
                    )
                },
            )
            .when(
                self.panel_view.sidebar_section == SidebarSection::PortForwarding
                    && !has_active_session
                    && !has_active_sftp_tab
                    && !show_port_forward_editor_sidebar,
                |this| {
                    this.child(
                        div()
                            .absolute()
                            .right(px(28.0))
                            .bottom(px(28.0))
                            .child(self.render_forward_fab(entity.clone())),
                    )
                },
            )
            .when(
                self.panel_view.sidebar_section == SidebarSection::Keychain
                    && !has_active_session
                    && !has_active_sftp_tab
                    && !show_keychain_editor_sidebar,
                |this| {
                    this.child(
                        div()
                            .absolute()
                            .right(px(28.0))
                            .bottom(px(28.0))
                            .child(self.render_keychain_fab(entity.clone())),
                    )
                },
            )
            .when(
                self.panel_view.sidebar_section == SidebarSection::Snippets
                    && !has_active_session
                    && !has_active_sftp_tab
                    && !show_snippets_editor_sidebar,
                |this| {
                    this.child(
                        div()
                            .absolute()
                            .right(px(28.0))
                            .bottom(px(28.0))
                            .child(self.render_snippets_fab(entity.clone())),
                    )
                },
            )
            .into_any_element()
    }

    fn render_managed_key_delete_prompt(
        &self,
        entity: Entity<AppView>,
        prompt: &PendingManagedKeyDeleteState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let key_name = if prompt.key_name.trim().is_empty() {
            i18n::string("dialogs.managed_key_delete.untitled_key")
        } else {
            prompt.key_name.clone()
        };
        let subtitle = i18n::string_args(
            "dialogs.managed_key_delete.message",
            &[("key_name", key_name.as_str())],
        );

        let entity_cancel = entity.clone();
        let entity_confirm = entity.clone();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "managed-key-delete-cancel",
                    i18n::string("dialogs.managed_key_delete.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    entity_cancel.update(cx, |this, cx| {
                        this.cancel_managed_key_delete(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "managed-key-delete-confirm",
                    i18n::string("dialogs.managed_key_delete.confirm"),
                    BasicDialogActionTone::Destructive,
                )
                .on_click(move |_, window, cx| {
                    entity_confirm.update(cx, |this, cx| {
                        this.confirm_managed_key_delete(window, cx);
                    });
                }),
            );

        render_basic_dialog(
            "managed-key-delete",
            i18n::string("dialogs.managed_key_delete.title"),
            Some(subtitle),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }

    fn render_profile_delete_prompt(
        &self,
        entity: Entity<AppView>,
        prompt: &PendingProfileDeleteState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let profile_name = if prompt.profile_name.trim().is_empty() {
            i18n::string("dialogs.profile_delete.untitled_profile")
        } else {
            prompt.profile_name.clone()
        };
        let subtitle = i18n::string_args(
            "dialogs.profile_delete.message",
            &[("profile_name", profile_name.as_str())],
        );

        let entity_cancel = entity.clone();
        let entity_confirm = entity.clone();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "profile-delete-cancel",
                    i18n::string("dialogs.profile_delete.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    entity_cancel.update(cx, |this, cx| {
                        this.cancel_profile_delete(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "profile-delete-confirm",
                    i18n::string("dialogs.profile_delete.confirm"),
                    BasicDialogActionTone::Destructive,
                )
                .on_click(move |_, window, cx| {
                    entity_confirm.update(cx, |this, cx| {
                        this.confirm_profile_delete(window, cx);
                    });
                }),
            );

        render_basic_dialog(
            "profile-delete",
            i18n::string("dialogs.profile_delete.title"),
            Some(subtitle),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }

    fn render_snippet_delete_prompt(
        &self,
        entity: Entity<AppView>,
        prompt: &PendingSnippetDeleteState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let description = if prompt.snippet_description.trim().is_empty() {
            i18n::string("dialogs.snippet_delete.untitled_snippet")
        } else {
            prompt.snippet_description.clone()
        };
        let subtitle = i18n::string_args(
            "dialogs.snippet_delete.message",
            &[("description", description.as_str())],
        );

        let entity_cancel = entity.clone();
        let entity_confirm = entity.clone();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "snippet-delete-cancel",
                    i18n::string("dialogs.snippet_delete.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    entity_cancel.update(cx, |this, cx| {
                        this.cancel_snippet_delete(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "snippet-delete-confirm",
                    i18n::string("dialogs.snippet_delete.confirm"),
                    BasicDialogActionTone::Destructive,
                )
                .on_click(move |_, _, cx| {
                    entity_confirm.update(cx, |this, cx| {
                        this.confirm_snippet_delete(cx);
                    });
                }),
            );

        render_basic_dialog(
            "snippet-delete",
            i18n::string("dialogs.snippet_delete.title"),
            Some(subtitle),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }

    fn render_port_forward_rule_delete_prompt(
        &self,
        entity: Entity<AppView>,
        prompt: &PendingPortForwardRuleDeleteState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let subtitle = i18n::string_args(
            "dialogs.forwarding_rule_delete.message",
            &[
                ("rule", prompt.rule_label.as_str()),
                ("profile", prompt.profile_label.as_str()),
            ],
        );

        let entity_cancel = entity.clone();
        let entity_confirm = entity.clone();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "port-forward-rule-delete-cancel",
                    i18n::string("dialogs.forwarding_rule_delete.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    entity_cancel.update(cx, |this, cx| {
                        this.cancel_port_forward_rule_removal(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "port-forward-rule-delete-confirm",
                    i18n::string("dialogs.forwarding_rule_delete.confirm"),
                    BasicDialogActionTone::Destructive,
                )
                .on_click(move |_, _, cx| {
                    entity_confirm.update(cx, |this, cx| {
                        this.confirm_port_forward_rule_removal(cx);
                    });
                }),
            );

        render_basic_dialog(
            "port-forward-rule-delete",
            i18n::string("dialogs.forwarding_rule_delete.title"),
            Some(subtitle),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }

    fn render_sync_direction_prompt(
        &self,
        entity: Entity<AppView>,
        _prompt: &PendingSyncDirectionState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let entity_cancel = entity.clone();
        let entity_pull = entity.clone();
        let entity_push = entity.clone();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "sync-direction-cancel",
                    i18n::string("settings.sync.dialogs.common.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    entity_cancel.update(cx, |this, cx| {
                        this.cancel_sync_direction_prompt(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "sync-direction-pull",
                    i18n::string("settings.sync.dialogs.direction.pull"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    entity_pull.update(cx, |this, cx| {
                        this.select_sync_now_pull(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "sync-direction-push",
                    i18n::string("settings.sync.dialogs.direction.push"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, window, cx| {
                    entity_push.update(cx, |this, cx| {
                        this.select_sync_now_push(window, cx);
                    });
                }),
            );

        render_basic_dialog(
            "sync-direction",
            i18n::string("settings.sync.dialogs.direction.title"),
            Some(i18n::string("settings.sync.dialogs.direction.message")),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }

    fn render_sync_pull_confirm_prompt(
        &self,
        entity: Entity<AppView>,
        prompt: &PendingSyncPullConfirmState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let entity_cancel = entity.clone();
        let entity_force_push = entity.clone();
        let entity_confirm = entity.clone();

        let message_key = match prompt.reason {
            SyncPullConfirmReason::Manual => "settings.sync.dialogs.pull_confirm.message",
            SyncPullConfirmReason::RemoteNewer => {
                "settings.sync.dialogs.pull_confirm.remote_newer_message"
            }
        };

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "sync-pull-confirm-cancel",
                    i18n::string("settings.sync.dialogs.common.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    entity_cancel.update(cx, |this, cx| {
                        this.cancel_sync_pull_confirm(cx);
                    });
                }),
            )
            .when(
                prompt.reason == SyncPullConfirmReason::RemoteNewer,
                |this| {
                    this.child(
                        basic_dialog_action_button(
                            "sync-pull-confirm-force-push",
                            i18n::string("settings.sync.dialogs.pull_confirm.force_push"),
                            BasicDialogActionTone::Default,
                        )
                        .on_click(move |_, window, cx| {
                            entity_force_push.update(cx, |this, cx| {
                                this.confirm_sync_force_push(window, cx);
                            });
                        }),
                    )
                },
            )
            .child(
                basic_dialog_action_button(
                    "sync-pull-confirm-accept",
                    i18n::string("settings.sync.dialogs.pull_confirm.confirm"),
                    BasicDialogActionTone::Destructive,
                )
                .on_click(move |_, window, cx| {
                    entity_confirm.update(cx, |this, cx| {
                        this.confirm_sync_pull(window, cx);
                    });
                }),
            );

        render_basic_dialog(
            "sync-pull-confirm",
            i18n::string("settings.sync.dialogs.pull_confirm.title"),
            Some(i18n::string(message_key)),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }

    fn render_local_vault_disable_confirm_prompt(
        &self,
        entity: Entity<AppView>,
        _prompt: &PendingLocalVaultDisableConfirmState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let entity_cancel = entity.clone();
        let entity_confirm = entity.clone();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "local-vault-disable-confirm-cancel",
                    i18n::string("settings.sync.dialogs.common.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    entity_cancel.update(cx, |this, cx| {
                        this.cancel_local_vault_disable_confirm(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "local-vault-disable-confirm-accept",
                    i18n::string("settings.sync.vault.dialogs.disable_confirm.confirm"),
                    BasicDialogActionTone::Destructive,
                )
                .on_click(move |_, _, cx| {
                    entity_confirm.update(cx, |this, cx| {
                        this.confirm_local_vault_disable(cx);
                    });
                }),
            );

        render_basic_dialog(
            "local-vault-disable-confirm",
            i18n::string("settings.sync.vault.dialogs.disable_confirm.title"),
            Some(i18n::string(
                "settings.sync.vault.dialogs.disable_confirm.message",
            )),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }

    fn render_local_data_reset_confirm_prompt(
        &self,
        entity: Entity<AppView>,
        _prompt: &PendingLocalDataResetConfirmState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let entity_cancel = entity.clone();
        let entity_confirm = entity.clone();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "local-data-reset-confirm-cancel",
                    i18n::string("dialogs.common.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    entity_cancel.update(cx, |this, cx| {
                        this.cancel_local_data_reset_confirm(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "local-data-reset-confirm-continue",
                    i18n::string("settings.about.reset_local.confirm.confirm"),
                    BasicDialogActionTone::Destructive,
                )
                .on_click(move |_, window, cx| {
                    entity_confirm.update(cx, |this, cx| {
                        this.continue_local_data_reset_confirm(window, cx);
                    });
                }),
            );

        render_basic_dialog(
            "local-data-reset-confirm",
            i18n::string("settings.about.reset_local.confirm.title"),
            Some(i18n::string("settings.about.reset_local.confirm.message")),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }

    fn render_keyboard_interactive_prompt(
        &self,
        entity: Entity<AppView>,
        challenge: &KbiChallenge,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let roles = settings::current_theme().material.roles;
        let title: SharedString = if challenge.name.is_empty() {
            i18n::string("prompts.authentication_challenge").into()
        } else {
            challenge.name.clone().into()
        };

        let entity_submit = entity.clone();

        let submit_button = basic_dialog_action_button(
            "keyboard-interactive-submit",
            i18n::string("dialogs.common.submit"),
            BasicDialogActionTone::Default,
        )
        .on_click(move |_, _, cx| {
            entity_submit.update(cx, |this, cx| {
                let responses: Vec<String> = this
                    .kbi_inputs
                    .iter()
                    .map(|input| input.read(cx).value().to_string())
                    .collect();
                let challenge = this.workspace_state.tabs.iter().find_map(|tab| {
                    tab.as_session()
                        .and_then(|session| session.pending_keyboard_interactive.clone())
                });
                let commands = this.workspace_state.tabs.iter().find_map(|tab| {
                    tab.as_session()
                        .filter(|s| s.pending_keyboard_interactive.is_some())
                        .and_then(|s| s.commands.clone())
                });
                if let Some(commands) = commands {
                    let _ = commands.respond_keyboard_interactive(responses);
                }
                for tab in &mut this.workspace_state.tabs {
                    if let TabKind::Session(session) = &mut tab.kind {
                        if session.pending_keyboard_interactive.is_some() {
                            session.pending_keyboard_interactive = None;
                            break;
                        }
                    }
                }
                if let Some(challenge) = challenge {
                    this.start_dialog_exit(
                        DialogOverlaySnapshot::KeyboardInteractive(challenge),
                        cx,
                    );
                }
                cx.notify();
            });
        });

        let cancel_button = basic_dialog_action_button(
            "keyboard-interactive-cancel",
            i18n::string("dialogs.common.cancel"),
            BasicDialogActionTone::Default,
        )
        .on_click(move |_, _, cx| {
            entity.update(cx, |this, cx| {
                let challenge = this.workspace_state.tabs.iter().find_map(|tab| {
                    tab.as_session()
                        .and_then(|session| session.pending_keyboard_interactive.clone())
                });
                for tab in &mut this.workspace_state.tabs {
                    if let TabKind::Session(session) = &mut tab.kind {
                        if session.pending_keyboard_interactive.is_some() {
                            session.pending_keyboard_interactive = None;
                            if let Some(commands) = &session.commands {
                                let _ = commands.close();
                            }
                            break;
                        }
                    }
                }
                if let Some(challenge) = challenge {
                    this.start_dialog_exit(
                        DialogOverlaySnapshot::KeyboardInteractive(challenge),
                        cx,
                    );
                }
                cx.notify();
            });
        });

        let body = v_flex()
            .w_full()
            .gap_4()
            .children(
                challenge
                    .prompts
                    .iter()
                    .enumerate()
                    .filter_map(|(i, prompt)| {
                        self.kbi_inputs.get(i).map(|input| {
                            v_flex()
                                .w_full()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(settings::scaled_font_size(11.0))
                                        .text_color(rgb(roles.on_surface_variant))
                                        .child(prompt.prompt.clone()),
                                )
                                .child(
                                    Input::new(input)
                                        .large()
                                        .w_full()
                                        .border_0()
                                        .rounded(px(14.0))
                                        .bg(rgb(roles.surface_container_low)),
                                )
                                .into_any_element()
                        })
                    }),
            )
            .into_any_element();

        render_basic_dialog(
            "keyboard-interactive",
            title.to_string(),
            (!challenge.instructions.is_empty()).then(|| challenge.instructions.clone()),
            Some(body),
            h_flex()
                .gap_2()
                .justify_end()
                .child(cancel_button)
                .child(submit_button)
                .into_any_element(),
            exit_progress,
        )
    }

    fn render_local_vault_passphrase_popup(
        &self,
        entity: Entity<AppView>,
        mode: LocalVaultPassphrasePopupMode,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let roles = settings::current_theme().material.roles;
        let operation_in_progress = self.local_vault_unlock_in_progress;
        let input = self
            .panel_forms
            .settings
            .local_vault_passphrase_input
            .clone();
        let confirmation_input = self
            .panel_forms
            .settings
            .local_vault_passphrase_confirmation_input
            .clone();
        let local_vault_auto_lock_duration_select = self
            .panel_forms
            .settings
            .local_vault_auto_lock_duration_select
            .clone();
        let entity_cancel = entity.clone();
        let entity_submit = entity.clone();
        let title = self.local_vault_passphrase_popup_title(mode);
        let requires_passphrase_confirmation = mode
            == LocalVaultPassphrasePopupMode::ChangePassphrase
            || (mode == LocalVaultPassphrasePopupMode::PrimaryAction
                && self.local_vault_status == LocalVaultStatus::Disabled);
        let popup_body = v_flex()
            .w_full()
            .gap_5()
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .text_color(rgb(roles.primary))
                    .child(Icon::new(AppIcon::Vault).size(px(128.0))),
            )
            .child(
                surface_secret_text_input_stack(
                    i18n::string("settings.sync.vault.passphrase.label"),
                    input.clone(),
                    TextInputSurface::Low,
                    gpui_component::Size::Large,
                    true,
                    operation_in_progress,
                    self.secret_reveal_icon(SecretRevealTarget::LocalVaultPassphrase),
                    {
                        let entity = entity.clone();
                        move |window, cx| {
                            entity.update(cx, |this, cx| {
                                this.toggle_secret_visibility(
                                    SecretRevealTarget::LocalVaultPassphrase,
                                    window,
                                    cx,
                                );
                            });
                        }
                    },
                ),
            )
            .when(requires_passphrase_confirmation, |this| {
                this.child(
                    surface_secret_text_input_stack(
                        i18n::string("settings.sync.vault.confirm_passphrase.label"),
                        confirmation_input.clone(),
                        TextInputSurface::Low,
                        gpui_component::Size::Large,
                        true,
                        operation_in_progress,
                        self.secret_reveal_icon(
                            SecretRevealTarget::LocalVaultPassphraseConfirmation,
                        ),
                        {
                            let entity = entity.clone();
                            move |window, cx| {
                                entity.update(cx, |this, cx| {
                                    this.toggle_secret_visibility(
                                        SecretRevealTarget::LocalVaultPassphraseConfirmation,
                                        window,
                                        cx,
                                    );
                                });
                            }
                        },
                    ),
                )
            })
            .when(
                mode == LocalVaultPassphrasePopupMode::PrimaryAction,
                |this| {
                    this.child(
                        v_flex()
                            .w_full()
                            .gap_2()
                            .child(field_label(
                                i18n::string("settings.sync.vault.auto_lock_duration.label"),
                                false,
                            ))
                            .child(
                                Select::new(&local_vault_auto_lock_duration_select)
                                    .large()
                                    .w_full()
                                    .rounded(px(14.0))
                                    .border_0()
                                    .bg(rgb(roles.surface_container_low))
                                    .disabled(operation_in_progress),
                            ),
                    )
                },
            )
            .into_any_element();

        let actions = h_flex()
            .w_full()
            .justify_end()
            .gap_3()
            .child(
                Button::new("local-vault-passphrase-popup-cancel")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .disabled(operation_in_progress)
                    .text_color(rgb(roles.on_surface_variant))
                    .label(i18n::string("dialogs.common.cancel"))
                    .on_click(move |_, window, cx| {
                        entity_cancel.update(cx, |this, cx| {
                            this.close_local_vault_passphrase_popup(window, cx);
                        });
                    }),
            )
            .child(if operation_in_progress {
                div()
                    .id("local-vault-passphrase-popup-submit-spinner")
                    .min_w(px(116.0))
                    .min_h(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(crate::ui::components::md3_spinner(18.0))
                    .into_any_element()
            } else {
                Button::new("local-vault-passphrase-popup-submit")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .disabled(operation_in_progress)
                    .text_color(rgb(roles.primary))
                    .label(self.local_vault_passphrase_popup_title(mode))
                    .on_click(move |_, window, cx| {
                        entity_submit.update(cx, |this, cx| {
                            this.submit_local_vault_passphrase_popup_action(mode, window, cx);
                        });
                    })
                    .into_any_element()
            })
            .into_any_element();

        render_bottom_popup(
            bottom_popup_panel(title, None, Some(popup_body), actions),
            "local-vault-passphrase",
            exit_progress,
            move |window, cx| {
                entity.update(cx, |this, cx| {
                    this.close_local_vault_passphrase_popup(window, cx);
                });
            },
        )
    }

    fn render_sync_passphrase_popup(
        &self,
        entity: Entity<AppView>,
        _popup: PendingSyncPassphrasePopupState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let roles = settings::current_theme().material.roles;
        let operation_in_progress = self.sync_passphrase_operation_in_progress();
        let save_in_progress = self.sync_passphrase_save_in_progress();
        let input = self.panel_forms.settings.sync_passphrase_input.clone();
        let confirmation_input = self
            .panel_forms
            .settings
            .sync_passphrase_confirmation_input
            .clone();
        let entity_cancel = entity.clone();
        let entity_submit = entity.clone();
        let title = self.sync_passphrase_action_label();
        let popup_body = v_flex()
            .w_full()
            .gap_5()
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .text_color(rgb(roles.primary))
                    .child(Icon::new(AppIcon::Key).size(px(128.0))),
            )
            .child(
                surface_secret_text_input_stack(
                    i18n::string("settings.sync.encryption.passphrase.label"),
                    input.clone(),
                    TextInputSurface::Low,
                    gpui_component::Size::Large,
                    true,
                    operation_in_progress,
                    self.secret_reveal_icon(SecretRevealTarget::SyncPassphrase),
                    {
                        let entity = entity.clone();
                        move |window, cx| {
                            entity.update(cx, |this, cx| {
                                this.toggle_secret_visibility(
                                    SecretRevealTarget::SyncPassphrase,
                                    window,
                                    cx,
                                );
                            });
                        }
                    },
                ),
            )
            .child(
                surface_secret_text_input_stack(
                    i18n::string(
                        "settings.sync.encryption.passphrase.confirm_passphrase.label",
                    ),
                    confirmation_input.clone(),
                    TextInputSurface::Low,
                    gpui_component::Size::Large,
                    true,
                    operation_in_progress,
                    self.secret_reveal_icon(SecretRevealTarget::SyncPassphraseConfirmation),
                    {
                        let entity = entity.clone();
                        move |window, cx| {
                            entity.update(cx, |this, cx| {
                                this.toggle_secret_visibility(
                                    SecretRevealTarget::SyncPassphraseConfirmation,
                                    window,
                                    cx,
                                );
                            });
                        }
                    },
                ),
            )
            .into_any_element();

        let actions = h_flex()
            .w_full()
            .justify_end()
            .gap_3()
            .child(
                Button::new("sync-passphrase-popup-cancel")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .disabled(operation_in_progress)
                    .text_color(rgb(roles.on_surface_variant))
                    .label(i18n::string("dialogs.common.cancel"))
                    .on_click(move |_, window, cx| {
                        entity_cancel.update(cx, |this, cx| {
                            this.close_sync_passphrase_popup(window, cx);
                        });
                    }),
            )
            .child(if save_in_progress {
                div()
                    .id("sync-passphrase-popup-submit-spinner")
                    .min_w(px(116.0))
                    .min_h(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(crate::ui::components::md3_spinner(18.0))
                    .into_any_element()
            } else {
                Button::new("sync-passphrase-popup-submit")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .disabled(operation_in_progress)
                    .text_color(rgb(roles.primary))
                    .label(title.clone())
                    .on_click(move |_, window, cx| {
                        entity_submit.update(cx, |this, cx| {
                            this.submit_sync_passphrase_popup_action(window, cx);
                        });
                    })
                    .into_any_element()
            })
            .into_any_element();

        render_bottom_popup(
            bottom_popup_panel(title, None, Some(popup_body), actions),
            "sync-passphrase",
            exit_progress,
            move |window, cx| {
                entity.update(cx, |this, cx| {
                    this.close_sync_passphrase_popup(window, cx);
                });
            },
        )
    }

    fn render_sync_passphrase_clear_confirm_popup(
        &self,
        entity: Entity<AppView>,
        _popup: PendingSyncPassphraseClearConfirmPopupState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let roles = settings::current_theme().material.roles;
        let operation_in_progress = self.sync_passphrase_operation_in_progress();
        let entity_cancel = entity.clone();
        let entity_submit = entity.clone();
        let popup_body = v_flex()
            .w_full()
            .gap_5()
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .text_color(rgb(roles.error))
                    .child(Icon::new(AppIcon::Trash).size(px(128.0))),
            )
            .child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .text_size(settings::scaled_font_size(14.0))
                    .line_height(settings::scaled_line_height(20.0))
                    .text_color(rgb(roles.on_surface_variant))
                    .child(i18n::string(
                        "settings.sync.encryption.passphrase.clear_confirm.message",
                    )),
            )
            .into_any_element();

        let actions = h_flex()
            .w_full()
            .justify_end()
            .gap_3()
            .child(
                Button::new("sync-passphrase-clear-confirm-cancel")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .disabled(operation_in_progress)
                    .text_color(rgb(roles.on_surface_variant))
                    .label(i18n::string("dialogs.common.cancel"))
                    .on_click(move |_, _window, cx| {
                        entity_cancel.update(cx, |this, cx| {
                            this.close_sync_passphrase_clear_confirm_popup(cx);
                        });
                    }),
            )
            .child(
                Button::new("sync-passphrase-clear-confirm-submit")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .disabled(operation_in_progress)
                    .text_color(rgb(roles.error))
                    .label(i18n::string(
                        "settings.sync.encryption.passphrase.clear_confirm.confirm",
                    ))
                    .on_click(move |_, window, cx| {
                        entity_submit.update(cx, |this, cx| {
                            this.submit_sync_passphrase_clear_confirm_popup_action(window, cx);
                        });
                    }),
            )
            .into_any_element();

        render_bottom_popup(
            bottom_popup_panel(
                i18n::string("settings.sync.encryption.passphrase.clear_confirm.title"),
                None,
                Some(popup_body),
                actions,
            ),
            "sync-passphrase-clear-confirm",
            exit_progress,
            move |_window, cx| {
                entity.update(cx, |this, cx| {
                    this.close_sync_passphrase_clear_confirm_popup(cx);
                });
            },
        )
    }

    fn render_local_data_reset_confirmation_popup(
        &self,
        entity: Entity<AppView>,
        _popup: PendingLocalDataResetConfirmationPopupState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let roles = settings::current_theme().material.roles;
        let input = self
            .panel_forms
            .settings
            .local_data_reset_confirmation_input
            .clone();
        let entity_cancel = entity.clone();
        let entity_submit = entity.clone();
        let popup_body = v_flex()
            .w_full()
            .gap_5()
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .text_color(rgb(roles.error))
                    .child(Icon::new(AppIcon::Trash).size(px(128.0))),
            )
            .child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .text_size(settings::scaled_font_size(14.0))
                    .line_height(settings::scaled_line_height(20.0))
                    .text_color(rgb(roles.on_surface_variant))
                    .child(i18n::string("settings.about.reset_local.popup.description")),
            )
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(field_label(
                        i18n::string("settings.about.reset_local.popup.field_label"),
                        true,
                    ))
                    .child(
                        surface_text_input(&input, TextInputSurface::Low)
                            .large()
                            .text_color(rgb(roles.on_surface)),
                    ),
            )
            .into_any_element();

        let actions = h_flex()
            .w_full()
            .justify_end()
            .gap_3()
            .child(
                Button::new("local-data-reset-popup-cancel")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .text_color(rgb(roles.on_surface_variant))
                    .label(i18n::string("dialogs.common.cancel"))
                    .on_click(move |_, window, cx| {
                        entity_cancel.update(cx, |this, cx| {
                            this.close_local_data_reset_confirmation_popup(window, cx);
                        });
                    }),
            )
            .child(
                Button::new("local-data-reset-popup-submit")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .text_color(rgb(roles.error))
                    .label(i18n::string("settings.about.reset_local.popup.confirm"))
                    .on_click(move |_, window, cx| {
                        entity_submit.update(cx, |this, cx| {
                            this.submit_local_data_reset_confirmation_popup_action(window, cx);
                        });
                    }),
            )
            .into_any_element();

        render_bottom_popup(
            bottom_popup_panel(
                i18n::string("settings.about.reset_local.popup.title"),
                None,
                Some(popup_body),
                actions,
            ),
            "local-data-reset-confirmation",
            exit_progress,
            move |window, cx| {
                entity.update(cx, |this, cx| {
                    this.close_local_data_reset_confirmation_popup(window, cx);
                });
            },
        )
    }

    fn render_exiting_dialog_overlay(
        &self,
        entity: Entity<AppView>,
        snapshot: DialogOverlaySnapshot,
        exit_progress: f32,
    ) -> gpui::AnyElement {
        match snapshot {
            DialogOverlaySnapshot::HostKey(prompt) => {
                self.render_trusted_host_key_prompt(entity, &prompt, Some(exit_progress))
            }
            DialogOverlaySnapshot::KeyboardInteractive(challenge) => {
                self.render_keyboard_interactive_prompt(entity, &challenge, Some(exit_progress))
            }
            DialogOverlaySnapshot::ProfileDelete(prompt) => {
                self.render_profile_delete_prompt(entity, &prompt, Some(exit_progress))
            }
            DialogOverlaySnapshot::ManagedKeyDelete(prompt) => {
                self.render_managed_key_delete_prompt(entity, &prompt, Some(exit_progress))
            }
            DialogOverlaySnapshot::KnownHostDelete(prompt) => {
                self.render_trusted_known_host_delete_prompt(entity, &prompt, Some(exit_progress))
            }
            DialogOverlaySnapshot::SnippetDelete(prompt) => {
                self.render_snippet_delete_prompt(entity, &prompt, Some(exit_progress))
            }
            DialogOverlaySnapshot::PortForwardRuleDelete(prompt) => {
                self.render_port_forward_rule_delete_prompt(entity, &prompt, Some(exit_progress))
            }
            DialogOverlaySnapshot::SyncDirection(prompt) => {
                self.render_sync_direction_prompt(entity, &prompt, Some(exit_progress))
            }
            DialogOverlaySnapshot::SyncPullConfirm(prompt) => {
                self.render_sync_pull_confirm_prompt(entity, &prompt, Some(exit_progress))
            }
            DialogOverlaySnapshot::LocalVaultDisableConfirm(prompt) => {
                self.render_local_vault_disable_confirm_prompt(entity, &prompt, Some(exit_progress))
            }
            DialogOverlaySnapshot::LocalDataResetConfirm(prompt) => {
                self.render_local_data_reset_confirm_prompt(entity, &prompt, Some(exit_progress))
            }
            DialogOverlaySnapshot::LocalDataResetConfirmationPopup(popup) => {
                self.render_local_data_reset_confirmation_popup(entity, popup, Some(exit_progress))
            }
            DialogOverlaySnapshot::SyncPassphraseClearConfirmPopup(popup) => {
                self.render_sync_passphrase_clear_confirm_popup(entity, popup, Some(exit_progress))
            }
            DialogOverlaySnapshot::SyncPassphrasePopup(popup) => {
                self.render_sync_passphrase_popup(entity, popup, Some(exit_progress))
            }
            DialogOverlaySnapshot::LocalVaultPassphrasePopup(mode) => {
                self.render_local_vault_passphrase_popup(entity, mode, Some(exit_progress))
            }
            DialogOverlaySnapshot::SftpPrompt { tab_id, prompt } => {
                self.render_sftp_prompt_overlay(entity, tab_id, &prompt, Some(exit_progress))
            }
        }
    }
}
