use super::*;

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

fn suspended_page_editor_sidebar_kind(
    visible: Option<PageEditorSidebarKind>,
    transition: Option<PageEditorSidebarTransition>,
) -> Option<PageEditorSidebarKind> {
    transition.map_or(visible, |transition| match transition.phase {
        PageEditorSidebarTransitionPhase::Entering => Some(transition.kind),
        PageEditorSidebarTransitionPhase::Exiting => None,
    })
}

fn page_editor_sidebar_matches_view(kind: PageEditorSidebarKind, view: PrimaryViewKind) -> bool {
    matches!(
        (kind, view),
        (
            PageEditorSidebarKind::Hosts,
            PrimaryViewKind::Sidebar(SidebarSection::Hosts)
        ) | (
            PageEditorSidebarKind::PortForwarding,
            PrimaryViewKind::Sidebar(SidebarSection::PortForwarding),
        ) | (
            PageEditorSidebarKind::Snippets,
            PrimaryViewKind::Sidebar(SidebarSection::Snippets),
        ) | (
            PageEditorSidebarKind::Keychain,
            PrimaryViewKind::Sidebar(SidebarSection::Keychain),
        ) | (
            PageEditorSidebarKind::KnownHosts,
            PrimaryViewKind::Sidebar(SidebarSection::KnownHosts),
        )
    )
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
            let overlays = self.render_window_overlays(bottom_popup_viewport_height, window, cx);
            let onboarding_page =
                pages::render_onboarding_page(self.controllers.settings.clone(), None, window, cx);
            return div()
                .size_full()
                .relative()
                .capture_any_mouse_down(cx.listener(|this, _, window, cx| {
                    this.remember_overlay_trigger_focus(window, cx)
                }))
                .child(onboarding_page)
                .child(overlays)
                .into_any_element();
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
        let primary_view_animating = primary_view_transition.animating;
        let root_right_gutter = self.primary_view_root_right_gutter(primary_view_transition);

        let overlays = self.render_window_overlays(bottom_popup_viewport_height, window, cx);

        let current_page_editor_sidebar = if has_active_session || has_active_sftp_tab {
            None
        } else {
            self.page_editor_sidebar_for_view(desired_primary_view, cx)
        };
        let show_host_editor_sidebar =
            current_page_editor_sidebar == Some(PageEditorSidebarKind::Hosts);
        let show_port_forward_editor_sidebar =
            current_page_editor_sidebar == Some(PageEditorSidebarKind::PortForwarding);
        let show_snippets_editor_sidebar =
            current_page_editor_sidebar == Some(PageEditorSidebarKind::Snippets);
        let show_keychain_editor_sidebar =
            current_page_editor_sidebar == Some(PageEditorSidebarKind::Keychain);
        let show_known_hosts_sidebar =
            current_page_editor_sidebar == Some(PageEditorSidebarKind::KnownHosts);
        let page_editor_sidebar =
            if primary_view_animating || has_active_session || has_active_sftp_tab {
                self.suspend_page_editor_sidebar_transition();
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
            .font(miaominal_settings::interface_font())
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
            .capture_any_mouse_down(
                cx.listener(|this, _, window, cx| this.remember_overlay_trigger_focus(window, cx)),
            )
            .child(self.render_top_bar(entity.clone(), window, cx))
            .child(shell_body)
            .child(self.render_status_footer(entity.clone(), cx))
            .child(overlays)
            .into_any_element()
    }
}

impl AppView {
    fn active_terminal_tab_id(&self, cx: &App) -> Option<TabId> {
        let tab_id = self.workspace.active_topbar_tab?;
        self.session_tab(tab_id, cx)
            .is_some_and(|session| session.purpose() == SessionPurpose::Terminal)
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
                .is_some_and(|session| session.purpose() == SessionPurpose::Terminal),
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

    fn finish_primary_view_transition(&mut self, desired: PrimaryViewKind, cx: &App) {
        self.workspace.primary_view_transition = None;
        self.shell.shell_state.visible_page_editor_sidebar =
            self.page_editor_sidebar_for_view(desired, cx);
        self.shell.shell_state.page_editor_sidebar_transition = None;
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
            self.shell.shell_state.visible_page_editor_sidebar =
                self.page_editor_sidebar_for_view(desired, cx);
            self.shell.shell_state.page_editor_sidebar_transition = None;
            return PrimaryViewTransitionRenderState {
                from: desired,
                to: desired,
                visibility: 1.0,
                animating: false,
            };
        }

        let duration_seconds = transition.duration.as_secs_f32();
        if duration_seconds <= f32::EPSILON {
            self.finish_primary_view_transition(desired, cx);
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
            self.finish_primary_view_transition(desired, cx);
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
                let controller = self.controllers.hosts.clone();
                let connect_controller = controller.clone();
                let edit_controller = controller.clone();
                let sftp_controller = controller.clone();
                controller.read(cx).render_hosts_page(
                    controller.clone(),
                    move |index, _window, cx| {
                        connect_controller.update(cx, |controller, cx| {
                            if let Some(index) = controller.profile_index(&index) {
                                controller.request_session_at_profile_index(index, cx);
                            }
                        });
                    },
                    move |index, _window, cx| {
                        edit_controller.update(cx, |controller, cx| {
                            if let Some(index) = controller.profile_index(&index) {
                                controller.request_profile_editor_at_index(index, cx);
                            }
                        });
                    },
                    move |index, _window, cx| {
                        sftp_controller.update(cx, |controller, cx| {
                            if let Some(index) = controller.profile_index(&index) {
                                controller.request_sftp_at_profile_index(index, cx);
                            }
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
                let controller = self.controllers.forwarding.clone();
                controller
                    .read(cx)
                    .render_forward_page(controller.clone(), cx)
            }
            SidebarSection::KnownHosts => {
                let controller = self.controllers.trusted_hosts.clone();
                controller
                    .read(cx)
                    .render_trusted_page(controller.clone(), cx)
            }
            SidebarSection::Settings => {
                let settings = self.controllers.settings.clone();
                let (settings_instance_generation, destination) =
                    settings.read(cx).settings_render_request();
                pages::render_settings_page(settings, settings_instance_generation, destination)
            }
            SidebarSection::Snippets => {
                let controller = self.controllers.snippets.clone();
                let edit_controller = controller.clone();
                controller.read(cx).render_snippets_page(
                    controller.clone(),
                    move |index, window, cx| {
                        edit_controller.update(cx, |controller, cx| {
                            controller.open_existing_snippet_editor(&index, window, cx);
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

    fn suspend_page_editor_sidebar_transition(&mut self) {
        self.shell.shell_state.visible_page_editor_sidebar = suspended_page_editor_sidebar_kind(
            self.shell.shell_state.visible_page_editor_sidebar,
            self.shell.shell_state.page_editor_sidebar_transition,
        );
        self.shell.shell_state.page_editor_sidebar_transition = None;
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

    pub(in crate::ui::shell) fn page_editor_sidebar_for_view(
        &self,
        view: PrimaryViewKind,
        cx: &App,
    ) -> Option<PageEditorSidebarKind> {
        let PrimaryViewKind::Sidebar(section) = view else {
            return None;
        };
        let active_hosts_tab = self
            .workspace
            .active_topbar_tab
            .and_then(|tab_id| self.workspace.tabs.get(tab_id))
            .is_some_and(|tab| tab.is_hosts());
        if active_hosts_tab {
            let kind = restored_page_editor_sidebar(
                &self.workspace.tabs,
                self.workspace.active_topbar_tab,
            )
            .filter(|kind| page_editor_sidebar_matches_view(*kind, view))?;
            let is_open = match kind {
                PageEditorSidebarKind::Hosts => {
                    self.controllers
                        .hosts
                        .read(cx)
                        .editor_state()
                        .host_editor_open
                }
                PageEditorSidebarKind::PortForwarding => {
                    self.controllers.forwarding.read(cx).editor_open()
                }
                PageEditorSidebarKind::Snippets => self.controllers.snippets.read(cx).editor_open(),
                PageEditorSidebarKind::Keychain => self.controllers.keychain.read(cx).editor_open(),
                PageEditorSidebarKind::KnownHosts => self
                    .controllers
                    .trusted_hosts
                    .read(cx)
                    .selected_known_host()
                    .is_some(),
            };
            return is_open.then_some(kind);
        }
        match section {
            SidebarSection::Hosts
                if self
                    .controllers
                    .hosts
                    .read(cx)
                    .editor_state()
                    .host_editor_open =>
            {
                Some(PageEditorSidebarKind::Hosts)
            }
            SidebarSection::PortForwarding
                if self.controllers.forwarding.read(cx).editor_open() =>
            {
                Some(PageEditorSidebarKind::PortForwarding)
            }
            SidebarSection::Snippets if self.controllers.snippets.read(cx).editor_open() => {
                Some(PageEditorSidebarKind::Snippets)
            }
            SidebarSection::Keychain if self.controllers.keychain.read(cx).editor_open() => {
                Some(PageEditorSidebarKind::Keychain)
            }
            SidebarSection::KnownHosts
                if self
                    .controllers
                    .trusted_hosts
                    .read(cx)
                    .selected_known_host()
                    .is_some() =>
            {
                Some(PageEditorSidebarKind::KnownHosts)
            }
            _ => None,
        }
    }

    fn render_primary_transition_surface(
        &mut self,
        view: PrimaryViewKind,
        sidebar: Option<PageEditorSidebarKind>,
        entity: Entity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (panel, mut state) = self.render_primary_view_panel(view, None, window, cx);
        state.show_host_editor_sidebar = sidebar == Some(PageEditorSidebarKind::Hosts);
        state.show_port_forward_editor_sidebar =
            sidebar == Some(PageEditorSidebarKind::PortForwarding);
        state.show_snippets_editor_sidebar = sidebar == Some(PageEditorSidebarKind::Snippets);
        state.show_keychain_editor_sidebar = sidebar == Some(PageEditorSidebarKind::Keychain);
        state.show_known_hosts_sidebar = sidebar == Some(PageEditorSidebarKind::KnownHosts);

        // The retained editor travels with its page at full width. Its independent
        // open animation must not restart after the page transition finishes.
        div()
            .size_full()
            .flex()
            .items_stretch()
            .child(self.render_primary_surface_layer(entity.clone(), panel, state, cx))
            .when_some(sidebar, |this, kind| {
                this.child(self.render_page_editor_sidebar(
                    PageEditorSidebarRenderState {
                        kind,
                        visibility: 1.0,
                    },
                    entity,
                    cx,
                ))
            })
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

        let incoming_sidebar = self.page_editor_sidebar_for_view(transition.to, cx);
        let outgoing_sidebar = self
            .shell
            .shell_state
            .visible_page_editor_sidebar
            .filter(|kind| page_editor_sidebar_matches_view(*kind, transition.from))
            .or_else(|| self.page_editor_sidebar_for_view(transition.from, cx));
        let incoming_surface = self.render_primary_transition_surface(
            transition.to,
            incoming_sidebar,
            entity.clone(),
            window,
            cx,
        );
        let outgoing_surface = self.render_primary_transition_surface(
            transition.from,
            outgoing_sidebar,
            entity.clone(),
            window,
            cx,
        );
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
                let controller = self.controllers.hosts.clone();
                let session = self.controllers.session.clone();
                let connection_test = hosts::HostConnectionTest::new(
                    session.read(cx).connection_test_in_progress(),
                    move |cx| {
                        session.update(cx, |controller, cx| {
                            controller.cancel_profile_connection_test(cx);
                        });
                    },
                );
                let test_controller = controller.clone();
                let save_controller = controller.clone();
                let save_settings = self.controllers.settings.clone();
                let visibility_controller = controller.clone();
                controller
                    .read(cx)
                    .render_hosts_editor_sidebar(
                        controller.clone(),
                        connection_test,
                        move |window, cx| {
                            test_controller.update(cx, |controller, cx| {
                                controller.request_profile_connection_test(window, cx);
                            });
                        },
                        move |window, cx| {
                            let requires_unlock =
                                save_settings.read(cx).sync_requires_local_vault_unlock();
                            save_controller.update(cx, |controller, cx| {
                                controller.request_profile_save(requires_unlock, window, cx);
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
                let controller = self.controllers.forwarding.clone();
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
                let controller = self.controllers.snippets.clone();
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
                let controller = self.controllers.trusted_hosts.clone();
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
                                .forwarding
                                .read(cx)
                                .render_forward_fab(self.controllers.forwarding.clone()),
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
                        let controller = self.controllers.snippets.clone();
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
                        let controller = self.controllers.trusted_hosts.clone();
                        controller
                            .read(cx)
                            .render_known_hosts_refresh_fab(controller.clone())
                    }))
                },
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sidebar_transition(
        kind: PageEditorSidebarKind,
        phase: PageEditorSidebarTransitionPhase,
    ) -> PageEditorSidebarTransition {
        PageEditorSidebarTransition {
            kind,
            phase,
            started_at: Instant::now(),
            duration: Duration::ZERO,
        }
    }

    #[test]
    fn suspending_sidebar_transition_preserves_logical_editor_visibility() {
        assert_eq!(
            suspended_page_editor_sidebar_kind(Some(PageEditorSidebarKind::Hosts), None),
            Some(PageEditorSidebarKind::Hosts)
        );
        assert_eq!(
            suspended_page_editor_sidebar_kind(
                None,
                Some(sidebar_transition(
                    PageEditorSidebarKind::Hosts,
                    PageEditorSidebarTransitionPhase::Entering,
                )),
            ),
            Some(PageEditorSidebarKind::Hosts)
        );
        assert_eq!(
            suspended_page_editor_sidebar_kind(
                Some(PageEditorSidebarKind::Hosts),
                Some(sidebar_transition(
                    PageEditorSidebarKind::Hosts,
                    PageEditorSidebarTransitionPhase::Exiting,
                )),
            ),
            None
        );
    }
}
