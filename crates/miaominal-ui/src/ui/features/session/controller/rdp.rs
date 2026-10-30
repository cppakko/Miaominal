use super::*;
use crate::ui::contracts::{TabOpenRequest, overlay::OverlaySnapshot};
use miaominal_services::{
    RdpConnectionMode, RdpLaunchOptions, RdpRuntimeState, RdpSessionSnapshot, RdpSnapshot,
};

#[derive(Clone)]
pub(in crate::ui::features::session) struct RdpLaunchDraft {
    pub(in crate::ui::features::session) profile_id: String,
    pub(in crate::ui::features::session) name: String,
    pub(in crate::ui::features::session) mode: RdpConnectionMode,
    pub(in crate::ui::features::session) reuse: bool,
    /// i18n key explaining why the SSH password cannot be reused for this launch.
    pub(in crate::ui::features::session) reuse_unavailable: Option<&'static str>,
    /// Remote host as typed in the profile; the direct option connects to `host:3389`.
    pub(in crate::ui::features::session) host: String,
    /// `user@host[:port]` of the SSH connection the tunnel option goes through.
    pub(in crate::ui::features::session) ssh_target: String,
    pub(in crate::ui::features::session) username: String,
}

/// Parses the launch panel's port field. Empty means the default RDP port.
pub(in crate::ui::features::session) fn parse_rdp_port(text: &str) -> Option<u16> {
    let text = text.trim();
    if text.is_empty() {
        return Some(RdpLaunchOptions::DEFAULT_PORT);
    }
    text.parse::<u16>().ok().filter(|port| *port != 0)
}

fn ssh_target(profile: &SessionProfile) -> String {
    let host = if profile.host.contains(':') {
        format!("[{}]", profile.host)
    } else {
        profile.host.clone()
    };
    let address = if profile.port == 22 {
        host
    } else {
        format!("{host}:{}", profile.port)
    };
    if profile.username.trim().is_empty() {
        address
    } else {
        format!("{}@{address}", profile.username)
    }
}

/// Why "reuse SSH password" cannot apply, so the option is never offered and then
/// silently ignored by the service.
fn reuse_unavailable_reason(profile: &SessionProfile) -> Option<&'static str> {
    if profile.effective_auth_method() != AuthMethod::Password {
        return Some("rdp.reuse_unavailable_auth");
    }
    if profile.password.is_empty() && !profile.has_stored_password {
        return Some("rdp.reuse_unavailable_no_password");
    }
    match miaominal_services::resolve_rdp_client(&miaominal_settings::current_settings().rdp_client)
    {
        Ok(client) if client.supports_password() => None,
        Ok(_) => Some("rdp.reuse_unavailable_client"),
        // A missing client is reported when launching; nothing could receive a password.
        Err(_) => Some("rdp.reuse_unavailable_client"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        Bounds, IntoElement, Render, Subscription, WindowBounds, WindowOptions, div, prelude::*,
        px, size,
    };

    struct LaunchFixture {
        owner: Entity<SessionController>,
        snapshot: OverlaySnapshot,
        _owner_subscription: Subscription,
    }
    impl Render for LaunchFixture {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(self.snapshot.render(None, 700., cx))
        }
    }

    #[gpui_kit::test]
    fn rendered_launch_switches_mode_and_rejects_retained_callbacks(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let (handle, fixture) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        Default::default(),
                        size(px(800.), px(700.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |_, cx| {
                    cx.new(|cx| {
                        let owner = cx.new(|_| {
                            SessionController::new_for_test_with_profiles(vec![
                                SessionProfile::blank("host", 1),
                            ])
                        });
                        owner.update(cx, |owner, cx| owner.open_rdp_launch("host", cx));
                        let snapshot = owner.read(cx).rdp_launch_overlay(owner.clone()).unwrap();
                        let subscription = cx.observe(&owner, |_, _, cx| cx.notify());
                        LaunchFixture {
                            owner,
                            snapshot,
                            _owner_subscription: subscription,
                        }
                    })
                },
            )
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            let owner = fixture.read(cx).owner.clone();
            window.render_frame(cx);
            window.within("rdp-mode").click(1usize, cx);
            assert_eq!(
                owner.read(cx).rdp_launch_draft().unwrap().mode,
                RdpConnectionMode::Direct
            );
            owner.update(cx, |owner, cx| {
                owner.set_rdp_launch(Some(RdpConnectionMode::SshTunnel), None, cx)
            });
            window.render_frame(cx);
            for _ in 0..6 {
                if window.within("rdp-mode").find(1usize).focused() == Some(true) {
                    break;
                }
                window.focus_next(cx);
                window.render_frame(cx);
            }
            assert_eq!(window.within("rdp-mode").find(1usize).focused(), Some(true));
            window.press("space", cx);
            assert_eq!(
                owner.read(cx).rdp_launch_draft().unwrap().mode,
                RdpConnectionMode::Direct
            );
            let old = fixture.read(cx).snapshot.clone();
            owner.update(cx, |owner, cx| owner.open_rdp_launch("host", cx));
            assert!(old.submit(window, cx));
            old.dismiss(window, cx);
            assert_eq!(
                owner.read(cx).rdp_launch_draft().unwrap().mode,
                RdpConnectionMode::SshTunnel
            );
            let fresh = owner.read(cx).rdp_launch_overlay(owner.clone()).unwrap();
            fixture.update(cx, |fixture, cx| {
                fixture.snapshot = fresh.clone();
                cx.notify();
            });
            window.render_frame(cx);
            window.click("rdp-launch-submit", cx);
            assert!(owner.read(cx).rdp_launch_draft().is_none());
            assert!(fresh.submit(window, cx));
            assert!(owner.read(cx).rdp_launch_draft().is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn launch_is_transient_and_old_generations_cannot_act(cx: &mut gpui_kit::TestAppContext) {
        cx.update(gpui_kit::init);
        let mut profile = SessionProfile::blank("host", 1);
        profile.name = "远程主机".into();
        let owner = cx
            .update(|cx| cx.new(|_| SessionController::new_for_test_with_profiles(vec![profile])));
        owner.update(cx, |owner, cx| {
            owner.open_rdp_launch("host", cx);
            let revision = owner.rdp_launch_revision.get();
            assert_eq!(
                owner.rdp_launch_draft().unwrap().mode,
                RdpConnectionMode::SshTunnel
            );
            assert!(!owner.rdp_launch_draft().unwrap().reuse);
            owner.set_rdp_launch(Some(RdpConnectionMode::Direct), Some(true), cx);
            owner.open_rdp_launch("host", cx);
            assert!(!owner.is_rdp_launch_current(revision));
            assert_eq!(
                owner.rdp_launch_draft().unwrap().mode,
                RdpConnectionMode::SshTunnel
            );
            assert!(!owner.rdp_launch_draft().unwrap().reuse);
            owner.submit_rdp_launch(cx);
            owner.submit_rdp_launch(cx);
            assert!(owner.rdp_launch_draft().is_none());
            let mut local = SessionProfile::blank("local", 2);
            local.kind = miaominal_core::profile::ProfileKind::Local;
            owner.replace_profiles_from_application(vec![local]);
            owner.open_rdp_launch("local", cx);
            assert!(owner.rdp_launch_draft().is_none());
        });
    }

    struct BlankView;
    impl Render for BlankView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    #[test]
    fn port_field_accepts_only_real_ports() {
        assert_eq!(parse_rdp_port(""), Some(3389));
        assert_eq!(parse_rdp_port(" 13389 "), Some(13389));
        assert_eq!(parse_rdp_port("65535"), Some(65535));
        for invalid in ["0", "65536", "-1", "33 89", "rdp"] {
            assert_eq!(parse_rdp_port(invalid), None, "{invalid}");
        }
    }

    #[gpui_kit::test]
    fn invalid_port_keeps_the_panel_open_and_reopening_resets_it(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let (handle, owner) = cx.update(|cx| {
            let owner = cx.new(|_| {
                SessionController::new_for_test_with_profiles(vec![SessionProfile::blank(
                    "host", 1,
                )])
            });
            let (handle, _) =
                gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| BlankView))
                    .unwrap();
            (handle, owner)
        });
        cx.update_window(handle, |_, window, cx| {
            owner.update(cx, |owner, cx| {
                owner.open_rdp_launch("host", cx);
                owner.sync_rdp_launch_input(window, cx);
            });
            let input = owner.read(cx).rdp_port_input().unwrap();
            assert_eq!(input.read(cx).value().as_ref(), "3389");
            set_input_value(&input, "70000", window, cx);
            owner.update(cx, |owner, cx| owner.submit_rdp_launch(cx));
            assert!(owner.read(cx).rdp_launch_draft().is_some());
            set_input_value(&input, "13389", window, cx);
            assert_eq!(owner.read(cx).rdp_launch_port(cx), Some(13389));
            owner.update(cx, |owner, cx| {
                owner.dismiss_rdp_launch(cx);
                owner.open_rdp_launch("host", cx);
                owner.sync_rdp_launch_input(window, cx);
            });
            let fresh = owner.read(cx).rdp_port_input().unwrap();
            assert_eq!(fresh.read(cx).value().as_ref(), "3389");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn reuse_is_only_offered_when_it_can_apply(cx: &mut gpui_kit::TestAppContext) {
        cx.update(gpui_kit::init);
        let mut key_profile = SessionProfile::blank("key", 1);
        key_profile.auth_method = Some(AuthMethod::Agent);
        let no_password = SessionProfile::blank("empty", 2);
        let owner = cx.update(|cx| {
            cx.new(|_| {
                SessionController::new_for_test_with_profiles(vec![key_profile, no_password])
            })
        });
        owner.update(cx, |owner, cx| {
            for (id, reason) in [
                ("key", "rdp.reuse_unavailable_auth"),
                ("empty", "rdp.reuse_unavailable_no_password"),
            ] {
                owner.open_rdp_launch(id, cx);
                assert_eq!(
                    owner.rdp_launch_draft().unwrap().reuse_unavailable,
                    Some(reason)
                );
                owner.set_rdp_launch(None, Some(true), cx);
                assert!(!owner.rdp_launch_draft().unwrap().reuse);
                owner.dismiss_rdp_launch(cx);
            }
            // A pending close confirmation survives another launch request.
            *owner.rdp_confirmation.borrow_mut() = Some(RdpConfirmation::Disconnect(TabId::new(7)));
            owner.bump_rdp_confirmation_revision();
            let revision = owner.rdp_confirmation_revision.get();
            owner.open_rdp_launch("key", cx);
            assert!(owner.rdp_launch_draft().is_none());
            assert!(owner.is_rdp_confirmation_current(revision));
        });
    }
}
#[derive(Clone)]
pub(super) enum RdpConfirmation {
    Close(Vec<TabId>),
    Disconnect(TabId),
}

pub(in crate::ui::features::session) fn status(runtime: &RdpSessionSnapshot) -> String {
    match runtime.state() {
        RdpRuntimeState::Starting => i18n::string("rdp.starting"),
        RdpRuntimeState::WaitingForClient => i18n::string("rdp.client_started"),
        RdpRuntimeState::ClientRunning => i18n::string_args(
            "rdp.active",
            &[("count", &runtime.active_connections().to_string())],
        ),
        RdpRuntimeState::Reconnecting => i18n::string("rdp.reconnecting"),
        RdpRuntimeState::Stopped => i18n::string("rdp.stopped"),
        RdpRuntimeState::Failed(error) => error.clone(),
    }
}

impl SessionController {
    pub(super) fn rdp_service(&self) -> &miaominal_services::RdpService {
        self.services
            .rdp_service
            .as_ref()
            .expect("RDP is available on desktop")
    }
    pub(super) fn subscribe_rdp(&self, cx: &mut Context<Self>) {
        let mut receiver = self.rdp_service().subscribe();
        cx.spawn(async move |owner, cx| {
            while receiver.changed().await.is_ok() {
                let snapshot = receiver.borrow_and_update().clone();
                if owner
                    .update(cx, |owner, cx| owner.apply_rdp_snapshot(&snapshot, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }
    pub(in crate::ui) fn open_rdp_launch(&self, profile_id: &str, cx: &mut Context<Self>) {
        let Some(profile) = self
            .profiles
            .borrow()
            .iter()
            .find(|p| p.id == profile_id && p.kind == miaominal_core::profile::ProfileKind::Ssh)
            .cloned()
        else {
            // E.g. quick-connect tabs whose profile is not saved in the catalog.
            cx.emit(AppCommand::Feedback(i18n::string(
                "rdp.profile_unavailable",
            )));
            return;
        };
        if self.rdp_confirmation.borrow().is_some() {
            // A pending close/disconnect confirmation must not be dropped silently.
            return;
        }
        // Opening any entry point creates the same transient defaults.
        *self.rdp_launch.borrow_mut() = Some(RdpLaunchDraft {
            name: profile.connection_label(),
            reuse_unavailable: reuse_unavailable_reason(&profile),
            host: profile.host.clone(),
            ssh_target: ssh_target(&profile),
            username: profile.username.clone(),
            profile_id: profile.id,
            mode: RdpConnectionMode::SshTunnel,
            reuse: false,
        });
        self.rdp_launch_revision
            .set(self.rdp_launch_revision.get().wrapping_add(1));
        cx.notify();
    }
    pub(in crate::ui::features::session) fn set_rdp_launch(
        &self,
        mode: Option<RdpConnectionMode>,
        reuse: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        if let Some(draft) = self.rdp_launch.borrow_mut().as_mut() {
            if let Some(mode) = mode {
                draft.mode = mode;
            }
            if let Some(reuse) = reuse {
                draft.reuse = reuse && draft.reuse_unavailable.is_none();
            }
        }
        cx.notify();
    }
    pub(in crate::ui::features::session) fn dismiss_rdp_launch(&self, cx: &mut Context<Self>) {
        self.rdp_launch.borrow_mut().take();
        cx.notify();
    }
    /// Window lifecycle preparation: the port field exists exactly while a launch panel
    /// is open, and every newly opened panel starts from the default port.
    pub(in crate::ui) fn sync_rdp_launch_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        let revision = self
            .rdp_launch
            .borrow()
            .as_ref()
            .map(|_| self.rdp_launch_revision.get());
        let current = self
            .rdp_port_input
            .borrow()
            .as_ref()
            .map(|(revision, _)| *revision);
        match revision {
            Some(revision) if current == Some(revision) => {}
            Some(revision) => {
                let default = RdpLaunchOptions::DEFAULT_PORT.to_string();
                let input = new_input_state(default.clone(), default, false, window, cx);
                *self.rdp_port_input.borrow_mut() = Some((revision, input));
                cx.notify();
            }
            None => {
                if self.rdp_port_input.borrow_mut().take().is_some() {
                    cx.notify();
                }
            }
        }
    }
    pub(in crate::ui::features::session) fn rdp_port_input(&self) -> Option<Entity<InputState>> {
        let revision = self.rdp_launch_revision.get();
        self.rdp_port_input
            .borrow()
            .as_ref()
            .filter(|(input_revision, _)| *input_revision == revision)
            .map(|(_, input)| input.clone())
    }
    /// Port entered in the open launch panel; the default when the field is absent or empty,
    /// `None` when the text is not a port.
    pub(in crate::ui::features::session) fn rdp_launch_port(&self, cx: &App) -> Option<u16> {
        match self.rdp_port_input() {
            Some(input) => parse_rdp_port(&input.read(cx).value()),
            None => Some(RdpLaunchOptions::DEFAULT_PORT),
        }
    }
    pub(in crate::ui::features::session) fn submit_rdp_launch(&self, cx: &mut Context<Self>) {
        let port = match self.rdp_launch_port(cx) {
            Some(port) => port,
            // Keep the panel open; it shows the field's error next to the field.
            None => return,
        };
        let Some(draft) = self.rdp_launch.borrow_mut().take() else {
            return;
        };
        let options = RdpLaunchOptions::new(
            draft.mode,
            miaominal_settings::current_settings().rdp_client.clone(),
        )
        .with_port(port)
        .with_reuse_ssh_password(draft.reuse && draft.reuse_unavailable.is_none());
        cx.emit(AppCommand::OpenTab(TabOpenRequest::RdpConnect {
            profile_id: draft.profile_id,
            options,
        }));
        cx.notify();
    }
    pub(in crate::ui) fn rdp_requires_unlock(
        &self,
        profile_id: &str,
        options: &RdpLaunchOptions,
    ) -> bool {
        if self.services.local_vault_status != LocalVaultStatus::Locked {
            return false;
        }
        let profiles = self.profiles.borrow();
        let Some(profile) = profiles.iter().find(|p| p.id == profile_id) else {
            return false;
        };
        if options.mode() == RdpConnectionMode::Direct {
            return options.reuses_ssh_password()
                && profile.effective_auth_method() == AuthMethod::Password
                && self.profile_requires_local_vault_unlock(profile);
        }
        let mut route = miaominal_ssh::connection::resolve_proxy_jump_profiles(profile, &profiles)
            .unwrap_or_default();
        route.push(profile.clone());
        route.iter().any(|p| {
            self.profile_requires_local_vault_unlock(p)
                || p.entry_proxy_id.as_ref().is_some_and(|id| {
                    self.proxies
                        .borrow()
                        .iter()
                        .any(|proxy| &proxy.id == id && proxy.has_stored_password)
                })
        })
    }
    pub(in crate::ui) fn start_rdp(
        &self,
        tab_id: TabId,
        profile_id: &str,
        options: RdpLaunchOptions,
        cx: &mut Context<Self>,
    ) -> Option<SessionTabDescriptor> {
        // Resolve the profile before launching so a missing profile cannot leave a
        // running session without a tab that could ever stop it.
        let Some(profile) = self
            .profiles
            .borrow()
            .iter()
            .find(|p| p.id == profile_id)
            .cloned()
        else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "rdp.profile_unavailable",
            )));
            return None;
        };
        let runtime = match self.rdp_service().launch(profile_id, options) {
            Ok(runtime) => runtime,
            Err(error) => {
                cx.emit(AppCommand::Feedback(error.to_string()));
                return None;
            }
        };
        if runtime.mode() == RdpConnectionMode::Direct {
            self.rdp_direct
                .borrow_mut()
                .insert(runtime.id().to_string());
            cx.emit(AppCommand::Feedback(i18n::string("rdp.starting")));
            return None;
        }
        let rule = PortForwardRule {
            id: runtime.id().into(),
            label: "RDP".into(),
            kind: PortForwardKind::Local,
            listen_host: runtime.local_host().into(),
            listen_port: 0,
            target_host: "127.0.0.1".into(),
            target_port: runtime.remote_port(),
            enabled: true,
        };
        let (_, prepared) = Self::build_port_forwarding_tab(tab_id, &profile, &rule, None);
        let mut state = prepared.into_payload::<SessionTabState>();
        state.purpose = SessionPurpose::Rdp;
        state.port_forward_rule_id = None;
        state.rdp = Some(runtime);
        self.insert_tab(tab_id, state);
        self.apply_rdp_snapshot(&self.rdp_service().snapshot(), cx);
        Some(SessionTabDescriptor::new(
            tab_id,
            format!("RDP · {}", profile.connection_label()),
            i18n::string("rdp.starting"),
            TabKindTag::Session,
            TabPlacement::TopLevel,
        ))
    }
    fn apply_rdp_snapshot(&self, snapshot: &RdpSnapshot, cx: &mut Context<Self>) {
        let mut updates = Vec::new();
        for (tab_id, tab) in self.tabs.borrow_mut().iter_mut() {
            let Some(previous) = tab.rdp.as_ref() else {
                continue;
            };
            let Some(runtime) = snapshot.session(previous.id()) else {
                continue;
            };
            if previous.revision() == runtime.revision() {
                continue;
            }
            tab.set_connection_state(match runtime.state() {
                RdpRuntimeState::Starting => SessionConnectionState::Connecting,
                RdpRuntimeState::WaitingForClient | RdpRuntimeState::ClientRunning => {
                    SessionConnectionState::Ready
                }
                RdpRuntimeState::Reconnecting => SessionConnectionState::Reconnecting {
                    error: i18n::string("rdp.reconnecting"),
                    attempt: 0,
                },
                RdpRuntimeState::Stopped => SessionConnectionState::Disconnected,
                RdpRuntimeState::Failed(error) => SessionConnectionState::Failed {
                    error: error.clone(),
                    status: Some(SessionFailureStatus::Closed),
                },
            });
            tab.pending_host_key = None;
            tab.pending_host_key_request_id = 0;
            tab.pending_keyboard_interactive = None;
            tab.pending_keyboard_interactive_request_id = 0;
            match runtime.prompt() {
                Some(PortForwardPrompt::HostKey(request)) => {
                    tab.pending_host_key = Some(request.prompt.clone());
                    tab.pending_host_key_request_id = request.request_id;
                }
                Some(PortForwardPrompt::KeyboardInteractive(request)) => {
                    tab.pending_keyboard_interactive = Some(request.challenge.clone());
                    tab.pending_keyboard_interactive_request_id = request.request_id;
                }
                None => {}
            }
            tab.rdp = Some(runtime.clone());
            updates.push((*tab_id, status(runtime)));
        }
        let changed = !updates.is_empty();
        for (tab_id, status) in updates {
            cx.emit(AppCommand::TabStatusChanged { tab_id, status });
        }
        let completed = self
            .rdp_direct
            .borrow()
            .iter()
            .filter_map(|id| {
                snapshot
                    .session(id)
                    .and_then(|runtime| match runtime.state() {
                        RdpRuntimeState::ClientRunning => {
                            Some((id.clone(), i18n::string("rdp.direct_started"), false))
                        }
                        RdpRuntimeState::Failed(error) => Some((id.clone(), error.clone(), true)),
                        _ => None,
                    })
            })
            .collect::<Vec<_>>();
        let completed_any = !completed.is_empty();
        for (id, message, failed) in completed {
            self.rdp_direct.borrow_mut().remove(&id);
            cx.emit(AppCommand::Feedback(message));
            if failed {
                self.rdp_service().forget(&id);
            }
        }
        // Publications for sessions owned by other windows or unchanged revisions
        // must not re-render every window.
        if changed || completed_any {
            cx.notify();
        }
    }
    pub(in crate::ui) fn rdp_action(&self, tab_id: TabId, action: &str, cx: &mut Context<Self>) {
        let Some(runtime) = self.tab(tab_id).and_then(|tab| tab.rdp.clone()) else {
            return;
        };
        if action == "restart"
            && self.rdp_requires_unlock(
                runtime.profile_id(),
                &RdpLaunchOptions::new(
                    RdpConnectionMode::SshTunnel,
                    miaominal_settings::current_settings().rdp_client.clone(),
                ),
            )
        {
            cx.emit(AppCommand::vault_unlock(
                crate::ui::contracts::commands::DeferredAppCommand::Session(
                    crate::ui::contracts::commands::SessionDeferredCommand::RestartRdp(tab_id),
                ),
            ));
            return;
        }
        if action == "stop" && self.rdp_has_connections(tab_id) {
            *self.rdp_confirmation.borrow_mut() = Some(RdpConfirmation::Disconnect(tab_id));
            self.bump_rdp_confirmation_revision();
            cx.notify();
            return;
        }
        let result = match action {
            "reopen" => self.rdp_service().relaunch_client(runtime.id()),
            "hold" => self
                .rdp_service()
                .set_keep_tunnel(runtime.id(), !runtime.keep_tunnel()),
            "stop" => {
                self.rdp_service().stop(runtime.id());
                Ok(())
            }
            "restart" => self
                .rdp_service()
                .restart(
                    runtime.id(),
                    miaominal_settings::current_settings().rdp_client.clone(),
                )
                .map(|_| ()),
            _ => return,
        };
        if let Err(error) = result {
            cx.emit(AppCommand::Feedback(error.to_string()));
        }
        cx.notify();
    }
    pub(in crate::ui::features::session) fn rdp_launch_overlay(
        &self,
        entity: Entity<Self>,
    ) -> Option<OverlaySnapshot> {
        self.rdp_launch.borrow().as_ref().map(|_| {
            super::super::view::rdp::launch_overlay(entity, self.rdp_launch_revision.get())
        })
    }
    pub(in crate::ui::features::session) fn rdp_launch_draft(&self) -> Option<RdpLaunchDraft> {
        self.rdp_launch.borrow().clone()
    }
    pub(in crate::ui::features::session) fn is_rdp_launch_current(&self, revision: u64) -> bool {
        self.rdp_launch.borrow().is_some() && self.rdp_launch_revision.get() == revision
    }
    fn rdp_has_connections(&self, tab_id: TabId) -> bool {
        self.tab(tab_id)
            .and_then(|tab| tab.rdp.as_ref().map(|runtime| runtime.id().to_string()))
            .and_then(|id| self.rdp_service().snapshot().session(&id).cloned())
            .is_some_and(|runtime| runtime.active_connections() > 0)
    }
    pub(in crate::ui) fn request_rdp_close(&self, tabs: &[TabId], cx: &mut Context<Self>) -> bool {
        let requires = tabs.iter().any(|id| {
            !self.rdp_close_approved.borrow().contains(id) && self.rdp_has_connections(*id)
        });
        if !requires {
            for id in tabs {
                self.rdp_close_approved.borrow_mut().remove(id);
            }
            return false;
        }
        *self.rdp_confirmation.borrow_mut() = Some(RdpConfirmation::Close(tabs.to_vec()));
        self.bump_rdp_confirmation_revision();
        cx.notify();
        true
    }
    fn bump_rdp_confirmation_revision(&self) {
        self.rdp_confirmation_revision
            .set(self.rdp_confirmation_revision.get().wrapping_add(1));
    }
    pub(in crate::ui::features::session) fn finish_rdp_confirmation(
        &self,
        confirmed: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = self.rdp_confirmation.borrow_mut().take() else {
            return;
        };
        if confirmed {
            match action {
                RdpConfirmation::Close(tabs) => {
                    self.rdp_close_approved
                        .borrow_mut()
                        .extend(tabs.iter().copied());
                    for tab in tabs {
                        cx.emit(AppCommand::CloseTab(tab));
                    }
                }
                RdpConfirmation::Disconnect(tab) => {
                    if let Some(runtime) = self.tab(tab).and_then(|tab| tab.rdp.clone()) {
                        self.rdp_service().stop(runtime.id());
                    }
                }
            }
        }
        cx.notify();
    }
    pub(in crate::ui::features::session) fn is_rdp_confirmation_current(
        &self,
        revision: u64,
    ) -> bool {
        self.rdp_confirmation.borrow().is_some() && self.rdp_confirmation_revision.get() == revision
    }
    pub(in crate::ui::features::session) fn rdp_confirmation_overlay(
        &self,
        entity: Entity<Self>,
    ) -> Option<OverlaySnapshot> {
        self.rdp_confirmation.borrow().as_ref().map(|_| {
            super::super::view::rdp::confirmation_overlay(
                entity,
                self.rdp_confirmation_revision.get(),
            )
        })
    }
}
