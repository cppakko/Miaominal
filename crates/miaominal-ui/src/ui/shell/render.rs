use super::state::{
    PendingAiProviderPopupState, PendingChatSessionRenameState, PendingLocalDataResetConfirmState,
    PendingLocalDataResetConfirmationPopupState, PendingSyncPassphraseClearConfirmPopupState,
    PendingSyncPassphrasePopupState, PendingSyncProviderConfigPopupState,
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
struct PrimaryViewTransitionRenderState {
    from: PrimaryViewKind,
    to: PrimaryViewKind,
    visibility: f32,
    animating: bool,
}

#[derive(Clone, Copy)]
struct PrimarySurfaceRenderState {
    has_active_session: bool,
    has_active_sftp_tab: bool,
    show_host_editor_sidebar: bool,
    show_port_forward_editor_sidebar: bool,
    show_keychain_editor_sidebar: bool,
    show_snippets_editor_sidebar: bool,
    show_known_hosts_sidebar: bool,
}

#[derive(Clone, Copy)]
struct ShellBodyRenderState {
    show_sidebar: bool,
    page_editor_sidebar: Option<PageEditorSidebarRenderState>,
    primary_surface: PrimarySurfaceRenderState,
}

const PRIMARY_VIEW_GUTTER: f32 = 8.0;

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let window_title = self.window_title();
        if window.window_title() != window_title {
            window.set_window_title(&window_title);
        }

        let entity = cx.entity();
        let roles = miaominal_settings::current_theme().material.roles;
        let bottom_popup_viewport_height = f32::from(window.bounds().size.height);
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
        let desired_primary_view =
            self.desired_primary_view_kind(has_active_session, has_active_sftp_tab);
        let primary_view_transition =
            self.primary_view_transition_render_state(desired_primary_view, window);
        let primary_view_animating = primary_view_transition.animating;
        let root_right_gutter = self.primary_view_root_right_gutter(primary_view_transition);

        let pending_host_key = self.pending_host_key_prompt();
        let pending_kbi = self.pending_keyboard_interactive_prompt();
        let pending_profile_delete = self.pending_profile_delete_prompt();
        let pending_managed_key_delete = self.pending_managed_key_delete_prompt();
        let pending_known_host_delete = self.pending_known_host_delete_prompt();
        let pending_snippet_delete = self.pending_snippet_delete_prompt();
        let pending_port_forward_rule_delete = self.pending_port_forward_rule_delete_prompt();
        let pending_chat_session_delete = self.pending_chat_session_delete_prompt();
        let pending_chat_session_rename = self.pending_chat_session_rename_prompt();
        let pending_sync_direction = self.pending_sync_direction_prompt();
        let pending_sync_pull_confirm = self.pending_sync_pull_confirm_prompt();
        let pending_local_vault_disable_confirm = self.pending_local_vault_disable_confirm_prompt();
        let pending_local_data_reset_confirm = self.pending_local_data_reset_confirm_prompt();
        let pending_local_data_reset_confirmation_popup =
            self.pending_local_data_reset_confirmation_popup();
        let pending_sync_passphrase_clear_confirm_popup =
            self.pending_sync_passphrase_clear_confirm_popup();
        let pending_sync_passphrase_popup = self.pending_sync_passphrase_popup();
        let pending_ai_provider_popup = self.pending_ai_provider_popup();
        let pending_sync_provider_config_popup = self.pending_sync_provider_config_popup();
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
        let show_known_hosts_sidebar = self.panels.selected_known_host.is_some()
            && !has_active_session
            && !has_active_sftp_tab
            && self.panel_view.sidebar_section == SidebarSection::KnownHosts;
        let page_editor_sidebar =
            if primary_view_animating || has_active_session || has_active_sftp_tab {
                self.clear_page_editor_sidebar_transition_state();
                None
            } else {
                self.page_editor_sidebar_render_state(
                    self.desired_page_editor_sidebar_kind(
                        show_host_editor_sidebar,
                        show_port_forward_editor_sidebar,
                        show_snippets_editor_sidebar,
                        show_keychain_editor_sidebar,
                        show_known_hosts_sidebar,
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
        let show_known_hosts_sidebar = matches!(
            page_editor_sidebar,
            Some(sidebar) if sidebar.kind == PageEditorSidebarKind::KnownHosts
        );
        let shell_body = if primary_view_transition.animating {
            self.render_primary_view_transition(primary_view_transition, entity.clone(), window, cx)
        } else {
            self.render_primary_view_shell(
                desired_primary_view,
                entity.clone(),
                page_editor_sidebar,
                PrimarySurfaceRenderState {
                    has_active_session,
                    has_active_sftp_tab,
                    show_host_editor_sidebar,
                    show_port_forward_editor_sidebar,
                    show_snippets_editor_sidebar,
                    show_keychain_editor_sidebar,
                    show_known_hosts_sidebar,
                },
                window,
                cx,
            )
        };

        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(rgb(roles.surface_container))
            .pr(px(root_right_gutter))
            .child(self.render_top_bar(entity.clone(), window))
            .child(shell_body)
            .child(self.render_status_footer(entity.clone()))
            .when_some(pending_host_key, |this, prompt| {
                this.child(self.render_trusted_host_key_prompt(
                    entity.clone(),
                    &prompt,
                    None,
                    bottom_popup_viewport_height,
                ))
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
            .when_some(pending_chat_session_delete, |this, prompt| {
                this.child(self.render_chat_session_delete_prompt(entity.clone(), &prompt, None))
            })
            .when_some(pending_chat_session_rename, |this, prompt| {
                this.child(self.render_chat_session_rename_prompt(entity.clone(), &prompt, None))
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
                        bottom_popup_viewport_height,
                    ))
                },
            )
            .when_some(
                pending_sync_passphrase_clear_confirm_popup,
                |this, prompt| {
                    this.child(self.render_sync_passphrase_clear_confirm_popup(
                        entity.clone(),
                        prompt,
                        None,
                        bottom_popup_viewport_height,
                    ))
                },
            )
            .when_some(pending_sync_passphrase_popup, |this, prompt| {
                this.child(self.render_sync_passphrase_popup(
                    entity.clone(),
                    prompt,
                    None,
                    bottom_popup_viewport_height,
                ))
            })
            .when_some(pending_ai_provider_popup, |this, popup| {
                this.child(self.render_ai_provider_popup(
                    entity.clone(),
                    popup,
                    None,
                    bottom_popup_viewport_height,
                ))
            })
            .when_some(pending_sync_provider_config_popup, |this, popup| {
                this.child(self.render_sync_provider_config_popup(
                    entity.clone(),
                    popup,
                    None,
                    bottom_popup_viewport_height,
                ))
            })
            .when_some(pending_local_vault_passphrase_popup, |this, mode| {
                this.child(self.render_local_vault_passphrase_popup(
                    entity.clone(),
                    mode,
                    None,
                    bottom_popup_viewport_height,
                ))
            })
            .when_some(pending_sftp_prompt, |this, prompt| {
                let (tab_id, prompt) = prompt;
                this.child(self.render_sftp_prompt_overlay(entity.clone(), tab_id, &prompt, None))
            })
            .children(exiting_dialogs.into_iter().map(|(snapshot, progress)| {
                self.render_exiting_dialog_overlay(
                    entity.clone(),
                    snapshot,
                    progress,
                    bottom_popup_viewport_height,
                )
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

    fn desired_primary_view_kind(
        &self,
        has_active_session: bool,
        has_active_sftp_tab: bool,
    ) -> PrimaryViewKind {
        if has_active_session && let Some(tab_id) = self.active_terminal_tab_id() {
            return PrimaryViewKind::Terminal(tab_id);
        }

        if has_active_sftp_tab && let Some(tab_id) = self.active_sftp_tab_id() {
            return PrimaryViewKind::Sftp(tab_id);
        }

        PrimaryViewKind::Sidebar(self.panel_view.sidebar_section)
    }

    fn primary_view_exists(&self, view: PrimaryViewKind) -> bool {
        match view {
            PrimaryViewKind::Sidebar(_) => true,
            PrimaryViewKind::Terminal(tab_id) => self.workspace_state.tabs.iter().any(|tab| {
                tab.id == tab_id
                    && tab
                        .as_session()
                        .is_some_and(|session| session.purpose == SessionPurpose::Terminal)
            }),
            PrimaryViewKind::Sftp(tab_id) => self
                .workspace_state
                .tabs
                .iter()
                .any(|tab| tab.id == tab_id && tab.as_sftp().is_some()),
        }
    }

    fn primary_view_tab_index(&self, view: PrimaryViewKind) -> Option<usize> {
        match view {
            PrimaryViewKind::Terminal(tab_id) | PrimaryViewKind::Sftp(tab_id) => self
                .workspace_state
                .tabs
                .iter()
                .position(|tab| tab.id == tab_id),
            PrimaryViewKind::Sidebar(_) => None,
        }
    }

    fn primary_view_transition_axis_direction(
        &self,
        from: PrimaryViewKind,
        to: PrimaryViewKind,
    ) -> f32 {
        match (
            self.primary_view_tab_index(from),
            self.primary_view_tab_index(to),
        ) {
            (Some(from_index), Some(to_index)) if to_index < from_index => -1.0,
            (Some(from_index), Some(to_index)) if to_index > from_index => 1.0,
            _ => match (from, to) {
                (PrimaryViewKind::Terminal(_), PrimaryViewKind::Sftp(_)) => 1.0,
                (PrimaryViewKind::Sftp(_), PrimaryViewKind::Terminal(_)) => -1.0,
                _ => 1.0,
            },
        }
    }

    fn should_animate_primary_view_transition(
        &self,
        from: PrimaryViewKind,
        to: PrimaryViewKind,
    ) -> bool {
        !matches!(
            (from, to),
            (PrimaryViewKind::Terminal(_), PrimaryViewKind::Terminal(_))
                | (PrimaryViewKind::Sftp(_), PrimaryViewKind::Sftp(_))
        )
    }

    fn primary_view_root_right_gutter(&self, transition: PrimaryViewTransitionRenderState) -> f32 {
        let from_uses_sidebar = matches!(transition.from, PrimaryViewKind::Sidebar(_));
        let to_uses_sidebar = matches!(transition.to, PrimaryViewKind::Sidebar(_));

        match (from_uses_sidebar, to_uses_sidebar) {
            (true, true) => PRIMARY_VIEW_GUTTER,
            (true, false) => PRIMARY_VIEW_GUTTER * (1.0 - transition.visibility),
            (false, true) => PRIMARY_VIEW_GUTTER * transition.visibility,
            (false, false) => 0.0,
        }
    }

    fn finish_primary_view_transition(&mut self, transition: PrimaryViewTransition) {
        if matches!(
            (transition.from, transition.to),
            (
                PrimaryViewKind::Sidebar(SidebarSection::Hosts),
                PrimaryViewKind::Terminal(_)
            )
        ) {
            self.editors.host_editor_open = false;
            self.editors.host_editor_is_new = false;
        }

        self.workspace_state.primary_view_transition = None;
    }

    fn primary_view_transition_render_state(
        &mut self,
        desired: PrimaryViewKind,
        window: &mut Window,
    ) -> PrimaryViewTransitionRenderState {
        let now = Instant::now();
        let duration = super::support::CONTAINER_TRANSITION_DURATION;

        if self.workspace_state.visible_primary_view.is_none() {
            self.workspace_state.visible_primary_view = Some(desired);
        }

        if self.workspace_state.visible_primary_view != Some(desired) {
            let from = self
                .workspace_state
                .primary_view_transition
                .map(|transition| transition.to)
                .or(self.workspace_state.visible_primary_view)
                .unwrap_or(desired);
            self.workspace_state.visible_primary_view = Some(desired);
            if self.should_animate_primary_view_transition(from, desired) {
                self.workspace_state.primary_view_transition = Some(PrimaryViewTransition {
                    from,
                    to: desired,
                    started_at: now,
                    duration,
                });
            } else {
                self.workspace_state.primary_view_transition = None;
            }
        }

        let Some(transition) = self.workspace_state.primary_view_transition else {
            return PrimaryViewTransitionRenderState {
                from: desired,
                to: desired,
                visibility: 1.0,
                animating: false,
            };
        };

        if !self.primary_view_exists(transition.from) || !self.primary_view_exists(transition.to) {
            self.workspace_state.primary_view_transition = None;
            self.workspace_state.visible_primary_view = Some(desired);
            return PrimaryViewTransitionRenderState {
                from: desired,
                to: desired,
                visibility: 1.0,
                animating: false,
            };
        }

        let duration_seconds = transition.duration.as_secs_f32();
        if duration_seconds <= f32::EPSILON {
            self.finish_primary_view_transition(transition);
            return PrimaryViewTransitionRenderState {
                from: desired,
                to: desired,
                visibility: 1.0,
                animating: false,
            };
        }

        let elapsed = now.saturating_duration_since(transition.started_at);
        let progress = (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0);
        let eased = progress * progress * (3.0 - 2.0 * progress);

        if progress >= 1.0 {
            self.finish_primary_view_transition(transition);
            return PrimaryViewTransitionRenderState {
                from: desired,
                to: desired,
                visibility: 1.0,
                animating: false,
            };
        }

        window.request_animation_frame();
        PrimaryViewTransitionRenderState {
            from: transition.from,
            to: transition.to,
            visibility: eased,
            animating: true,
        }
    }

    fn render_sidebar_primary_panel(
        &mut self,
        section: SidebarSection,
        entity: Entity<Self>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match section {
            SidebarSection::Hosts => self.render_hosts_page(entity, cx),
            SidebarSection::Keychain => self.render_keychain_page(entity, cx),
            SidebarSection::PortForwarding => self.render_forward_page(entity, cx),
            SidebarSection::KnownHosts => self.render_trusted_page(entity, cx),
            SidebarSection::Settings => self.render_settings_page(entity),
            SidebarSection::Snippets => self.render_snippets_page(entity, cx),
        }
    }

    fn render_primary_view_panel(
        &mut self,
        view: PrimaryViewKind,
        entity: Entity<Self>,
        current_surface: Option<PrimarySurfaceRenderState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (AnyElement, PrimarySurfaceRenderState) {
        match view {
            PrimaryViewKind::Sidebar(section) => {
                let mut render_state = current_surface.unwrap_or(PrimarySurfaceRenderState {
                    has_active_session: false,
                    has_active_sftp_tab: false,
                    show_host_editor_sidebar: false,
                    show_port_forward_editor_sidebar: false,
                    show_keychain_editor_sidebar: false,
                    show_snippets_editor_sidebar: false,
                    show_known_hosts_sidebar: false,
                });
                render_state.has_active_session = false;
                render_state.has_active_sftp_tab = false;
                (
                    self.render_sidebar_primary_panel(section, entity, cx),
                    render_state,
                )
            }
            PrimaryViewKind::Terminal(tab_id) => (
                self.render_terminal_page_for_tab(tab_id, window, cx)
                    .unwrap_or_else(|| self.render_terminal_page(window, cx)),
                PrimarySurfaceRenderState {
                    has_active_session: true,
                    has_active_sftp_tab: false,
                    show_host_editor_sidebar: false,
                    show_port_forward_editor_sidebar: false,
                    show_keychain_editor_sidebar: false,
                    show_snippets_editor_sidebar: false,
                    show_known_hosts_sidebar: false,
                },
            ),
            PrimaryViewKind::Sftp(tab_id) => (
                self.render_sftp_page_for_tab(entity, tab_id, cx),
                PrimarySurfaceRenderState {
                    has_active_session: false,
                    has_active_sftp_tab: true,
                    show_host_editor_sidebar: false,
                    show_port_forward_editor_sidebar: false,
                    show_keychain_editor_sidebar: false,
                    show_snippets_editor_sidebar: false,
                    show_known_hosts_sidebar: false,
                },
            ),
        }
    }

    fn render_primary_view_shell(
        &mut self,
        view: PrimaryViewKind,
        entity: Entity<Self>,
        page_editor_sidebar: Option<PageEditorSidebarRenderState>,
        current_surface: PrimarySurfaceRenderState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (primary_panel, primary_surface) =
            self.render_primary_view_panel(view, entity.clone(), Some(current_surface), window, cx);
        self.render_shell_body(
            entity,
            primary_panel,
            ShellBodyRenderState {
                show_sidebar: matches!(view, PrimaryViewKind::Sidebar(_)),
                page_editor_sidebar,
                primary_surface,
            },
            cx,
        )
    }

    fn clear_page_editor_sidebar_transition_state(&mut self) {
        self.shell_state.page_editor_sidebar_transition = None;
        self.shell_state.visible_page_editor_sidebar = None;
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
        show_known_hosts_sidebar: bool,
    ) -> Option<PageEditorSidebarKind> {
        if show_host_editor_sidebar {
            Some(PageEditorSidebarKind::Hosts)
        } else if show_port_forward_editor_sidebar {
            Some(PageEditorSidebarKind::PortForwarding)
        } else if show_snippets_editor_sidebar {
            Some(PageEditorSidebarKind::Snippets)
        } else if show_keychain_editor_sidebar {
            Some(PageEditorSidebarKind::Keychain)
        } else if show_known_hosts_sidebar {
            Some(PageEditorSidebarKind::KnownHosts)
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
        render_state: ShellBodyRenderState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ShellBodyRenderState {
            show_sidebar,
            page_editor_sidebar,
            primary_surface,
        } = render_state;
        let roles = miaominal_settings::current_theme().material.roles;

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
            .when(
                primary_surface.has_active_session || primary_surface.has_active_sftp_tab,
                |this| this.pr_2().pl_2(),
            )
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
                        primary_surface,
                    ))
                    .when_some(page_editor_sidebar, |this, sidebar| {
                        this.child(self.render_page_editor_sidebar(sidebar, entity.clone(), cx))
                    }),
            )
            .into_any_element()
    }

    fn render_primary_view_transition(
        &mut self,
        transition: PrimaryViewTransitionRenderState,
        entity: Entity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let progress = transition.visibility;
        let from_uses_sidebar = matches!(transition.from, PrimaryViewKind::Sidebar(_));
        let to_uses_sidebar = matches!(transition.to, PrimaryViewKind::Sidebar(_));
        let full_surface_to_full_surface = !from_uses_sidebar && !to_uses_sidebar;
        let sidebar_visibility = match (from_uses_sidebar, to_uses_sidebar) {
            (true, true) => 1.0,
            (true, false) => 1.0 - progress,
            (false, true) => progress,
            (false, false) => 0.0,
        };
        let full_surface_gutter = match (from_uses_sidebar, to_uses_sidebar) {
            (true, true) => 0.0,
            (true, false) => PRIMARY_VIEW_GUTTER * progress,
            (false, true) => PRIMARY_VIEW_GUTTER * (1.0 - progress),
            (false, false) => PRIMARY_VIEW_GUTTER,
        };

        let (incoming_panel, incoming_state) =
            self.render_primary_view_panel(transition.to, entity.clone(), None, window, cx);
        let (outgoing_panel, outgoing_state) =
            self.render_primary_view_panel(transition.from, entity.clone(), None, window, cx);
        let incoming_surface =
            self.render_primary_surface_layer(entity.clone(), incoming_panel, incoming_state);
        let outgoing_surface =
            self.render_primary_surface_layer(entity.clone(), outgoing_panel, outgoing_state);
        let incoming_opacity = if full_surface_to_full_surface {
            0.7 + progress * 0.3
        } else {
            0.82 + progress * 0.18
        };
        let outgoing_opacity = 1.0 - progress;
        let axis_direction = if full_surface_to_full_surface {
            self.primary_view_transition_axis_direction(transition.from, transition.to)
        } else {
            0.0
        };
        let incoming_x_shift = axis_direction * (1.0 - progress) * 22.0;
        let outgoing_x_shift = axis_direction * progress * -22.0;
        let incoming_y_shift = if full_surface_to_full_surface {
            0.0
        } else {
            (1.0 - progress) * 8.0
        };
        let outgoing_y_shift = if full_surface_to_full_surface {
            0.0
        } else {
            progress * -10.0
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
            .when(sidebar_visibility > 0.0, |this| {
                this.child(
                    div()
                        .relative()
                        .h_full()
                        .w(px(LEFT_RAIL_WIDTH * sidebar_visibility))
                        .min_w(px(0.0))
                        .flex_shrink_0()
                        .overflow_hidden()
                        .child(
                            div()
                                .absolute()
                                .top(px(0.0))
                                .left(px(0.0))
                                .bottom(px(0.0))
                                .w(px(LEFT_RAIL_WIDTH))
                                .opacity(sidebar_visibility)
                                .child(self.render_sidebar(entity.clone())),
                        ),
                )
            })
            .pl(px(full_surface_gutter))
            .pr(px(full_surface_gutter))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .rounded(px(16.0))
                    .overflow_hidden()
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
                                    .top(px(incoming_y_shift))
                                    .right(px(-incoming_x_shift))
                                    .bottom(px(-incoming_y_shift))
                                    .left(px(incoming_x_shift))
                                    .opacity(incoming_opacity)
                                    .child(incoming_surface),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top(px(outgoing_y_shift))
                                    .right(px(-outgoing_x_shift))
                                    .bottom(px(-outgoing_y_shift))
                                    .left(px(outgoing_x_shift))
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
            PageEditorSidebarKind::KnownHosts => self
                .render_trusted_known_host_sidebar(entity)
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

    fn render_primary_surface_layer(
        &self,
        entity: Entity<Self>,
        panel: AnyElement,
        render_state: PrimarySurfaceRenderState,
    ) -> AnyElement {
        let PrimarySurfaceRenderState {
            has_active_session,
            has_active_sftp_tab,
            show_host_editor_sidebar,
            show_port_forward_editor_sidebar,
            show_keychain_editor_sidebar,
            show_snippets_editor_sidebar,
            show_known_hosts_sidebar,
        } = render_state;

        let roles = miaominal_settings::current_theme().material.roles;

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
            .when(
                self.panel_view.sidebar_section == SidebarSection::KnownHosts
                    && !has_active_session
                    && !has_active_sftp_tab
                    && !show_known_hosts_sidebar,
                |this| {
                    this.child(
                        div()
                            .absolute()
                            .right(px(28.0))
                            .bottom(px(28.0))
                            .child(self.render_known_hosts_refresh_fab(entity.clone())),
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

    fn render_chat_session_delete_prompt(
        &self,
        entity: Entity<AppView>,
        prompt: &PendingChatSessionDeleteState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let title = if prompt.title.trim().is_empty() {
            i18n::string("workspace.panel.agent.history.untitled_chat")
        } else {
            prompt.title.clone()
        };
        let subtitle = i18n::string_args("dialogs.chat_delete.message", &[("title", &title)]);

        let entity_cancel = entity.clone();
        let entity_confirm = entity.clone();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "chat-session-delete-cancel",
                    i18n::string("dialogs.chat_delete.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    entity_cancel.update(cx, |this, cx| {
                        this.cancel_session_agent_chat_delete(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "chat-session-delete-confirm",
                    i18n::string("dialogs.chat_delete.confirm"),
                    BasicDialogActionTone::Destructive,
                )
                .on_click(move |_, _, cx| {
                    entity_confirm.update(cx, |this, cx| {
                        this.confirm_session_agent_chat_delete(cx);
                    });
                }),
            );

        render_basic_dialog(
            "chat-session-delete",
            i18n::string("dialogs.chat_delete.title"),
            Some(subtitle),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }

    fn render_chat_session_rename_prompt(
        &self,
        entity: Entity<AppView>,
        prompt: &PendingChatSessionRenameState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let title_input = self.workspace_forms.agent.rename_title_input.clone();
        let current_title = if prompt.current_title.trim().is_empty() {
            i18n::string("workspace.panel.agent.history.untitled_chat")
        } else {
            prompt.current_title.clone()
        };
        let subtitle =
            i18n::string_args("dialogs.chat_rename.message", &[("title", &current_title)]);

        let entity_cancel = entity.clone();
        let entity_confirm = entity.clone();

        let body = v_flex()
            .w_full()
            .child(surface_text_input_stack(
                i18n::string("dialogs.chat_rename.title_label"),
                title_input.clone(),
                TextInputSurface::Highest,
                false,
            ))
            .into_any_element();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "chat-session-rename-cancel",
                    i18n::string("dialogs.common.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    entity_cancel.update(cx, |this, cx| {
                        this.cancel_session_agent_chat_rename(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "chat-session-rename-confirm",
                    i18n::string("dialogs.chat_rename.confirm"),
                    BasicDialogActionTone::Default,
                )
                .on_click({
                    let entity = entity_confirm.clone();
                    let title_input = title_input.clone();
                    move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            let new_title = title_input.read(cx).value().to_string();
                            this.confirm_session_agent_chat_rename(new_title, cx);
                        });
                    }
                }),
            );

        render_basic_dialog(
            "chat-session-rename",
            i18n::string("dialogs.chat_rename.title"),
            Some(subtitle),
            Some(body),
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
        let roles = miaominal_settings::current_theme().material.roles;
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
                    if let TabKind::Session(session) = &mut tab.kind
                        && session.pending_keyboard_interactive.is_some()
                    {
                        session.pending_keyboard_interactive = None;
                        break;
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
                    if let TabKind::Session(session) = &mut tab.kind
                        && session.pending_keyboard_interactive.is_some()
                    {
                        session.pending_keyboard_interactive = None;
                        if let Some(commands) = &session.commands {
                            let _ = commands.close();
                        }
                        break;
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
                                .id(SharedString::from(format!(
                                    "keyboard-interactive-prompt-{:?}",
                                    input.entity_id()
                                )))
                                .w_full()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(miaominal_settings::FontSize::Body.scaled())
                                        .text_color(rgb(roles.on_surface_variant))
                                        .child(prompt.prompt.clone()),
                                )
                                .child(
                                    HintedInput::new(input)
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
        bottom_popup_viewport_height: f32,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
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
            .child(surface_secret_text_input_stack(
                i18n::string("settings.sync.vault.passphrase.label"),
                input.clone(),
                crate::ui::components::SecretTextInputStackOptions {
                    surface: TextInputSurface::Low,
                    size: gpui_component::Size::Large,
                    required: true,
                    disabled: operation_in_progress,
                    trailing: None,
                    reveal_icon: self.secret_reveal_icon(SecretRevealTarget::LocalVaultPassphrase),
                },
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
            ))
            .when(requires_passphrase_confirmation, |this| {
                this.child(surface_secret_text_input_stack(
                    i18n::string("settings.sync.vault.confirm_passphrase.label"),
                    confirmation_input.clone(),
                    crate::ui::components::SecretTextInputStackOptions {
                        surface: TextInputSurface::Low,
                        size: gpui_component::Size::Large,
                        required: true,
                        disabled: operation_in_progress,
                        trailing: None,
                        reveal_icon: self.secret_reveal_icon(
                            SecretRevealTarget::LocalVaultPassphraseConfirmation,
                        ),
                    },
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
                ))
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
                                md3_select(&local_vault_auto_lock_duration_select)
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
            bottom_popup_panel(
                title,
                None,
                Some(popup_body),
                actions,
                bottom_popup_viewport_height,
            ),
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
        bottom_popup_viewport_height: f32,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
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
            .child(surface_secret_text_input_stack(
                i18n::string("settings.sync.encryption.passphrase.label"),
                input.clone(),
                crate::ui::components::SecretTextInputStackOptions {
                    surface: TextInputSurface::Low,
                    size: gpui_component::Size::Large,
                    required: true,
                    disabled: operation_in_progress,
                    trailing: None,
                    reveal_icon: self.secret_reveal_icon(SecretRevealTarget::SyncPassphrase),
                },
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
            ))
            .child(surface_secret_text_input_stack(
                i18n::string("settings.sync.encryption.passphrase.confirm_passphrase.label"),
                confirmation_input.clone(),
                crate::ui::components::SecretTextInputStackOptions {
                    surface: TextInputSurface::Low,
                    size: gpui_component::Size::Large,
                    required: true,
                    disabled: operation_in_progress,
                    trailing: None,
                    reveal_icon: self
                        .secret_reveal_icon(SecretRevealTarget::SyncPassphraseConfirmation),
                },
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
            ))
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
            bottom_popup_panel(
                title,
                None,
                Some(popup_body),
                actions,
                bottom_popup_viewport_height,
            ),
            "sync-passphrase",
            exit_progress,
            move |window, cx| {
                entity.update(cx, |this, cx| {
                    this.close_sync_passphrase_popup(window, cx);
                });
            },
        )
    }

    fn render_ai_provider_popup(
        &self,
        entity: Entity<AppView>,
        _popup: PendingAiProviderPopupState,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let kind_select = self.panel_forms.settings.ai_provider_kind_select.clone();
        let name_input = self.panel_forms.settings.ai_provider_name_input.clone();
        let model_input = self.panel_forms.settings.ai_provider_model_input.clone();
        let base_url_input = self.panel_forms.settings.ai_provider_base_url_input.clone();
        let api_key_input = self.panel_forms.settings.ai_provider_api_key_input.clone();
        let temperature_input = self
            .panel_forms
            .settings
            .ai_provider_temperature_input
            .clone();
        let max_tokens_input = self
            .panel_forms
            .settings
            .ai_provider_max_tokens_input
            .clone();
        let context_window_input = self
            .panel_forms
            .settings
            .ai_provider_context_window_input
            .clone();
        let provider_id = self
            .panel_forms
            .settings
            .editing_ai_provider_id
            .clone()
            .unwrap_or_default();
        let editing_provider_id = self.panel_forms.settings.editing_ai_provider_id.clone();
        let save_in_progress = self.ai_provider_save_in_progress();
        let api_key_load_in_progress = self.ai_provider_api_key_load_in_progress_for(&provider_id);
        let operation_in_progress = save_in_progress || api_key_load_in_progress;
        let target = SecretRevealTarget::AiProviderApiKey(provider_id);
        let reveal_icon = self.secret_reveal_icon(target.clone());
        let entity_cancel = entity.clone();
        let entity_submit = entity.clone();

        let popup_body = v_flex()
            .w_full()
            .gap_5()
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(field_label(
                        i18n::string("settings.ai_providers.kind.label"),
                        true,
                    ))
                    .child(
                        div().w_full().min_w(px(320.0)).child(
                            md3_select(&kind_select)
                                .large()
                                .w_full()
                                .bg(rgb(roles.surface_container_low)),
                        ),
                    ),
            )
            .child(surface_text_input_stack(
                i18n::string("settings.ai_providers.name.label"),
                name_input,
                TextInputSurface::Low,
                true,
            ))
            .child(surface_text_input_stack(
                i18n::string("settings.ai_providers.model.label"),
                model_input,
                TextInputSurface::Low,
                false,
            ))
            .child(surface_text_input_stack(
                i18n::string("settings.ai_providers.base_url.label"),
                base_url_input,
                TextInputSurface::Low,
                false,
            ))
            .child(surface_secret_text_input_stack(
                i18n::string("settings.ai_providers.api_key.label"),
                api_key_input,
                crate::ui::components::SecretTextInputStackOptions {
                    surface: TextInputSurface::Low,
                    size: gpui_component::Size::Large,
                    required: true,
                    disabled: operation_in_progress,
                    trailing: api_key_load_in_progress.then(|| {
                        div()
                            .id("ai-provider-api-key-load-spinner")
                            .size(px(30.0))
                            .rounded(px(10.0))
                            .bg(rgb(roles.surface_container_highest))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(crate::ui::components::md3_spinner(16.0))
                            .into_any_element()
                    }),
                    reveal_icon,
                },
                {
                    let entity = entity.clone();
                    move |window, cx| {
                        let target = target.clone();
                        entity.update(cx, |this, cx| {
                            this.toggle_secret_visibility(target, window, cx);
                        });
                    }
                },
            ))
            .child(surface_text_input_stack(
                i18n::string("settings.ai_providers.temperature.label"),
                temperature_input,
                TextInputSurface::Low,
                false,
            ))
            .child(surface_text_input_stack(
                i18n::string("settings.ai_providers.max_tokens.label"),
                max_tokens_input,
                TextInputSurface::Low,
                false,
            ))
            .child(surface_text_input_stack(
                i18n::string("settings.ai_providers.context_window.label"),
                context_window_input,
                TextInputSurface::Low,
                false,
            ))
            .into_any_element();

        let actions = h_flex()
            .w_full()
            .justify_between()
            .gap_3()
            .child(if let Some(provider_id) = editing_provider_id {
                let entity_delete = entity.clone();
                Button::new("ai-provider-popup-delete")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .disabled(operation_in_progress)
                    .text_color(rgb(roles.error))
                    .label(i18n::string("settings.ai_providers.actions.delete"))
                    .on_click(move |_, window, cx| {
                        let provider_id = provider_id.clone();
                        entity_delete.update(cx, |this, cx| {
                            this.delete_ai_provider(provider_id, window, cx);
                        });
                    })
                    .into_any_element()
            } else {
                div().into_any_element()
            })
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        Button::new("ai-provider-popup-cancel")
                            .ghost()
                            .border_0()
                            .rounded(px(20.0))
                            .large()
                            .disabled(save_in_progress)
                            .text_color(rgb(roles.on_surface_variant))
                            .label(i18n::string("dialogs.common.cancel"))
                            .on_click(move |_, window, cx| {
                                entity_cancel.update(cx, |this, cx| {
                                    this.close_ai_provider_popup(window, cx);
                                });
                            }),
                    )
                    .child(if save_in_progress {
                        div()
                            .id("ai-provider-popup-submit-spinner")
                            .min_w(px(116.0))
                            .min_h(px(32.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(crate::ui::components::md3_spinner(18.0))
                            .into_any_element()
                    } else {
                        Button::new("ai-provider-popup-submit")
                            .ghost()
                            .border_0()
                            .rounded(px(20.0))
                            .large()
                            .disabled(operation_in_progress)
                            .text_color(rgb(roles.primary))
                            .label(i18n::string("settings.ai_providers.actions.save"))
                            .on_click(move |_, window, cx| {
                                entity_submit.update(cx, |this, cx| {
                                    this.submit_ai_provider_save(window, cx);
                                });
                            })
                            .into_any_element()
                    }),
            )
            .into_any_element();

        render_bottom_popup(
            bottom_popup_panel(
                i18n::string("settings.ai_providers.editor_group.title"),
                Some(i18n::string(
                    "settings.ai_providers.editor_group.description",
                )),
                Some(popup_body),
                actions,
                bottom_popup_viewport_height,
            ),
            "ai-provider",
            exit_progress,
            move |window, cx| {
                entity.update(cx, |this, cx| {
                    this.close_ai_provider_popup(window, cx);
                });
            },
        )
    }

    fn render_sync_provider_config_popup(
        &self,
        entity: Entity<AppView>,
        popup: PendingSyncProviderConfigPopupState,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let save_in_progress = self.sync_provider_config_save_in_progress_for(popup.provider);
        let entity_cancel = entity.clone();
        let entity_submit = entity.clone();

        let (title, description, popup_body, popup_key) = match popup.provider {
            SyncProvider::GithubGist => {
                let token_input = self.panel_forms.settings.sync_github_token_input.clone();
                let gist_id_input = self.panel_forms.settings.sync_github_gist_id_input.clone();
                let target = SecretRevealTarget::SyncGithubToken;
                let reveal_icon = self.secret_reveal_icon(target.clone());
                let entity_toggle = entity.clone();
                (
                    i18n::string("settings.sync.gist_group.title"),
                    i18n::string("settings.sync.gist_group.description"),
                    v_flex()
                        .w_full()
                        .gap_5()
                        .child(surface_secret_text_input_stack(
                            i18n::string("settings.sync.gist.token.label"),
                            token_input,
                            crate::ui::components::SecretTextInputStackOptions {
                                surface: TextInputSurface::Low,
                                size: gpui_component::Size::Large,
                                required: true,
                                disabled: save_in_progress,
                                trailing: None,
                                reveal_icon,
                            },
                            move |window, cx| {
                                entity_toggle.update(cx, |this, cx| {
                                    this.toggle_secret_visibility(
                                        SecretRevealTarget::SyncGithubToken,
                                        window,
                                        cx,
                                    );
                                });
                            },
                        ))
                        .child(surface_text_input_stack(
                            i18n::string("settings.sync.gist.gist_id.label"),
                            gist_id_input,
                            TextInputSurface::Low,
                            false,
                        ))
                        .into_any_element(),
                    "sync-provider-gist",
                )
            }
            SyncProvider::WebDav => {
                let url_input = self.panel_forms.settings.sync_webdav_url_input.clone();
                let username_input = self.panel_forms.settings.sync_webdav_username_input.clone();
                let password_input = self.panel_forms.settings.sync_webdav_password_input.clone();
                let target = SecretRevealTarget::SyncWebdavPassword;
                let reveal_icon = self.secret_reveal_icon(target.clone());
                let entity_toggle = entity.clone();
                (
                    i18n::string("settings.sync.webdav_group.title"),
                    i18n::string("settings.sync.webdav_group.description"),
                    v_flex()
                        .w_full()
                        .gap_5()
                        .child(surface_text_input_stack(
                            i18n::string("settings.sync.webdav.url.label"),
                            url_input,
                            TextInputSurface::Low,
                            true,
                        ))
                        .child(surface_text_input_stack(
                            i18n::string("settings.sync.webdav.username.label"),
                            username_input,
                            TextInputSurface::Low,
                            true,
                        ))
                        .child(surface_secret_text_input_stack(
                            i18n::string("settings.sync.webdav.password.label"),
                            password_input,
                            crate::ui::components::SecretTextInputStackOptions {
                                surface: TextInputSurface::Low,
                                size: gpui_component::Size::Large,
                                required: true,
                                disabled: save_in_progress,
                                trailing: None,
                                reveal_icon,
                            },
                            move |window, cx| {
                                entity_toggle.update(cx, |this, cx| {
                                    this.toggle_secret_visibility(
                                        SecretRevealTarget::SyncWebdavPassword,
                                        window,
                                        cx,
                                    );
                                });
                            },
                        ))
                        .into_any_element(),
                    "sync-provider-webdav",
                )
            }
            SyncProvider::None => (
                i18n::string("settings.sync.provider_group.title"),
                i18n::string("settings.sync.provider_group.description"),
                div().into_any_element(),
                "sync-provider-none",
            ),
        };

        let actions = h_flex()
            .w_full()
            .justify_end()
            .gap_3()
            .child(
                Button::new("sync-provider-config-popup-cancel")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .disabled(save_in_progress)
                    .text_color(rgb(roles.on_surface_variant))
                    .label(i18n::string("dialogs.common.cancel"))
                    .on_click(move |_, window, cx| {
                        entity_cancel.update(cx, |this, cx| {
                            this.close_sync_provider_config_popup(window, cx);
                        });
                    }),
            )
            .child(if save_in_progress {
                div()
                    .id("sync-provider-config-popup-submit-spinner")
                    .min_w(px(116.0))
                    .min_h(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(crate::ui::components::md3_spinner(18.0))
                    .into_any_element()
            } else {
                Button::new("sync-provider-config-popup-submit")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .text_color(rgb(roles.primary))
                    .label(i18n::string("settings.sync.save_action"))
                    .on_click(move |_, window, cx| {
                        entity_submit.update(cx, |this, cx| {
                            this.submit_sync_provider_config_popup_action(window, cx);
                        });
                    })
                    .into_any_element()
            })
            .into_any_element();

        render_bottom_popup(
            bottom_popup_panel(
                title,
                Some(description),
                Some(popup_body),
                actions,
                bottom_popup_viewport_height,
            ),
            popup_key,
            exit_progress,
            move |window, cx| {
                entity.update(cx, |this, cx| {
                    this.close_sync_provider_config_popup(window, cx);
                });
            },
        )
    }

    fn render_sync_passphrase_clear_confirm_popup(
        &self,
        entity: Entity<AppView>,
        _popup: PendingSyncPassphraseClearConfirmPopupState,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
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
                    .text_size(miaominal_settings::FontSize::Heading.scaled())
                    .line_height(miaominal_settings::scaled_line_height(20.0))
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
                bottom_popup_viewport_height,
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
        bottom_popup_viewport_height: f32,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
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
                    .text_size(miaominal_settings::FontSize::Heading.scaled())
                    .line_height(miaominal_settings::scaled_line_height(20.0))
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
                bottom_popup_viewport_height,
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
        bottom_popup_viewport_height: f32,
    ) -> gpui::AnyElement {
        match snapshot {
            DialogOverlaySnapshot::HostKey(prompt) => self.render_trusted_host_key_prompt(
                entity,
                &prompt,
                Some(exit_progress),
                bottom_popup_viewport_height,
            ),
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
            DialogOverlaySnapshot::ChatSessionDelete(prompt) => {
                self.render_chat_session_delete_prompt(entity, &prompt, Some(exit_progress))
            }
            DialogOverlaySnapshot::ChatSessionRename(prompt) => {
                self.render_chat_session_rename_prompt(entity, &prompt, Some(exit_progress))
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
            DialogOverlaySnapshot::LocalDataResetConfirmationPopup(popup) => self
                .render_local_data_reset_confirmation_popup(
                    entity,
                    popup,
                    Some(exit_progress),
                    bottom_popup_viewport_height,
                ),
            DialogOverlaySnapshot::SyncPassphraseClearConfirmPopup(popup) => self
                .render_sync_passphrase_clear_confirm_popup(
                    entity,
                    popup,
                    Some(exit_progress),
                    bottom_popup_viewport_height,
                ),
            DialogOverlaySnapshot::SyncPassphrasePopup(popup) => self.render_sync_passphrase_popup(
                entity,
                popup,
                Some(exit_progress),
                bottom_popup_viewport_height,
            ),
            DialogOverlaySnapshot::AiProviderPopup(popup) => self.render_ai_provider_popup(
                entity,
                popup,
                Some(exit_progress),
                bottom_popup_viewport_height,
            ),
            DialogOverlaySnapshot::SyncProviderConfigPopup(popup) => self
                .render_sync_provider_config_popup(
                    entity,
                    popup,
                    Some(exit_progress),
                    bottom_popup_viewport_height,
                ),
            DialogOverlaySnapshot::LocalVaultPassphrasePopup(mode) => self
                .render_local_vault_passphrase_popup(
                    entity,
                    mode,
                    Some(exit_progress),
                    bottom_popup_viewport_height,
                ),
            DialogOverlaySnapshot::SftpPrompt { tab_id, prompt } => {
                self.render_sftp_prompt_overlay(entity, tab_id, &prompt, Some(exit_progress))
            }
        }
    }
}
