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
        let window_title = self.window_title(cx);
        if window.window_title() != window_title {
            window.set_window_title(&window_title);
        }

        let entity = cx.entity();
        let roles = miaominal_settings::current_theme().material.roles;
        let bottom_popup_viewport_height = f32::from(window.bounds().size.height);
        if self.controllers.settings.read(cx).show_onboarding() {
            if self.active_terminal_session_index(cx).is_none()
                && self
                    .controllers
                    .agent
                    .read(cx)
                    .session_agent()
                    .conversation_view
                    .is_some()
            {
                self.controllers.agent.update(cx, |controller, cx| {
                    controller.finish_text_drag(cx);
                    controller.release_conversation_view(cx);
                });
            }
            let notification_layer =
                Root::render_notification_layer(window, cx).map(IntoElement::into_any_element);
            return pages::render_onboarding_page(
                self.controllers.settings.clone(),
                notification_layer,
                window,
                cx,
            );
        }
        let has_active_session = self.has_active_session();
        let has_active_sftp_tab = self
            .workspace
            .active_topbar_tab
            .and_then(|tab_id| self.workspace.tabs.get(tab_id))
            .is_some_and(|tab| tab.is_sftp());
        let desired_primary_view =
            self.desired_primary_view_kind(has_active_session, has_active_sftp_tab, cx);
        let primary_view_transition =
            self.primary_view_transition_render_state(desired_primary_view, window, cx);
        if self.active_terminal_session_index(cx).is_none()
            && !primary_view_transition.animating
            && self
                .controllers
                .agent
                .read(cx)
                .session_agent()
                .conversation_view
                .is_some()
        {
            // The workspace surface may disappear immediately (for example when the last
            // terminal closes), so its side-panel exit animation is not guaranteed to get a
            // final frame in which to release the expensive conversation projection. Keep the
            // projection only while an outgoing terminal workspace is actually animating.
            self.controllers.agent.update(cx, |controller, cx| {
                controller.finish_text_drag(cx);
                controller.release_conversation_view(cx);
            });
        }
        let primary_view_animating = primary_view_transition.animating;
        let root_right_gutter = self.primary_view_root_right_gutter(primary_view_transition);

        let ordered_tab_ids = self.workspace.tabs.ids().collect::<Vec<_>>();
        let active_tab_id = self.workspace.workspace.active_tab;
        let (pending_host_key, pending_kbi) = {
            let controller = self.controllers.session.read(cx);
            (
                controller.pending_host_key_prompt(active_tab_id, &ordered_tab_ids),
                controller.pending_keyboard_interactive_prompt(active_tab_id, &ordered_tab_ids),
            )
        };
        let pending_profile_delete = self.pending_profile_delete_prompt(cx);
        let pending_managed_key_delete = self.pending_managed_key_delete_prompt(cx);
        let pending_managed_key_rename = self.pending_managed_key_rename_prompt(cx);
        let pending_known_host_delete = self.pending_known_host_delete_prompt(cx);
        let pending_snippet_delete = self.pending_snippet_delete_prompt(cx);
        let pending_port_forward_rule_delete = self.pending_port_forward_rule_delete_prompt(cx);
        let pending_chat_session_delete = self.pending_chat_session_delete_prompt(cx);
        let pending_chat_session_rename = self.pending_chat_session_rename_prompt(cx);
        let pending_sync_direction = self.pending_sync_direction_prompt(cx);
        let pending_sync_pull_confirm = self.pending_sync_pull_confirm_prompt(cx);
        let pending_local_vault_disable_confirm =
            self.pending_local_vault_disable_confirm_prompt(cx);
        let pending_local_data_reset_confirm = self.pending_local_data_reset_confirm_prompt(cx);
        let pending_local_data_reset_confirmation_popup =
            self.pending_local_data_reset_confirmation_popup(cx);
        let pending_sync_passphrase_clear_confirm_popup =
            self.pending_sync_passphrase_clear_confirm_popup(cx);
        let pending_sync_passphrase_popup = self.pending_sync_passphrase_popup(cx);
        let pending_ai_provider_popup = self.pending_ai_provider_popup(cx);
        let pending_web_search_config_popup = self.pending_web_search_config_popup(cx);
        let pending_sync_provider_config_popup = self.pending_sync_provider_config_popup(cx);
        let pending_local_vault_passphrase_popup = self.pending_local_vault_passphrase_popup(cx);
        let pending_sftp_prompt = self.pending_sftp_prompt(cx);
        let exiting_dialogs = self.active_exiting_dialogs(window);
        let has_exiting_kbi = exiting_dialogs.iter().any(|(snapshot, _)| {
            matches!(snapshot, DialogOverlaySnapshot::KeyboardInteractive { .. })
        });

        let pending_kbi_challenge = pending_kbi.as_ref().map(|(_, challenge)| challenge.clone());
        self.controllers.session.update(cx, |controller, cx| {
            controller.sync_keyboard_interactive_inputs(
                pending_kbi_challenge,
                has_exiting_kbi,
                window,
                cx,
            );
        });

        let editor_state = self.controllers.session.read(cx).editor_state();
        let show_host_editor_sidebar = editor_state.host_editor_open
            && !has_active_session
            && self.shell.shell_state.sidebar_section == SidebarSection::Hosts;
        let show_port_forward_editor_sidebar = editor_state.port_forward_editor_open
            && !has_active_session
            && !has_active_sftp_tab
            && self.shell.shell_state.sidebar_section == SidebarSection::PortForwarding;
        let show_snippets_editor_sidebar = editor_state.snippets_editor_open
            && !has_active_session
            && !has_active_sftp_tab
            && self.shell.shell_state.sidebar_section == SidebarSection::Snippets;
        let show_keychain_editor_sidebar = self.controllers.keychain.read(cx).editor_open()
            && !has_active_session
            && !has_active_sftp_tab
            && self.shell.shell_state.sidebar_section == SidebarSection::Keychain;
        let show_known_hosts_sidebar = self
            .controllers
            .session
            .read(cx)
            .selected_known_host()
            .is_some()
            && !has_active_session
            && !has_active_sftp_tab
            && self.shell.shell_state.sidebar_section == SidebarSection::KnownHosts;
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

        let finish_agent_drag = self.controllers.agent.clone();
        let finish_agent_drag_out = finish_agent_drag.clone();
        let recover_sftp_drag = self.controllers.sftp.clone();
        let finish_sftp_drag_out = recover_sftp_drag.clone();

        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(rgb(roles.surface_container))
            .on_mouse_move(move |event: &MouseMoveEvent, _window, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    recover_sftp_drag.update(cx, |controller, cx| {
                        controller.finish_any_active_drag_selection(cx);
                    });
                }
            })
            .capture_any_mouse_up(move |event: &MouseUpEvent, _window, cx| {
                if event.button == MouseButton::Left {
                    finish_agent_drag.update(cx, |controller, cx| {
                        controller.finish_text_drag(cx);
                    });
                }
            })
            .on_mouse_up_out(
                MouseButton::Left,
                move |_event: &MouseUpEvent, _window, cx| {
                    finish_agent_drag_out.update(cx, |controller, cx| {
                        controller.finish_text_drag(cx);
                    });
                    finish_sftp_drag_out.update(cx, |controller, cx| {
                        controller.finish_any_active_drag_selection(cx);
                    });
                },
            )
            .pr(px(root_right_gutter))
            .child(self.render_top_bar(entity.clone(), window, cx))
            .child(shell_body)
            .child(self.render_status_footer(entity.clone(), cx))
            .when_some(pending_host_key, |this, (tab_id, prompt)| {
                this.child(self.render_session_host_key_prompt(
                    tab_id,
                    &prompt,
                    None,
                    bottom_popup_viewport_height,
                    cx,
                ))
            })
            .when_some(pending_kbi, |this, (tab_id, challenge)| {
                this.child(self.render_keyboard_interactive_prompt(tab_id, &challenge, None, cx))
            })
            .when_some(pending_profile_delete, |this, prompt| {
                this.child(self.render_profile_delete_prompt(entity.clone(), &prompt, None))
            })
            .when_some(pending_managed_key_delete, |this, prompt| {
                this.child(self.render_managed_key_delete_prompt(entity.clone(), &prompt, None))
            })
            .when_some(pending_managed_key_rename, |this, prompt| {
                this.child(self.render_managed_key_rename_prompt(&prompt, None, cx))
            })
            .when_some(pending_known_host_delete, |this, prompt| {
                let controller = self.controllers.session.clone();
                this.child(controller.read(cx).render_trusted_known_host_delete_prompt(
                    controller.clone(),
                    &prompt,
                    None,
                ))
            })
            .when_some(pending_snippet_delete, |this, prompt| {
                this.child(self.render_snippet_delete_prompt(entity.clone(), &prompt, None))
            })
            .when_some(pending_port_forward_rule_delete, |this, prompt| {
                this.child(self.render_port_forward_rule_delete_prompt(&prompt, None))
            })
            .when_some(pending_chat_session_delete, |this, prompt| {
                this.child(self.render_chat_session_delete_prompt(&prompt, None))
            })
            .when_some(pending_chat_session_rename, |this, prompt| {
                this.child(self.render_chat_session_rename_prompt(&prompt, None, cx))
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
                        cx,
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
                        cx,
                    ))
                },
            )
            .when_some(pending_sync_passphrase_popup, |this, prompt| {
                this.child(self.render_sync_passphrase_popup(
                    entity.clone(),
                    prompt,
                    None,
                    bottom_popup_viewport_height,
                    cx,
                ))
            })
            .when_some(pending_ai_provider_popup, |this, popup| {
                this.child(self.render_ai_provider_popup(
                    entity.clone(),
                    popup,
                    None,
                    bottom_popup_viewport_height,
                    cx,
                ))
            })
            .when_some(pending_web_search_config_popup, |this, popup| {
                this.child(self.render_web_search_config_popup(
                    entity.clone(),
                    popup,
                    None,
                    bottom_popup_viewport_height,
                    cx,
                ))
            })
            .when_some(pending_sync_provider_config_popup, |this, popup| {
                this.child(self.render_sync_provider_config_popup(
                    entity.clone(),
                    popup,
                    None,
                    bottom_popup_viewport_height,
                    cx,
                ))
            })
            .when_some(pending_local_vault_passphrase_popup, |this, mode| {
                this.child(self.render_local_vault_passphrase_popup(
                    entity.clone(),
                    mode,
                    None,
                    bottom_popup_viewport_height,
                    cx,
                ))
            })
            .when_some(pending_sftp_prompt, |this, prompt| {
                let (tab_id, prompt) = prompt;
                let controller = self.controllers.sftp.clone();
                this.child(controller.read(cx).render_sftp_prompt_overlay(
                    controller.clone(),
                    tab_id,
                    &prompt,
                    None,
                ))
            })
            .children(exiting_dialogs.into_iter().map(|(snapshot, progress)| {
                self.render_exiting_dialog_overlay(
                    entity.clone(),
                    snapshot,
                    progress,
                    bottom_popup_viewport_height,
                    cx,
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
    fn active_terminal_tab_id(&self, cx: &App) -> Option<TabId> {
        let tab_id = self.workspace.active_topbar_tab?;
        self.session_tab(tab_id, cx)
            .is_some_and(|session| session.purpose == SessionPurpose::Terminal)
            .then_some(tab_id)
    }

    fn desired_primary_view_kind(
        &self,
        has_active_session: bool,
        has_active_sftp_tab: bool,
        cx: &App,
    ) -> PrimaryViewKind {
        if has_active_session && let Some(tab_id) = self.active_terminal_tab_id(cx) {
            return PrimaryViewKind::Terminal(tab_id);
        }

        if has_active_sftp_tab && let Some(tab_id) = self.active_sftp_tab_id() {
            return PrimaryViewKind::Sftp(tab_id);
        }

        PrimaryViewKind::Sidebar(self.shell.shell_state.sidebar_section)
    }

    fn primary_view_exists(&self, view: PrimaryViewKind, cx: &App) -> bool {
        match view {
            PrimaryViewKind::Sidebar(_) => true,
            PrimaryViewKind::Terminal(tab_id) => self
                .session_tab(tab_id, cx)
                .is_some_and(|session| session.purpose == SessionPurpose::Terminal),
            PrimaryViewKind::Sftp(tab_id) => self
                .workspace
                .tabs
                .get(tab_id)
                .is_some_and(|tab| tab.is_sftp()),
        }
    }

    fn primary_view_tab_index(&self, view: PrimaryViewKind) -> Option<usize> {
        match view {
            PrimaryViewKind::Terminal(tab_id) | PrimaryViewKind::Sftp(tab_id) => {
                self.workspace.tabs.index_of(tab_id)
            }
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

    fn finish_primary_view_transition(&mut self, transition: PrimaryViewTransition, cx: &App) {
        if matches!(
            (transition.from, transition.to),
            (
                PrimaryViewKind::Sidebar(SidebarSection::Hosts),
                PrimaryViewKind::Terminal(_)
            )
        ) {
            self.controllers
                .session
                .read(cx)
                .set_host_editor_state(false, false);
        }

        self.workspace.primary_view_transition = None;
    }

    fn primary_view_transition_render_state(
        &mut self,
        desired: PrimaryViewKind,
        window: &mut Window,
        cx: &App,
    ) -> PrimaryViewTransitionRenderState {
        let now = Instant::now();
        let duration = super::support::CONTAINER_TRANSITION_DURATION;

        if self.workspace.visible_primary_view.is_none() {
            self.workspace.visible_primary_view = Some(desired);
        }

        if self.workspace.visible_primary_view != Some(desired) {
            let from = self
                .workspace
                .primary_view_transition
                .map(|transition| transition.to)
                .or(self.workspace.visible_primary_view)
                .unwrap_or(desired);
            self.workspace.visible_primary_view = Some(desired);
            if self.should_animate_primary_view_transition(from, desired) {
                self.workspace.primary_view_transition = Some(PrimaryViewTransition {
                    from,
                    to: desired,
                    started_at: now,
                    duration,
                });
            } else {
                self.workspace.primary_view_transition = None;
            }
        }

        let Some(transition) = self.workspace.primary_view_transition else {
            return PrimaryViewTransitionRenderState {
                from: desired,
                to: desired,
                visibility: 1.0,
                animating: false,
            };
        };

        if !self.primary_view_exists(transition.from, cx)
            || !self.primary_view_exists(transition.to, cx)
        {
            self.workspace.primary_view_transition = None;
            self.workspace.visible_primary_view = Some(desired);
            return PrimaryViewTransitionRenderState {
                from: desired,
                to: desired,
                visibility: 1.0,
                animating: false,
            };
        }

        let duration_seconds = transition.duration.as_secs_f32();
        if duration_seconds <= f32::EPSILON {
            self.finish_primary_view_transition(transition, cx);
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
            self.finish_primary_view_transition(transition, cx);
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
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match section {
            SidebarSection::Hosts => {
                let controller = self.controllers.session.clone();
                let connect_controller = controller.clone();
                let edit_controller = controller.clone();
                let sftp_controller = controller.clone();
                controller.read(cx).render_hosts_page(
                    controller.clone(),
                    move |index, _window, cx| {
                        connect_controller.update(cx, |controller, cx| {
                            controller.request_session_at_profile_index(index, cx);
                        });
                    },
                    move |index, _window, cx| {
                        edit_controller.update(cx, |controller, cx| {
                            controller.request_profile_editor_at_index(index, cx);
                        });
                    },
                    move |index, _window, cx| {
                        sftp_controller.update(cx, |controller, cx| {
                            controller.request_sftp_at_profile_index(index, cx);
                        });
                    },
                    cx,
                )
            }
            SidebarSection::Keychain => {
                let controller = self.controllers.keychain.clone();
                controller
                    .read(cx)
                    .render_keychain_page(controller.clone(), cx)
            }
            SidebarSection::PortForwarding => {
                let controller = self.controllers.session.clone();
                controller
                    .read(cx)
                    .render_forward_page(controller.clone(), cx)
            }
            SidebarSection::KnownHosts => {
                let controller = self.controllers.session.clone();
                controller
                    .read(cx)
                    .render_trusted_page(controller.clone(), cx)
            }
            SidebarSection::Settings => {
                pages::render_settings_page(self.controllers.settings.clone())
            }
            SidebarSection::Snippets => {
                let controller = self.controllers.session.clone();
                let edit_controller = controller.clone();
                controller.read(cx).render_snippets_page(
                    controller.clone(),
                    move |index, window, cx| {
                        edit_controller.update(cx, |controller, cx| {
                            controller.open_existing_snippet_editor(index, window, cx);
                        });
                    },
                    cx,
                )
            }
        }
    }

    fn render_primary_view_panel(
        &mut self,
        view: PrimaryViewKind,
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
                (self.render_sidebar_primary_panel(section, cx), render_state)
            }
            PrimaryViewKind::Terminal(tab_id) => (
                self.render_terminal_page_for_tab(tab_id, window, cx)
                    .unwrap_or_else(|| {
                        layout::workspace::render_workspace_surface(self, window, cx)
                    }),
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
            PrimaryViewKind::Sftp(tab_id) => {
                let controller = self.controllers.sftp.clone();
                let render_controller = controller.clone();
                let ordered_tab_ids = self.workspace.tabs.ids().collect::<Vec<_>>();
                let fallback_section = self.shell.shell_state.sidebar_section;
                let page = controller.update(cx, |controller, cx| {
                    controller.render_sftp_page_for_tab(
                        render_controller,
                        tab_id,
                        &ordered_tab_ids,
                        Some(tab_id),
                        fallback_section,
                        window,
                        cx,
                    )
                });
                (
                    page,
                    PrimarySurfaceRenderState {
                        has_active_session: false,
                        has_active_sftp_tab: true,
                        show_host_editor_sidebar: false,
                        show_port_forward_editor_sidebar: false,
                        show_keychain_editor_sidebar: false,
                        show_snippets_editor_sidebar: false,
                        show_known_hosts_sidebar: false,
                    },
                )
            }
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
            self.render_primary_view_panel(view, Some(current_surface), window, cx);
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
        self.shell.shell_state.page_editor_sidebar_transition = None;
        self.shell.shell_state.visible_page_editor_sidebar = None;
    }

    fn render_terminal_page_for_tab(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let index = self
            .workspace
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)?;

        if self.workspace.active_topbar_tab == self.workspace.tabs.id_at(index)
            && self.session_tab(tab_id, cx).is_some()
        {
            return Some(layout::workspace::render_workspace_surface(
                self, window, cx,
            ));
        }

        let parked_workspace = self.workspace.take_parked_workspace(tab_id)?;
        let live_workspace = std::mem::replace(&mut self.workspace.workspace, parked_workspace);
        let rendered = layout::workspace::render_workspace_surface(self, window, cx);
        let parked_workspace = std::mem::replace(&mut self.workspace.workspace, live_workspace);
        self.workspace.park_workspace(tab_id, parked_workspace);

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
            Some(kind) => match self.shell.shell_state.page_editor_sidebar_transition {
                Some(transition) if transition.kind == kind => {
                    if transition.phase == PageEditorSidebarTransitionPhase::Exiting {
                        self.shell.shell_state.page_editor_sidebar_transition =
                            Some(PageEditorSidebarTransition {
                                phase: PageEditorSidebarTransitionPhase::Entering,
                                started_at: now,
                                ..transition
                            });
                    }
                }
                _ => {
                    if self.shell.shell_state.visible_page_editor_sidebar != Some(kind)
                        || self
                            .shell
                            .shell_state
                            .page_editor_sidebar_transition
                            .is_some()
                    {
                        self.shell.shell_state.visible_page_editor_sidebar = Some(kind);
                        self.shell.shell_state.page_editor_sidebar_transition =
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
                if let Some(kind) = self.shell.shell_state.visible_page_editor_sidebar {
                    match self.shell.shell_state.page_editor_sidebar_transition {
                        Some(transition) if transition.kind == kind => {
                            if transition.phase == PageEditorSidebarTransitionPhase::Entering {
                                self.shell.shell_state.page_editor_sidebar_transition =
                                    Some(PageEditorSidebarTransition {
                                        phase: PageEditorSidebarTransitionPhase::Exiting,
                                        started_at: now,
                                        ..transition
                                    });
                            }
                        }
                        _ => {
                            self.shell.shell_state.page_editor_sidebar_transition =
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

        if let Some(transition) = self.shell.shell_state.page_editor_sidebar_transition {
            let duration_seconds = transition.duration.as_secs_f32();
            if duration_seconds <= f32::EPSILON {
                self.shell.shell_state.page_editor_sidebar_transition = None;
                self.shell.shell_state.visible_page_editor_sidebar = match transition.phase {
                    PageEditorSidebarTransitionPhase::Entering => Some(transition.kind),
                    PageEditorSidebarTransitionPhase::Exiting => None,
                };
                return self
                    .shell
                    .shell_state
                    .visible_page_editor_sidebar
                    .map(|kind| PageEditorSidebarRenderState {
                        kind,
                        visibility: 1.0,
                    });
            }

            let elapsed = now.saturating_duration_since(transition.started_at);
            let progress = (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0);
            let eased = progress * progress * (3.0 - 2.0 * progress);

            if progress >= 1.0 {
                self.shell.shell_state.page_editor_sidebar_transition = None;
                self.shell.shell_state.visible_page_editor_sidebar = match transition.phase {
                    PageEditorSidebarTransitionPhase::Entering => Some(transition.kind),
                    PageEditorSidebarTransitionPhase::Exiting => None,
                };

                return self
                    .shell
                    .shell_state
                    .visible_page_editor_sidebar
                    .map(|kind| PageEditorSidebarRenderState {
                        kind,
                        visibility: 1.0,
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

        self.shell.shell_state.visible_page_editor_sidebar = desired;
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
                this.child(layout::sidebar::render_sidebar(
                    self,
                    self.controllers.settings.clone(),
                ))
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
                        cx,
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
            self.render_primary_view_panel(transition.to, None, window, cx);
        let (outgoing_panel, outgoing_state) =
            self.render_primary_view_panel(transition.from, None, window, cx);
        let incoming_surface =
            self.render_primary_surface_layer(entity.clone(), incoming_panel, incoming_state, cx);
        let outgoing_surface =
            self.render_primary_surface_layer(entity.clone(), outgoing_panel, outgoing_state, cx);
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
                                .child(layout::sidebar::render_sidebar(
                                    self,
                                    self.controllers.settings.clone(),
                                )),
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
        _entity: Entity<Self>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match kind {
            PageEditorSidebarKind::Hosts => {
                let controller = self.controllers.session.clone();
                let test_controller = controller.clone();
                let save_controller = controller.clone();
                let visibility_controller = controller.clone();
                controller
                    .read(cx)
                    .render_hosts_editor_sidebar(
                        controller.clone(),
                        move |window, cx| {
                            test_controller.update(cx, |controller, cx| {
                                controller.request_profile_connection_test(window, cx);
                            });
                        },
                        move |window, cx| {
                            save_controller.update(cx, |controller, cx| {
                                controller.request_profile_save(window, cx);
                            });
                        },
                        move |window, cx| {
                            visibility_controller.update(cx, |controller, cx| {
                                controller.toggle_host_password_visibility(window, cx);
                            });
                        },
                        cx,
                    )
                    .into_any_element()
            }
            PageEditorSidebarKind::PortForwarding => {
                let controller = self.controllers.session.clone();
                let settings_controller = self.controllers.settings.clone();
                let save_controller = controller.clone();
                controller
                    .read(cx)
                    .render_port_forward_editor_sidebar(
                        controller.clone(),
                        move |window, cx| {
                            let unlock_required = settings_controller
                                .read(cx)
                                .sync_requires_local_vault_unlock();
                            save_controller.update(cx, |controller, cx| {
                                controller.save_port_forward_rule(unlock_required, window, cx);
                            });
                        },
                        cx,
                    )
                    .into_any_element()
            }
            PageEditorSidebarKind::Snippets => {
                let controller = self.controllers.session.clone();
                let save_controller = controller.clone();
                controller
                    .read(cx)
                    .render_snippets_editor_sidebar(
                        controller.clone(),
                        move |window, cx| {
                            save_controller.update(cx, |controller, cx| {
                                controller.request_save_snippet(window, cx);
                            });
                        },
                        cx,
                    )
                    .into_any_element()
            }
            PageEditorSidebarKind::Keychain => self
                .controllers
                .keychain
                .read(cx)
                .render_keychain_editor_sidebar(self.controllers.keychain.clone())
                .into_any_element(),
            PageEditorSidebarKind::KnownHosts => {
                let controller = self.controllers.session.clone();
                controller
                    .read(cx)
                    .render_trusted_known_host_sidebar(controller.clone())
                    .into_any_element()
            }
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
        cx: &App,
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
                self.shell.shell_state.sidebar_section == SidebarSection::Hosts
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
                self.shell.shell_state.sidebar_section == SidebarSection::PortForwarding
                    && !has_active_session
                    && !has_active_sftp_tab
                    && !show_port_forward_editor_sidebar,
                |this| {
                    this.child(
                        div().absolute().right(px(28.0)).bottom(px(28.0)).child(
                            self.controllers
                                .session
                                .read(cx)
                                .render_forward_fab(self.controllers.session.clone()),
                        ),
                    )
                },
            )
            .when(
                self.shell.shell_state.sidebar_section == SidebarSection::Keychain
                    && !has_active_session
                    && !has_active_sftp_tab
                    && !show_keychain_editor_sidebar,
                |this| {
                    this.child(
                        div().absolute().right(px(28.0)).bottom(px(28.0)).child(
                            self.controllers
                                .keychain
                                .read(cx)
                                .render_keychain_fab(self.controllers.keychain.clone()),
                        ),
                    )
                },
            )
            .when(
                self.shell.shell_state.sidebar_section == SidebarSection::Snippets
                    && !has_active_session
                    && !has_active_sftp_tab
                    && !show_snippets_editor_sidebar,
                |this| {
                    this.child(div().absolute().right(px(28.0)).bottom(px(28.0)).child({
                        let controller = self.controllers.session.clone();
                        let open_controller = controller.clone();
                        controller.read(cx).render_snippets_fab(move |window, cx| {
                            open_controller.update(cx, |controller, cx| {
                                controller.open_snippets_editor(window, cx);
                            });
                        })
                    }))
                },
            )
            .when(
                self.shell.shell_state.sidebar_section == SidebarSection::KnownHosts
                    && !has_active_session
                    && !has_active_sftp_tab
                    && !show_known_hosts_sidebar,
                |this| {
                    this.child(div().absolute().right(px(28.0)).bottom(px(28.0)).child({
                        let controller = self.controllers.session.clone();
                        controller
                            .read(cx)
                            .render_known_hosts_refresh_fab(controller.clone())
                    }))
                },
            )
            .into_any_element()
    }

    fn render_managed_key_delete_prompt(
        &self,
        _entity: Entity<Self>,
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

        let controller_cancel = self.controllers.keychain.clone();
        let controller_confirm = self.controllers.keychain.clone();

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
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_managed_key_delete(cx);
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
                    controller_confirm.update(cx, |controller, cx| {
                        controller.confirm_managed_key_delete(window, cx);
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

    fn render_managed_key_rename_prompt(
        &self,
        prompt: &PendingManagedKeyRenameState,
        exit_progress: Option<f32>,
        cx: &App,
    ) -> gpui::AnyElement {
        let rename_input = self
            .controllers
            .keychain
            .read(cx)
            .managed_key_rename_input();
        let subtitle = i18n::string_args(
            "dialogs.managed_key_rename.message",
            &[("key_name", &prompt.current_name)],
        );

        let controller_cancel = self.controllers.keychain.clone();
        let controller_confirm = self.controllers.keychain.clone();
        let body = v_flex()
            .w_full()
            .child(surface_text_input_stack(
                i18n::string("dialogs.managed_key_rename.name_label"),
                rename_input,
                TextInputSurface::Highest,
                true,
            ))
            .into_any_element();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "managed-key-rename-cancel",
                    i18n::string("dialogs.common.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_managed_key_rename(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "managed-key-rename-confirm",
                    i18n::string("dialogs.managed_key_rename.confirm"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, window, cx| {
                    controller_confirm.update(cx, |controller, cx| {
                        controller.confirm_managed_key_rename(window, cx);
                    });
                }),
            );

        render_basic_dialog(
            "managed-key-rename",
            i18n::string("dialogs.managed_key_rename.title"),
            Some(subtitle),
            Some(body),
            actions.into_any_element(),
            exit_progress,
        )
    }

    fn render_profile_delete_prompt(
        &self,
        entity: Entity<Self>,
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

        let controller_cancel = self.controllers.session.clone();
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
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_profile_delete(cx);
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
        _entity: Entity<Self>,
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

        let controller_cancel = self.controllers.session.clone();
        let controller_confirm = self.controllers.session.clone();

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
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_snippet_delete(cx);
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
                    controller_confirm.update(cx, |controller, cx| {
                        controller.confirm_snippet_delete(cx);
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

        let controller_cancel = self.controllers.session.clone();
        let controller_confirm = self.controllers.session.clone();

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
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_port_forward_rule_removal(cx);
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
                    controller_confirm.update(cx, |controller, cx| {
                        controller.confirm_port_forward_rule_removal(cx);
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
        prompt: &PendingChatSessionDeleteState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let title = if prompt.title.trim().is_empty() {
            i18n::string("workspace.panel.agent.history.untitled_chat")
        } else {
            prompt.title.clone()
        };
        let subtitle = i18n::string_args("dialogs.chat_delete.message", &[("title", &title)]);

        let controller_cancel = self.controllers.agent.clone();
        let controller_confirm = self.controllers.agent.clone();

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
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_chat_session_delete(cx);
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
                    controller_confirm.update(cx, |controller, cx| {
                        controller.confirm_chat_session_delete(cx);
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
        prompt: &PendingChatSessionRenameState,
        exit_progress: Option<f32>,
        cx: &App,
    ) -> gpui::AnyElement {
        let title_input = self.controllers.agent.read(cx).rename_title_input();
        let current_title = if prompt.current_title.trim().is_empty() {
            i18n::string("workspace.panel.agent.history.untitled_chat")
        } else {
            prompt.current_title.clone()
        };
        let subtitle =
            i18n::string_args("dialogs.chat_rename.message", &[("title", &current_title)]);

        let controller_cancel = self.controllers.agent.clone();
        let controller_confirm = self.controllers.agent.clone();

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
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_chat_session_rename(cx);
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
                    let controller = controller_confirm.clone();
                    let title_input = title_input.clone();
                    move |_, _, cx| {
                        controller.update(cx, |controller, cx| {
                            let new_title = title_input.read(cx).value().to_string();
                            controller.confirm_chat_session_rename(new_title, cx);
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
        _entity: Entity<Self>,
        _prompt: &PendingSyncDirectionState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let controller_cancel = self.controllers.settings.clone();
        let controller_pull = self.controllers.settings.clone();
        let controller_push = self.controllers.settings.clone();

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
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_sync_direction(cx);
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
                    controller_pull.update(cx, |controller, cx| {
                        controller.select_sync_pull(cx);
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
                    controller_push.update(cx, |controller, cx| {
                        controller.select_sync_push(window, cx);
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
        _entity: Entity<Self>,
        prompt: &PendingSyncPullConfirmState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let controller_cancel = self.controllers.settings.clone();
        let controller_force_push = self.controllers.settings.clone();
        let controller_confirm = self.controllers.settings.clone();

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
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_sync_pull_confirm(cx);
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
                            controller_force_push.update(cx, |controller, cx| {
                                controller.confirm_sync_force_push(window, cx);
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
                    controller_confirm.update(cx, |controller, cx| {
                        controller.confirm_sync_pull(window, cx);
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
        _entity: Entity<Self>,
        _prompt: &PendingLocalVaultDisableConfirmState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let controller_cancel = self.controllers.settings.clone();
        let controller_confirm = self.controllers.settings.clone();

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
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_local_vault_disable_confirm(cx);
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
                    controller_confirm.update(cx, |controller, cx| {
                        controller.confirm_local_vault_disable(cx);
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
        _entity: Entity<Self>,
        _prompt: &PendingLocalDataResetConfirmState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let controller_cancel = self.controllers.settings.clone();
        let controller_confirm = self.controllers.settings.clone();

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
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_local_data_reset_confirm(cx);
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
                    controller_confirm.update(cx, |controller, cx| {
                        controller.continue_local_data_reset_confirm(window, cx);
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
        tab_id: TabId,
        challenge: &KbiChallenge,
        exit_progress: Option<f32>,
        cx: &App,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let title: SharedString = if challenge.name.is_empty() {
            i18n::string("prompts.authentication_challenge").into()
        } else {
            challenge.name.clone().into()
        };

        let submit_controller = self.controllers.session.clone();
        let cancel_controller = self.controllers.session.clone();
        let inputs = self
            .controllers
            .session
            .read(cx)
            .keyboard_interactive_inputs();
        let submit_inputs = inputs.clone();

        let submit_button = basic_dialog_action_button(
            "keyboard-interactive-submit",
            i18n::string("dialogs.common.submit"),
            BasicDialogActionTone::Default,
        )
        .on_click(move |_, _, cx| {
            let responses: Vec<String> = submit_inputs
                .iter()
                .map(|input| input.read(cx).value().to_string())
                .collect();
            submit_controller.update(cx, |controller, cx| {
                controller.submit_keyboard_interactive(tab_id, responses, cx);
            });
        });

        let cancel_button = basic_dialog_action_button(
            "keyboard-interactive-cancel",
            i18n::string("dialogs.common.cancel"),
            BasicDialogActionTone::Default,
        )
        .on_click(move |_, _, cx| {
            cancel_controller.update(cx, |controller, cx| {
                controller.cancel_keyboard_interactive(tab_id, cx);
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
                        inputs.get(i).map(|input| {
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
        entity: Entity<Self>,
        mode: LocalVaultPassphrasePopupMode,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
        cx: &App,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let settings_controller = self.controllers.settings.clone();
        let settings_forms = settings_controller.read(cx).forms();
        let operation_in_progress = settings_controller
            .read(cx)
            .local_vault_unlock_in_progress();
        let input = settings_forms.local_vault_passphrase_input.clone();
        let confirmation_input = settings_forms
            .local_vault_passphrase_confirmation_input
            .clone();
        let local_vault_auto_lock_duration_select =
            settings_forms.local_vault_auto_lock_duration_select.clone();
        let entity_cancel = entity.clone();
        let controller_submit = settings_controller.clone();
        let title = settings_controller
            .read(cx)
            .local_vault_passphrase_popup_title(mode);
        let requires_passphrase_confirmation = mode
            == LocalVaultPassphrasePopupMode::ChangePassphrase
            || (mode == LocalVaultPassphrasePopupMode::PrimaryAction
                && settings_controller.read(cx).local_vault_status() == LocalVaultStatus::Disabled);
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
                    reveal_icon: settings_controller
                        .read(cx)
                        .secret_reveal_icon(&SecretRevealTarget::LocalVaultPassphrase),
                },
                {
                    let controller = settings_controller.clone();
                    move |window, cx| {
                        controller.update(cx, |controller, cx| {
                            controller.toggle_secret_visibility(
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
                        reveal_icon: settings_controller.read(cx).secret_reveal_icon(
                            &SecretRevealTarget::LocalVaultPassphraseConfirmation,
                        ),
                    },
                    {
                        let controller = settings_controller.clone();
                        move |window, cx| {
                            controller.update(cx, |controller, cx| {
                                controller.toggle_secret_visibility(
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
                    .label(
                        settings_controller
                            .read(cx)
                            .local_vault_passphrase_popup_title(mode),
                    )
                    .on_click(move |_, window, cx| {
                        controller_submit.update(cx, |controller, cx| {
                            controller.submit_local_vault_passphrase_popup_action(mode, window, cx);
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
        _entity: Entity<Self>,
        _popup: PendingSyncPassphrasePopupState,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
        cx: &App,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let settings_controller = self.controllers.settings.clone();
        let settings_forms = settings_controller.read(cx).forms();
        let operation_in_progress = settings_controller
            .read(cx)
            .sync_passphrase_operation_in_progress();
        let save_in_progress = settings_controller
            .read(cx)
            .sync_passphrase_save_in_progress();
        let input = settings_forms.sync_passphrase_input.clone();
        let confirmation_input = settings_forms.sync_passphrase_confirmation_input.clone();
        let controller_cancel = settings_controller.clone();
        let controller_submit = settings_controller.clone();
        let controller_dismiss = settings_controller.clone();
        let title = settings_controller.read(cx).sync_passphrase_action_label();
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
                    reveal_icon: settings_controller
                        .read(cx)
                        .secret_reveal_icon(&SecretRevealTarget::SyncPassphrase),
                },
                {
                    let controller = settings_controller.clone();
                    move |window, cx| {
                        controller.update(cx, |controller, cx| {
                            controller.toggle_secret_visibility(
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
                    reveal_icon: settings_controller
                        .read(cx)
                        .secret_reveal_icon(&SecretRevealTarget::SyncPassphraseConfirmation),
                },
                {
                    let controller = settings_controller.clone();
                    move |window, cx| {
                        controller.update(cx, |controller, cx| {
                            controller.toggle_secret_visibility(
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
                        controller_cancel.update(cx, |controller, cx| {
                            controller.close_sync_passphrase_popup(window, cx);
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
                        controller_submit.update(cx, |controller, cx| {
                            controller.submit_sync_passphrase_popup_action(window, cx);
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
                controller_dismiss.update(cx, |controller, cx| {
                    controller.close_sync_passphrase_popup(window, cx);
                });
            },
        )
    }

    fn render_ai_provider_popup(
        &self,
        _entity: Entity<Self>,
        _popup: PendingAiProviderPopupState,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
        cx: &App,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let settings_controller = self.controllers.settings.clone();
        let settings_forms = settings_controller.read(cx).forms();
        let kind_select = settings_forms.ai_provider_kind_select.clone();
        let name_input = settings_forms.ai_provider_name_input.clone();
        let model_input = settings_forms.ai_provider_model_input.clone();
        let base_url_input = settings_forms.ai_provider_base_url_input.clone();
        let api_key_input = settings_forms.ai_provider_api_key_input.clone();
        let temperature_input = settings_forms.ai_provider_temperature_input.clone();
        let max_tokens_input = settings_forms.ai_provider_max_tokens_input.clone();
        let context_window_input = settings_forms.ai_provider_context_window_input.clone();
        let editing_provider_id = settings_controller
            .read(cx)
            .editing_ai_provider_id()
            .map(str::to_owned);
        let provider_id = editing_provider_id.clone().unwrap_or_default();
        let save_in_progress = settings_controller.read(cx).ai_provider_save_in_progress();
        let api_key_load_in_progress = settings_controller
            .read(cx)
            .ai_provider_api_key_load_in_progress_for(&provider_id);
        let operation_in_progress = save_in_progress || api_key_load_in_progress;
        let target = SecretRevealTarget::AiProviderApiKey(provider_id);
        let reveal_icon = settings_controller.read(cx).secret_reveal_icon(&target);
        let controller_cancel = settings_controller.clone();
        let controller_submit = settings_controller.clone();
        let controller_dismiss = settings_controller.clone();

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
                    let controller = settings_controller.clone();
                    move |window, cx| {
                        let target = target.clone();
                        controller.update(cx, |controller, cx| {
                            controller.toggle_secret_visibility(target, window, cx);
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
                let controller_delete = settings_controller.clone();
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
                        controller_delete.update(cx, |controller, cx| {
                            controller.delete_ai_provider(provider_id, window, cx);
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
                                controller_cancel.update(cx, |controller, cx| {
                                    controller.close_ai_provider_popup(window, cx);
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
                                controller_submit.update(cx, |controller, cx| {
                                    controller.submit_ai_provider_save(window, cx);
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
                controller_dismiss.update(cx, |controller, cx| {
                    controller.close_ai_provider_popup(window, cx);
                });
            },
        )
    }

    fn render_web_search_config_popup(
        &self,
        _entity: Entity<Self>,
        _popup: PendingWebSearchConfigPopupState,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
        cx: &App,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let settings_controller = self.controllers.settings.clone();
        let settings_forms = settings_controller.read(cx).forms();
        let kind_select = settings_forms.web_search_kind_select.clone();
        let api_key_input = settings_forms.web_search_api_key_input.clone();
        let endpoint_input = settings_forms.web_search_endpoint_input.clone();
        let max_results_input = settings_forms.web_search_max_results_input.clone();
        let save_in_progress = settings_controller.read(cx).web_search_save_in_progress();
        let current_kind = settings_controller.read(cx).settings().web_search.kind;
        let api_key_required = current_kind.requires_api_key();
        let endpoint_required = current_kind == miaominal_settings::WebSearchProviderKind::SearXng;
        let target = SecretRevealTarget::WebSearchApiKey;
        let reveal_icon = settings_controller.read(cx).secret_reveal_icon(&target);
        let controller_cancel = settings_controller.clone();
        let controller_submit = settings_controller.clone();
        let controller_toggle = settings_controller.clone();
        let controller_dismiss = settings_controller.clone();

        let popup_body = v_flex()
            .w_full()
            .gap_5()
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(field_label(
                        i18n::string("settings.web_search.kind.label"),
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
            .child(surface_secret_text_input_stack(
                i18n::string("settings.web_search.api_key.label"),
                api_key_input,
                crate::ui::components::SecretTextInputStackOptions {
                    surface: TextInputSurface::Low,
                    size: gpui_component::Size::Large,
                    required: api_key_required,
                    disabled: save_in_progress,
                    trailing: None,
                    reveal_icon,
                },
                move |window, cx| {
                    controller_toggle.update(cx, |controller, cx| {
                        controller.toggle_secret_visibility(
                            SecretRevealTarget::WebSearchApiKey,
                            window,
                            cx,
                        );
                    });
                },
            ))
            .child(surface_text_input_stack(
                i18n::string("settings.web_search.endpoint.label"),
                endpoint_input,
                TextInputSurface::Low,
                endpoint_required,
            ))
            .child(surface_text_input_stack(
                i18n::string("settings.web_search.max_results.label"),
                max_results_input,
                TextInputSurface::Low,
                true,
            ))
            .into_any_element();

        let actions = h_flex()
            .w_full()
            .justify_end()
            .gap_3()
            .child(
                Button::new("web-search-config-popup-cancel")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .disabled(save_in_progress)
                    .text_color(rgb(roles.on_surface_variant))
                    .label(i18n::string("dialogs.common.cancel"))
                    .on_click(move |_, window, cx| {
                        controller_cancel.update(cx, |controller, cx| {
                            controller.close_web_search_config_popup(window, cx);
                        });
                    }),
            )
            .child(if save_in_progress {
                div()
                    .id("web-search-config-popup-submit-spinner")
                    .min_w(px(116.0))
                    .min_h(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(crate::ui::components::md3_spinner(18.0))
                    .into_any_element()
            } else {
                Button::new("web-search-config-popup-submit")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .text_color(rgb(roles.primary))
                    .label(i18n::string("settings.web_search.actions.save"))
                    .on_click(move |_, window, cx| {
                        controller_submit.update(cx, |controller, cx| {
                            controller.submit_web_search_settings_save(window, cx);
                        });
                    })
                    .into_any_element()
            })
            .into_any_element();

        render_bottom_popup(
            bottom_popup_panel(
                i18n::string("settings.web_search.group.title"),
                Some(i18n::string("settings.web_search.group.description")),
                Some(popup_body),
                actions,
                bottom_popup_viewport_height,
            ),
            "web-search-config",
            exit_progress,
            move |window, cx| {
                controller_dismiss.update(cx, |controller, cx| {
                    controller.close_web_search_config_popup(window, cx);
                });
            },
        )
    }

    fn render_sync_provider_config_popup(
        &self,
        _entity: Entity<Self>,
        popup: PendingSyncProviderConfigPopupState,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
        cx: &App,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let settings_controller = self.controllers.settings.clone();
        let settings_forms = settings_controller.read(cx).forms();
        let save_in_progress = settings_controller
            .read(cx)
            .sync_provider_config_save_in_progress_for(popup.provider);
        let controller_cancel = settings_controller.clone();
        let controller_submit = settings_controller.clone();
        let controller_dismiss = settings_controller.clone();

        let (title, description, popup_body, popup_key) = match popup.provider {
            SyncProvider::GithubGist => {
                let token_input = settings_forms.sync_github_token_input.clone();
                let gist_id_input = settings_forms.sync_github_gist_id_input.clone();
                let target = SecretRevealTarget::SyncGithubToken;
                let reveal_icon = settings_controller.read(cx).secret_reveal_icon(&target);
                let controller_toggle = settings_controller.clone();
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
                                controller_toggle.update(cx, |controller, cx| {
                                    controller.toggle_secret_visibility(
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
                let url_input = settings_forms.sync_webdav_url_input.clone();
                let username_input = settings_forms.sync_webdav_username_input.clone();
                let password_input = settings_forms.sync_webdav_password_input.clone();
                let target = SecretRevealTarget::SyncWebdavPassword;
                let reveal_icon = settings_controller.read(cx).secret_reveal_icon(&target);
                let controller_toggle = settings_controller.clone();
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
                                controller_toggle.update(cx, |controller, cx| {
                                    controller.toggle_secret_visibility(
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
                        controller_cancel.update(cx, |controller, cx| {
                            controller.close_sync_provider_config_popup(window, cx);
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
                    .on_click(move |_, _window, cx| {
                        controller_submit.update(cx, |controller, cx| {
                            controller.submit_sync_provider_config_popup_action(cx);
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
                controller_dismiss.update(cx, |controller, cx| {
                    controller.close_sync_provider_config_popup(window, cx);
                });
            },
        )
    }

    fn render_sync_passphrase_clear_confirm_popup(
        &self,
        _entity: Entity<Self>,
        _popup: PendingSyncPassphraseClearConfirmPopupState,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
        cx: &App,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let settings_controller = self.controllers.settings.clone();
        let operation_in_progress = settings_controller
            .read(cx)
            .sync_passphrase_operation_in_progress();
        let controller_cancel = settings_controller.clone();
        let controller_submit = settings_controller.clone();
        let controller_dismiss = settings_controller.clone();
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
                        controller_cancel.update(cx, |controller, cx| {
                            controller.close_sync_passphrase_clear_confirm_popup(cx);
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
                    .on_click(move |_, _window, cx| {
                        controller_submit.update(cx, |controller, cx| {
                            controller.submit_sync_passphrase_clear_confirm_popup_action(cx);
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
                controller_dismiss.update(cx, |controller, cx| {
                    controller.close_sync_passphrase_clear_confirm_popup(cx);
                });
            },
        )
    }

    fn render_local_data_reset_confirmation_popup(
        &self,
        _entity: Entity<Self>,
        _popup: PendingLocalDataResetConfirmationPopupState,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
        cx: &App,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let settings_controller = self.controllers.settings.clone();
        let input = settings_controller
            .read(cx)
            .forms()
            .local_data_reset_confirmation_input;
        let controller_cancel = settings_controller.clone();
        let controller_submit = settings_controller.clone();
        let controller_dismiss = settings_controller.clone();
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
                        controller_cancel.update(cx, |controller, cx| {
                            controller.close_local_data_reset_confirmation_popup(window, cx);
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
                        controller_submit.update(cx, |controller, cx| {
                            controller
                                .submit_local_data_reset_confirmation_popup_action(window, cx);
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
                controller_dismiss.update(cx, |controller, cx| {
                    controller.close_local_data_reset_confirmation_popup(window, cx);
                });
            },
        )
    }

    fn render_session_host_key_prompt(
        &self,
        tab_id: TabId,
        prompt: &HostKeyPrompt,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
        cx: &App,
    ) -> gpui::AnyElement {
        let controller = self.controllers.session.clone();
        let action_controller = controller.clone();
        controller.read(cx).render_trusted_host_key_prompt(
            move |decision, cx| {
                action_controller.update(cx, |controller, cx| {
                    controller.resolve_host_key_prompt(tab_id, decision, cx);
                });
            },
            prompt,
            exit_progress,
            bottom_popup_viewport_height,
        )
    }

    fn render_exiting_dialog_overlay(
        &self,
        entity: Entity<Self>,
        snapshot: DialogOverlaySnapshot,
        exit_progress: f32,
        bottom_popup_viewport_height: f32,
        cx: &App,
    ) -> gpui::AnyElement {
        match snapshot {
            DialogOverlaySnapshot::HostKey { tab_id, prompt } => self
                .render_session_host_key_prompt(
                    tab_id,
                    &prompt,
                    Some(exit_progress),
                    bottom_popup_viewport_height,
                    cx,
                ),
            DialogOverlaySnapshot::KeyboardInteractive { tab_id, challenge } => {
                self.render_keyboard_interactive_prompt(tab_id, &challenge, Some(exit_progress), cx)
            }
            DialogOverlaySnapshot::ProfileDelete(prompt) => {
                self.render_profile_delete_prompt(entity, &prompt, Some(exit_progress))
            }
            DialogOverlaySnapshot::ManagedKeyDelete(prompt) => {
                self.render_managed_key_delete_prompt(entity, &prompt, Some(exit_progress))
            }
            DialogOverlaySnapshot::ManagedKeyRename(prompt) => {
                self.render_managed_key_rename_prompt(&prompt, Some(exit_progress), cx)
            }
            DialogOverlaySnapshot::KnownHostDelete(prompt) => {
                let controller = self.controllers.session.clone();
                controller.read(cx).render_trusted_known_host_delete_prompt(
                    controller.clone(),
                    &prompt,
                    Some(exit_progress),
                )
            }
            DialogOverlaySnapshot::SnippetDelete(prompt) => {
                self.render_snippet_delete_prompt(entity, &prompt, Some(exit_progress))
            }
            DialogOverlaySnapshot::PortForwardRuleDelete(prompt) => {
                self.render_port_forward_rule_delete_prompt(&prompt, Some(exit_progress))
            }
            DialogOverlaySnapshot::ChatSessionDelete(prompt) => {
                self.render_chat_session_delete_prompt(&prompt, Some(exit_progress))
            }
            DialogOverlaySnapshot::ChatSessionRename(prompt) => {
                self.render_chat_session_rename_prompt(&prompt, Some(exit_progress), cx)
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
                    cx,
                ),
            DialogOverlaySnapshot::SyncPassphraseClearConfirmPopup(popup) => self
                .render_sync_passphrase_clear_confirm_popup(
                    entity,
                    popup,
                    Some(exit_progress),
                    bottom_popup_viewport_height,
                    cx,
                ),
            DialogOverlaySnapshot::SyncPassphrasePopup(popup) => self.render_sync_passphrase_popup(
                entity,
                popup,
                Some(exit_progress),
                bottom_popup_viewport_height,
                cx,
            ),
            DialogOverlaySnapshot::AiProviderPopup(popup) => self.render_ai_provider_popup(
                entity,
                popup,
                Some(exit_progress),
                bottom_popup_viewport_height,
                cx,
            ),
            DialogOverlaySnapshot::WebSearchConfigPopup(popup) => self
                .render_web_search_config_popup(
                    entity,
                    popup,
                    Some(exit_progress),
                    bottom_popup_viewport_height,
                    cx,
                ),
            DialogOverlaySnapshot::SyncProviderConfigPopup(popup) => self
                .render_sync_provider_config_popup(
                    entity,
                    popup,
                    Some(exit_progress),
                    bottom_popup_viewport_height,
                    cx,
                ),
            DialogOverlaySnapshot::LocalVaultPassphrasePopup(mode) => self
                .render_local_vault_passphrase_popup(
                    entity,
                    mode,
                    Some(exit_progress),
                    bottom_popup_viewport_height,
                    cx,
                ),
            DialogOverlaySnapshot::SftpPrompt { tab_id, prompt } => {
                let controller = self.controllers.sftp.clone();
                controller.read(cx).render_sftp_prompt_overlay(
                    controller.clone(),
                    tab_id,
                    &prompt,
                    Some(exit_progress),
                )
            }
        }
    }
}
