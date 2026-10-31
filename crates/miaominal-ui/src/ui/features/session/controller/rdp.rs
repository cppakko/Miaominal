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

    /// Launches through a real service with an unavailable client: the session must be
    /// listed immediately and its failure must stay visible with Reconnect / Remove.
    #[gpui_kit::test]
    fn launch_is_listed_and_failure_stays_visible(cx: &mut gpui_kit::TestAppContext) {
        cx.update(gpui_kit::init);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let service = miaominal_services::RdpService::new(
            runtime.handle().clone(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(directory.path().join("known_hosts")),
        );
        let mut profile = SessionProfile::blank("host", 1);
        profile.host = "127.0.0.1".into();
        service.replace_catalogs(vec![profile.clone()], Vec::new());
        let owner = cx.update(|cx| {
            cx.new(|cx| {
                let mut owner = SessionController::new_for_test_with_profiles(vec![profile]);
                owner.services.rdp_service = Some(service);
                owner.subscribe_rdp(cx);
                owner
            })
        });
        let client = miaominal_settings::RdpClientSettings::new(
            if cfg!(target_os = "macos") {
                miaominal_settings::RdpClientKind::WindowsApp
            } else if cfg!(windows) {
                miaominal_settings::RdpClientKind::Mstsc
            } else {
                miaominal_settings::RdpClientKind::Remmina
            },
            directory
                .path()
                .join("missing-rdp-client")
                .to_string_lossy()
                .into_owned(),
        );
        owner.update(cx, |owner, cx| {
            owner.start_rdp(
                TabId::new(900),
                "host",
                RdpLaunchOptions::new(RdpConnectionMode::Direct, client),
                cx,
            );
            assert_eq!(owner.rdp_sessions().len(), 1, "a launch is listed at once");
        });
        let mut failed = false;
        for _ in 0..200 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            cx.run_until_parked();
            failed = owner.read_with(cx, |owner, _| {
                owner
                    .rdp_sessions()
                    .first()
                    .is_some_and(|runtime| matches!(runtime.state(), RdpRuntimeState::Failed(_)))
            });
            if failed {
                break;
            }
        }
        assert!(failed, "a failed launch stays listed with its error");
        // The status-center section must lay its rows out; a collapsed list once showed only
        // the header.
        struct PanelFixture(Entity<SessionController>);
        impl Render for PanelFixture {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div().children(self.0.read(cx).render_rdp_status_section(self.0.clone()))
            }
        }
        let (handle, _) = cx.update(|cx| {
            let panel_owner = owner.clone();
            gpui_kit::open_window(WindowOptions::default(), cx, move |_, cx| {
                cx.new(|_| PanelFixture(panel_owner))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let id = owner.read(cx).rdp_sessions()[0].id().to_string();
            let reconnect = window
                .find(gpui_kit::SharedString::from(format!("rdp-restart-{id}")))
                .bounds();
            let panel = window.find("rdp-panel").bounds();
            // Rows keep their own size while a collapsed list lets them spill past the panel.
            assert!(
                reconnect.size.height > px(0.0) && reconnect.bottom() <= panel.bottom(),
                "session rows are laid out inside the panel"
            );
            assert!(
                window.try_find("rdp-clear-ended").is_some(),
                "an ended session offers clearing"
            );
        })
        .unwrap();
        owner.update(cx, |owner, cx| {
            assert!(owner.rdp_panel_open(), "a failure opens the status center");
            let id = owner.rdp_sessions()[0].id().to_string();
            assert!(owner.is_rdp_log_open(&id), "a failure expands its log");
            owner.clear_ended_rdp(cx);
            assert!(owner.rdp_sessions().is_empty());
            assert!(
                owner.render_rdp_status_section(cx.entity()).is_none(),
                "without sessions the center shows notifications only"
            );
        });
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
        let mut password = SessionProfile::blank("password", 3);
        password.auth_method = Some(AuthMethod::Password);
        password.password = "secret".into();
        let owner = cx.update(|cx| {
            cx.new(|_| {
                SessionController::new_for_test_with_profiles(vec![
                    key_profile,
                    no_password,
                    password,
                ])
            })
        });
        owner.update(cx, |owner, cx| {
            // Preselected whenever it can apply. Whether it can also depends on the
            // configured client (mstsc / SDL FreeRDP accept passwords), so only the
            // relation is fixed here.
            owner.open_rdp_launch("password", cx);
            let draft = owner.rdp_launch_draft().unwrap();
            assert_eq!(draft.reuse, draft.reuse_unavailable.is_none());
            owner.set_rdp_launch(None, Some(false), cx);
            owner.open_rdp_launch("password", cx);
            assert_eq!(
                owner.rdp_launch_draft().unwrap().reuse,
                draft.reuse_unavailable.is_none(),
                "reopening restores the default"
            );
            owner.dismiss_rdp_launch(cx);
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
            *owner.rdp_confirmation.borrow_mut() =
                Some(RdpConfirmation::Disconnect("pending".into()));
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
    /// Closing this window closes the listed tabs and ends every remote desktop it launched.
    CloseWindow(Vec<TabId>),
    Disconnect(String),
}

/// Commands offered for one remote desktop in the footer panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum RdpAction {
    Reopen,
    ToggleHold,
    Disconnect,
    Reconnect,
    Remove,
}

/// A remote desktop launched from this window, kept in launch order. Remote desktops do not
/// open tabs; the footer panel lists them. A tunnelled session also keeps a controller-only
/// session payload under `prompt_tab`. That payload is never registered in the workspace; it
/// exists so SSH host-key and keyboard-interactive prompts reuse the session overlays.
#[derive(Clone, Debug)]
pub(in crate::ui::features::session) struct OwnedRdpSession {
    id: String,
    prompt_tab: Option<TabId>,
}

pub(in crate::ui::features::session) fn status(runtime: &RdpSessionSnapshot) -> String {
    match runtime.state() {
        // A tunnel waits on SSH before any client starts; say what it is waiting for.
        _ if matches!(runtime.prompt(), Some(PortForwardPrompt::HostKey(_))) => {
            i18n::string("rdp.awaiting_host_key")
        }
        _ if matches!(
            runtime.prompt(),
            Some(PortForwardPrompt::KeyboardInteractive(_))
        ) =>
        {
            i18n::string("rdp.awaiting_auth")
        }
        RdpRuntimeState::Starting if runtime.mode() == RdpConnectionMode::SshTunnel => {
            i18n::string("rdp.connecting_tunnel")
        }
        RdpRuntimeState::Starting => i18n::string("rdp.starting"),
        // msrdc-style clients pass the launch to a running instance that holds it until its
        // own pending connection or dialog is finished, so "waiting" would be misleading.
        RdpRuntimeState::WaitingForClient | RdpRuntimeState::ClientRunning
            if runtime.client_handed_off() && runtime.active_connections() == 0 =>
        {
            i18n::string("rdp.handed_off_short")
        }
        RdpRuntimeState::WaitingForClient => i18n::string("rdp.client_started"),
        RdpRuntimeState::ClientRunning if runtime.mode() == RdpConnectionMode::Direct => {
            i18n::string("rdp.direct_running")
        }
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
        // Opening any entry point creates the same transient defaults. Reusing the SSH
        // password is preselected whenever it can apply; otherwise it stays off and disabled.
        let reuse_unavailable = reuse_unavailable_reason(&profile);
        *self.rdp_launch.borrow_mut() = Some(RdpLaunchDraft {
            name: profile.connection_label(),
            reuse: reuse_unavailable.is_none(),
            reuse_unavailable,
            host: profile.host.clone(),
            ssh_target: ssh_target(&profile),
            username: profile.username.clone(),
            profile_id: profile.id,
            mode: RdpConnectionMode::SshTunnel,
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
    /// Launches a remote desktop owned by this window. No tab opens: the session appears in
    /// the footer's remote desktop panel until it ends or is removed. `prompt_tab` is a
    /// workspace-allocated id reserved for the session's SSH prompts.
    pub(in crate::ui) fn start_rdp(
        &self,
        prompt_tab: TabId,
        profile_id: &str,
        options: RdpLaunchOptions,
        cx: &mut Context<Self>,
    ) {
        // Resolve the profile before launching so a missing profile cannot leave a
        // running session that nothing lists or stops.
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
            return;
        };
        // One tunnel per profile and remote port: another launch opens a client on it
        // instead of a second SSH connection that would only compete for the same session.
        if let Some(existing) = self.reusable_rdp_tunnel(profile_id, &options) {
            self.reuse_rdp_tunnel(&existing, options, cx);
            return;
        }
        let runtime = match self.rdp_service().launch(profile_id, options) {
            Ok(runtime) => runtime,
            Err(error) => {
                // Launched from any page, so a footer status message alone could go unseen.
                self.notify_rdp_failure(
                    &format!("launch-{profile_id}"),
                    &profile.connection_label(),
                    &error.to_string(),
                    cx,
                );
                return;
            }
        };
        let prompt_tab = (runtime.mode() == RdpConnectionMode::SshTunnel).then(|| {
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
            let (_, prepared) = Self::build_port_forwarding_tab(prompt_tab, &profile, &rule, None);
            let mut state = prepared.into_payload::<SessionTabState>();
            state.purpose = SessionPurpose::Rdp;
            state.port_forward_rule_id = None;
            state.rdp = Some(runtime.clone());
            self.insert_tab(prompt_tab, state);
            prompt_tab
        });
        self.rdp_sessions.borrow_mut().push(OwnedRdpSession {
            id: runtime.id().to_string(),
            prompt_tab,
        });
        cx.emit(AppCommand::Feedback(i18n::string_args(
            "rdp.launched",
            &[("name", &profile.connection_label())],
        )));
        // The launch dialog closes here; the panel takes over showing progress.
        self.rdp_panel_open.set(true);
        self.apply_rdp_snapshot(&self.rdp_service().snapshot(), cx);
        cx.notify();
    }
    /// An active tunnel this window launched for the same profile and remote port.
    fn reusable_rdp_tunnel(
        &self,
        profile_id: &str,
        options: &RdpLaunchOptions,
    ) -> Option<RdpSessionSnapshot> {
        if options.mode() != RdpConnectionMode::SshTunnel {
            return None;
        }
        let snapshot = self.rdp_service().snapshot();
        self.rdp_sessions.borrow().iter().find_map(|session| {
            snapshot
                .session(&session.id)
                .filter(|runtime| {
                    runtime.profile_id() == profile_id
                        && runtime.mode() == RdpConnectionMode::SshTunnel
                        && runtime.remote_port() == options.port()
                        && runtime.state().is_active()
                })
                .cloned()
        })
    }
    fn reuse_rdp_tunnel(
        &self,
        existing: &RdpSessionSnapshot,
        options: RdpLaunchOptions,
        cx: &mut Context<Self>,
    ) {
        self.rdp_panel_open.set(true);
        let name = existing.profile_name().to_string();
        match self
            .rdp_service()
            .relaunch_client_with(existing.id(), options)
        {
            // A tunnel still connecting opens its client once it listens.
            Ok(()) => cx.emit(AppCommand::Feedback(i18n::string_args(
                if existing.local_port().is_some() {
                    "rdp.reused"
                } else {
                    "rdp.reuse_pending"
                },
                &[("name", &name)],
            ))),
            Err(error) => self.notify_rdp_failure(
                &format!("reuse-{}", existing.id()),
                &name,
                &error.to_string(),
                cx,
            ),
        }
        cx.notify();
    }
    /// A failure is announced as a notification and opens the panel with its log, wherever
    /// the user currently is.
    fn notify_rdp_failure(&self, key: &str, name: &str, error: &str, cx: &mut Context<Self>) {
        self.rdp_panel_open.set(true);
        cx.emit(AppCommand::SessionNotificationRequested(
            SessionNotificationRequest {
                tone: SessionNotificationTone::Error,
                title: i18n::string("rdp.failed_title"),
                message: i18n::string_args("rdp.failed", &[("name", name), ("error", error)]),
                id: format!("rdp-failed-{key}"),
            },
        ));
        cx.emit(AppCommand::Feedback(i18n::string_args(
            "rdp.failed",
            &[("name", name), ("error", error)],
        )));
    }
    pub(in crate::ui) fn rdp_panel_open(&self) -> bool {
        self.rdp_panel_open.get()
    }
    pub(in crate::ui) fn set_rdp_panel_open(&self, open: bool, cx: &mut Context<Self>) {
        if self.rdp_panel_open.replace(open) != open {
            cx.notify();
        }
    }
    /// Forgets every listed session that has stopped or failed; running ones stay.
    pub(in crate::ui::features::session) fn clear_ended_rdp(&self, cx: &mut Context<Self>) {
        let snapshot = self.rdp_service().snapshot();
        let ended = self
            .rdp_sessions
            .borrow()
            .iter()
            .filter(|session| {
                snapshot
                    .session(&session.id)
                    .is_none_or(|runtime| !runtime.state().is_active())
            })
            .map(|session| session.id.clone())
            .collect::<Vec<_>>();
        if ended.is_empty() {
            return;
        }
        for id in ended {
            self.rdp_service().forget(&id);
            self.release_owned_rdp(&id);
        }
        cx.emit(AppCommand::RdpSessionsChanged);
        cx.notify();
    }
    fn apply_rdp_snapshot(&self, snapshot: &RdpSnapshot, cx: &mut Context<Self>) {
        let previous = self.rdp_snapshot.replace(snapshot.clone());
        // Prompt payloads mirror their runtime so the authentication overlays see requests.
        for tab in self.tabs.borrow_mut().values_mut() {
            let Some(current) = tab.rdp.as_ref() else {
                continue;
            };
            let Some(runtime) = snapshot.session(current.id()) else {
                continue;
            };
            if current.revision() == runtime.revision() {
                continue;
            }
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
        }
        let owned = self.rdp_sessions.borrow().clone();
        let mut changed = false;
        let mut messages = Vec::new();
        let mut failures = Vec::new();
        let mut ended = Vec::new();
        for session in owned {
            let before = previous.session(&session.id);
            let Some(now) = snapshot.session(&session.id) else {
                // Direct launches leave the service once the client is gone; forgotten
                // sessions leave it immediately.
                ended.push(session.id);
                continue;
            };
            if before.is_some_and(|before| before.revision() == now.revision()) {
                continue;
            }
            changed = true;
            let was = before.map(RdpSessionSnapshot::state);
            if now.client_handed_off() && !before.is_some_and(|b| b.client_handed_off()) {
                // Nothing may appear on screen; show why where the user is looking.
                self.rdp_panel_open.set(true);
                self.rdp_logs_open.borrow_mut().insert(now.id().to_string());
                messages.push(i18n::string("rdp.handed_off_short"));
            }
            match now.state() {
                RdpRuntimeState::ClientRunning
                    if now.mode() == RdpConnectionMode::Direct
                        && was != Some(&RdpRuntimeState::ClientRunning) =>
                {
                    messages.push(i18n::string_args(
                        "rdp.direct_started",
                        &[("name", now.profile_name())],
                    ));
                }
                RdpRuntimeState::Failed(error)
                    if !matches!(was, Some(RdpRuntimeState::Failed(_))) =>
                {
                    failures.push((
                        format!("{}-{}", now.id(), now.revision()),
                        now.id().to_string(),
                        now.profile_name().to_string(),
                        error.clone(),
                    ));
                }
                _ => {}
            }
        }
        changed |= !ended.is_empty();
        for id in ended {
            self.release_owned_rdp(&id);
        }
        for message in messages {
            cx.emit(AppCommand::Feedback(message));
        }
        for (key, id, name, error) in failures {
            self.rdp_logs_open.borrow_mut().insert(id);
            self.notify_rdp_failure(&key, &name, &error, cx);
        }
        // Publications for sessions owned by other windows or unchanged revisions
        // must not re-render every window.
        if changed {
            cx.emit(AppCommand::RdpSessionsChanged);
            cx.notify();
        }
    }
    /// Drops this window's record of a session and its prompt payload. The service entry
    /// is left alone; callers stop or forget it first when needed.
    fn release_owned_rdp(&self, id: &str) {
        let removed = {
            let mut sessions = self.rdp_sessions.borrow_mut();
            let index = sessions.iter().position(|session| session.id == id);
            index.map(|index| sessions.remove(index))
        };
        self.rdp_logs_open.borrow_mut().remove(id);
        if let Some(tab) = removed.and_then(|session| session.prompt_tab) {
            self.remove_tab(tab);
        }
    }
    /// Remote desktops launched from this window, in launch order.
    pub(in crate::ui) fn rdp_sessions(&self) -> Vec<RdpSessionSnapshot> {
        let snapshot = self.rdp_snapshot.borrow();
        self.rdp_sessions
            .borrow()
            .iter()
            .filter_map(|session| snapshot.session(&session.id).cloned())
            .collect()
    }
    /// Payload ids that may carry SSH prompts for this window's remote desktops.
    pub(super) fn rdp_prompt_tabs(&self) -> Vec<TabId> {
        self.rdp_sessions
            .borrow()
            .iter()
            .filter_map(|session| session.prompt_tab)
            .collect()
    }
    pub(in crate::ui::features::session) fn is_rdp_log_open(&self, id: &str) -> bool {
        self.rdp_logs_open.borrow().contains(id)
    }
    pub(in crate::ui::features::session) fn toggle_rdp_log(
        &self,
        id: &str,
        cx: &mut Context<Self>,
    ) {
        let mut open = self.rdp_logs_open.borrow_mut();
        if !open.remove(id) {
            open.insert(id.to_string());
        }
        drop(open);
        cx.notify();
    }
    pub(in crate::ui) fn rdp_action(&self, id: &str, action: RdpAction, cx: &mut Context<Self>) {
        if !self
            .rdp_sessions
            .borrow()
            .iter()
            .any(|session| session.id == id)
        {
            return;
        }
        let Some(runtime) = self.rdp_service().snapshot().session(id).cloned() else {
            return;
        };
        if action == RdpAction::Reconnect
            && self.rdp_requires_unlock(
                runtime.profile_id(),
                &RdpLaunchOptions::new(
                    runtime.mode(),
                    miaominal_settings::current_settings().rdp_client.clone(),
                ),
            )
        {
            cx.emit(AppCommand::vault_unlock(
                crate::ui::contracts::commands::DeferredAppCommand::Session(
                    crate::ui::contracts::commands::SessionDeferredCommand::RestartRdp(
                        id.to_string(),
                    ),
                ),
            ));
            return;
        }
        if action == RdpAction::Disconnect && runtime.active_connections() > 0 {
            *self.rdp_confirmation.borrow_mut() = Some(RdpConfirmation::Disconnect(id.to_string()));
            self.bump_rdp_confirmation_revision();
            cx.notify();
            return;
        }
        let result = match action {
            RdpAction::Reopen => self.rdp_service().relaunch_client(id),
            RdpAction::ToggleHold => self
                .rdp_service()
                .set_keep_tunnel(id, !runtime.keep_tunnel()),
            RdpAction::Disconnect => {
                self.rdp_service().stop(id);
                Ok(())
            }
            RdpAction::Reconnect => self
                .rdp_service()
                .restart(
                    id,
                    miaominal_settings::current_settings().rdp_client.clone(),
                )
                .map(|_| ()),
            RdpAction::Remove => {
                self.rdp_service().forget(id);
                self.release_owned_rdp(id);
                cx.emit(AppCommand::RdpSessionsChanged);
                Ok(())
            }
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
    fn rdp_has_connections(&self) -> bool {
        let snapshot = self.rdp_service().snapshot();
        self.rdp_sessions.borrow().iter().any(|session| {
            snapshot
                .session(&session.id)
                .is_some_and(|runtime| runtime.active_connections() > 0)
        })
    }
    /// Closing a detached window ends the remote desktops it launched. Tunnels that still
    /// carry connections ask first. Returns `true` when the close must wait for that answer;
    /// confirming closes `tabs`, which closes the window.
    pub(in crate::ui) fn request_rdp_window_close(
        &self,
        tabs: Vec<TabId>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.rdp_window_close_approved.get() || !self.rdp_has_connections() {
            return false;
        }
        *self.rdp_confirmation.borrow_mut() = Some(RdpConfirmation::CloseWindow(tabs));
        self.bump_rdp_confirmation_revision();
        cx.notify();
        true
    }
    /// Window teardown: ends every remote desktop this window launched.
    pub(in crate::ui) fn retire_rdp_sessions(&self) {
        let ids = self
            .rdp_sessions
            .borrow()
            .iter()
            .map(|session| session.id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            self.rdp_service().forget(&id);
            self.release_owned_rdp(&id);
        }
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
                RdpConfirmation::CloseWindow(tabs) => {
                    self.rdp_window_close_approved.set(true);
                    for tab in tabs {
                        cx.emit(AppCommand::CloseTab(tab));
                    }
                }
                RdpConfirmation::Disconnect(id) => {
                    self.rdp_service().stop(&id);
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
