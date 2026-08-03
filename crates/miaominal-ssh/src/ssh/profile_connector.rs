use super::auth::{
    authenticate_bridge, authenticate_full, hydrate_profile_from_secrets,
    validate_bridge_auth_material,
};
use super::forwarding::RemoteForwardTargets;
use super::session::{
    self, ClientHandler, ConnectedClient, SessionCommand, SessionEventSender,
    build_strict_client_handler, connection,
};
use anyhow::{Context, Result};
use miaominal_core::profile::{PortForwardRule, SessionProfile};
use miaominal_core::proxy::ProxyProfile;
use miaominal_secrets::SecretStore;
use miaominal_storage::KnownHostsStore;
use russh::{Channel, Disconnect, client};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc::UnboundedReceiver;

/// Builds a complete authenticated SSH route from a persisted Miaominal profile.
///
/// The same route construction is used by interactive terminal sessions and the
/// non-interactive SSH Bridge. Only host-key prompting and keyboard-interactive
/// behavior differ between the two interaction policies.
#[derive(Clone)]
pub struct ProfileConnector {
    all_profiles: Arc<[SessionProfile]>,
    all_proxies: Arc<[ProxyProfile]>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
}

impl ProfileConnector {
    pub fn new(
        all_profiles: Vec<SessionProfile>,
        all_proxies: Vec<ProxyProfile>,
        secrets: SecretStore,
        known_hosts: KnownHostsStore,
    ) -> Self {
        Self {
            all_profiles: all_profiles.into(),
            all_proxies: all_proxies.into(),
            secrets,
            known_hosts,
        }
    }

    /// Connects using the strict, non-interactive SSH Bridge policy.
    pub async fn connect_bridge(&self, profile: SessionProfile) -> Result<ConnectedSshRoute> {
        self.validate_bridge_route_credentials(&profile)?;
        let mut interaction = ConnectorInteraction::Bridge;
        self.connect(profile, &mut interaction).await
    }

    fn validate_bridge_route_credentials(&self, profile: &SessionProfile) -> Result<()> {
        let target = hydrate_profile_from_secrets(profile.clone(), &self.secrets);
        validate_bridge_auth_material(&target, &self.secrets).with_context(|| {
            format!("credentials unavailable for {}", target.connection_label())
        })?;
        for jump in connection::resolve_proxy_jump_profiles(profile, &self.all_profiles)? {
            let jump = hydrate_profile_from_secrets(jump, &self.secrets);
            validate_bridge_auth_material(&jump, &self.secrets).with_context(|| {
                format!(
                    "credentials unavailable for jump host {}",
                    jump.connection_label()
                )
            })?;
        }
        Ok(())
    }

    pub(super) async fn connect_interactive(
        &self,
        profile: SessionProfile,
        command_receiver: &mut UnboundedReceiver<SessionCommand>,
        event_sender: &SessionEventSender,
    ) -> Result<ConnectedSshRoute> {
        let mut configured_port_forward_rules = profile.port_forwarding_rules.clone();
        let mut interaction = ConnectorInteraction::Interactive {
            command_receiver,
            event_sender,
            configured_port_forward_rules: &mut configured_port_forward_rules,
        };
        let mut route = self.connect(profile, &mut interaction).await?;
        route.configured_port_forward_rules = configured_port_forward_rules;
        Ok(route)
    }

    async fn connect(
        &self,
        profile: SessionProfile,
        interaction: &mut ConnectorInteraction<'_>,
    ) -> Result<ConnectedSshRoute> {
        let profile = hydrate_profile_from_secrets(profile, &self.secrets);
        let remote_label = profile.connection_label();
        let entry_proxy = crate::transport::resolve_entry_proxy(
            profile.entry_proxy_id.as_deref(),
            &self.all_proxies,
        )?
        .cloned();
        let proxy_jump_profiles =
            connection::resolve_proxy_jump_profiles(&profile, &self.all_profiles)?
                .into_iter()
                .map(|profile| hydrate_profile_from_secrets(profile, &self.secrets))
                .collect::<Vec<_>>();
        let config = connection::default_client_config();
        let mut jump_sessions = Vec::new();

        let (session, remote_forward_targets) = if let Some(first_hop) = proxy_jump_profiles.first()
        {
            interaction
                .status(format!(
                    "Connecting to jump host 1/{}: {}",
                    proxy_jump_profiles.len(),
                    first_hop.connection_label()
                ))
                .await?;

            let ConnectedClient {
                handle: mut current_session,
                remote_forward_targets: mut current_remote_forward_targets,
            } = interaction
                .connect_profile_with_optional_proxy(
                    first_hop,
                    entry_proxy.as_ref(),
                    &self.secrets,
                    config.clone(),
                    self.known_hosts.clone(),
                )
                .await?;
            interaction
                .status(format!(
                    "Authenticating jump host 1/{}",
                    proxy_jump_profiles.len()
                ))
                .await?;
            interaction
                .authenticate(&mut current_session, first_hop.clone(), &self.secrets)
                .await?;
            let mut current_session = Arc::new(current_session);

            let mut remaining_chain: Vec<_> = proxy_jump_profiles.iter().skip(1).cloned().collect();
            remaining_chain.push(profile);
            let total_hops = proxy_jump_profiles.len();

            for (index, next_profile) in remaining_chain.into_iter().enumerate() {
                let is_target = index + 1 == total_hops;
                interaction
                    .status(if is_target {
                        format!("Connecting to {remote_label} through ProxyJump")
                    } else {
                        format!(
                            "Connecting to jump host {}/{}: {}",
                            index + 2,
                            total_hops,
                            next_profile.connection_label()
                        )
                    })
                    .await?;

                let transport = current_session
                    .channel_open_direct_tcpip(
                        next_profile.host.clone(),
                        u32::from(next_profile.port),
                        "127.0.0.1".to_string(),
                        0,
                    )
                    .await
                    .with_context(|| {
                        format!(
                            "failed to open ProxyJump channel to {}:{}",
                            next_profile.host, next_profile.port
                        )
                    })?
                    .into_stream();

                let ConnectedClient {
                    handle: mut next_session,
                    remote_forward_targets: next_remote_forward_targets,
                } = interaction
                    .connect_profile_stream(
                        &next_profile,
                        transport,
                        config.clone(),
                        self.known_hosts.clone(),
                    )
                    .await?;
                interaction
                    .status(if is_target {
                        format!("Authenticating {remote_label}")
                    } else {
                        format!("Authenticating jump host {}/{}", index + 2, total_hops)
                    })
                    .await?;
                interaction
                    .authenticate(&mut next_session, next_profile, &self.secrets)
                    .await?;

                jump_sessions.push(current_session);
                current_session = Arc::new(next_session);
                current_remote_forward_targets = next_remote_forward_targets;
            }

            (current_session, current_remote_forward_targets)
        } else {
            interaction
                .status(format!("Connecting to {remote_label}"))
                .await?;
            let ConnectedClient {
                handle: mut session,
                remote_forward_targets,
            } = interaction
                .connect_profile_with_optional_proxy(
                    &profile,
                    entry_proxy.as_ref(),
                    &self.secrets,
                    config,
                    self.known_hosts.clone(),
                )
                .await?;
            interaction
                .status(format!("Authenticating {remote_label}"))
                .await?;
            interaction
                .authenticate(&mut session, profile, &self.secrets)
                .await?;
            (Arc::new(session), remote_forward_targets)
        };

        Ok(ConnectedSshRoute::new(
            session,
            Vec::new(),
            remote_forward_targets,
            jump_sessions,
        ))
    }
}

enum ConnectorInteraction<'a> {
    Interactive {
        command_receiver: &'a mut UnboundedReceiver<SessionCommand>,
        event_sender: &'a SessionEventSender,
        configured_port_forward_rules: &'a mut Vec<PortForwardRule>,
    },
    Bridge,
}

impl ConnectorInteraction<'_> {
    async fn status(&self, message: String) -> Result<()> {
        match self {
            Self::Interactive { event_sender, .. } => {
                session::emit_status(event_sender, message).await
            }
            Self::Bridge => Ok(()),
        }
    }

    async fn authenticate(
        &mut self,
        session: &mut client::Handle<ClientHandler>,
        profile: SessionProfile,
        secrets: &SecretStore,
    ) -> Result<()> {
        match self {
            Self::Interactive {
                command_receiver,
                event_sender,
                ..
            } => authenticate_full(session, profile, secrets, command_receiver, event_sender).await,
            Self::Bridge => authenticate_bridge(session, profile, secrets).await,
        }
    }

    async fn connect_profile_with_optional_proxy(
        &mut self,
        profile: &SessionProfile,
        proxy: Option<&ProxyProfile>,
        secrets: &SecretStore,
        config: Arc<client::Config>,
        known_hosts: KnownHostsStore,
    ) -> Result<ConnectedClient> {
        match self {
            Self::Interactive {
                command_receiver,
                event_sender,
                configured_port_forward_rules,
            } => {
                session::connect_profile_with_optional_proxy(
                    profile,
                    proxy,
                    secrets,
                    config,
                    known_hosts,
                    command_receiver,
                    event_sender,
                    configured_port_forward_rules,
                )
                .await
            }
            Self::Bridge => {
                let (handler, remote_forward_targets) =
                    build_strict_client_handler(profile, known_hosts);
                let handle = if let Some(proxy) = proxy {
                    let transport = crate::transport::connect_via_proxy(
                        proxy,
                        &profile.host,
                        profile.port,
                        secrets,
                    )
                    .await?;
                    connection::connect_profile_stream(profile, transport, config, handler).await?
                } else {
                    connection::connect_profile_session(profile, config, handler).await?
                };
                Ok(ConnectedClient {
                    handle,
                    remote_forward_targets,
                })
            }
        }
    }

    async fn connect_profile_stream<R>(
        &mut self,
        profile: &SessionProfile,
        transport: R,
        config: Arc<client::Config>,
        known_hosts: KnownHostsStore,
    ) -> Result<ConnectedClient>
    where
        R: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        match self {
            Self::Interactive {
                command_receiver,
                event_sender,
                configured_port_forward_rules,
            } => {
                session::connect_profile_stream(
                    profile,
                    transport,
                    config,
                    known_hosts,
                    command_receiver,
                    event_sender,
                    configured_port_forward_rules,
                )
                .await
            }
            Self::Bridge => {
                let (handler, remote_forward_targets) =
                    build_strict_client_handler(profile, known_hosts);
                let handle =
                    connection::connect_profile_stream(profile, transport, config, handler).await?;
                Ok(ConnectedClient {
                    handle,
                    remote_forward_targets,
                })
            }
        }
    }
}

/// Owns the authenticated target and every jump session in its route.
/// Dropping the route schedules an orderly best-effort disconnect of the full chain.
pub struct ConnectedSshRoute {
    pub(super) session: Arc<client::Handle<ClientHandler>>,
    pub(super) configured_port_forward_rules: Vec<PortForwardRule>,
    pub(super) remote_forward_targets: RemoteForwardTargets,
    pub(super) jump_sessions: Vec<Arc<client::Handle<ClientHandler>>>,
    pub(super) cleanup: RouteCleanup,
}

impl ConnectedSshRoute {
    fn new(
        session: Arc<client::Handle<ClientHandler>>,
        configured_port_forward_rules: Vec<PortForwardRule>,
        remote_forward_targets: RemoteForwardTargets,
        jump_sessions: Vec<Arc<client::Handle<ClientHandler>>>,
    ) -> Self {
        let cleanup = RouteCleanup::new(session.clone(), jump_sessions.clone());
        Self {
            session,
            configured_port_forward_rules,
            remote_forward_targets,
            jump_sessions,
            cleanup,
        }
    }

    pub async fn open_session(&self) -> Result<Channel<client::Msg>> {
        self.session
            .channel_open_session()
            .await
            .context("failed to open upstream SSH session channel")
    }

    pub async fn open_direct_tcpip(
        &self,
        host: String,
        port: u32,
        originator_host: String,
        originator_port: u32,
    ) -> Result<Channel<client::Msg>> {
        self.session
            .channel_open_direct_tcpip(host, port, originator_host, originator_port)
            .await
            .context("failed to open upstream direct-tcpip channel")
    }

    pub fn jump_count(&self) -> usize {
        self.jump_sessions.len()
    }

    pub async fn disconnect(&self) {
        self.cleanup.disconnect().await;
    }
}

pub(super) struct RouteCleanup {
    target: Arc<client::Handle<ClientHandler>>,
    jumps: Vec<Arc<client::Handle<ClientHandler>>>,
    disconnected: Arc<AtomicBool>,
    runtime: Option<tokio::runtime::Handle>,
}

impl RouteCleanup {
    fn new(
        target: Arc<client::Handle<ClientHandler>>,
        jumps: Vec<Arc<client::Handle<ClientHandler>>>,
    ) -> Self {
        Self {
            target,
            jumps,
            disconnected: Arc::new(AtomicBool::new(false)),
            runtime: tokio::runtime::Handle::try_current().ok(),
        }
    }

    async fn disconnect(&self) {
        if self.disconnected.swap(true, Ordering::AcqRel) {
            return;
        }
        disconnect_handles(self.target.clone(), self.jumps.clone()).await;
    }
}

impl Drop for RouteCleanup {
    fn drop(&mut self) {
        if self.disconnected.swap(true, Ordering::AcqRel) {
            return;
        }
        let target = self.target.clone();
        let jumps = self.jumps.clone();
        if let Some(runtime) = self
            .runtime
            .clone()
            .or_else(|| tokio::runtime::Handle::try_current().ok())
        {
            runtime.spawn(disconnect_handles(target, jumps));
        } else {
            log::debug!(
                "SSH route dropped without a Tokio runtime; transport handles will close without a graceful disconnect"
            );
        }
    }
}

async fn disconnect_handles(
    target: Arc<client::Handle<ClientHandler>>,
    jumps: Vec<Arc<client::Handle<ClientHandler>>>,
) {
    if let Err(error) = target
        .disconnect(Disconnect::ByApplication, "", "English")
        .await
    {
        log::debug!("failed to disconnect SSH route target cleanly: {error:?}");
    }
    for jump in jumps.into_iter().rev() {
        if let Err(error) = jump
            .disconnect(Disconnect::ByApplication, "", "English")
            .await
        {
            log::debug!("failed to disconnect SSH route jump cleanly: {error:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::validate_bridge_auth_material;
    use miaominal_core::profile::AuthMethod;
    use russh::client::Handler;
    use russh::keys::{Algorithm, PrivateKey};

    #[test]
    fn connector_resolves_jump_chain_in_saved_order() {
        let mut first = SessionProfile::blank("jump-a", 1);
        first.name = "First".into();
        let mut second = SessionProfile::blank("jump-b", 2);
        second.name = "Second".into();
        let mut target = SessionProfile::blank("target", 3);
        target.proxy_jump_profile_ids = vec![first.id.clone(), second.id.clone()];

        let resolved =
            connection::resolve_proxy_jump_profiles(&target, &[second.clone(), first.clone()])
                .expect("jump chain should resolve");
        assert_eq!(
            resolved
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            vec!["jump-a", "jump-b"]
        );
    }

    #[test]
    fn bridge_profiles_keep_all_supported_authentication_methods() {
        for method in [
            AuthMethod::Password,
            AuthMethod::KeyFile,
            AuthMethod::ManagedKey,
            AuthMethod::Agent,
            AuthMethod::KeyboardInteractive,
        ] {
            let mut profile = SessionProfile::blank("profile", 1);
            profile.auth_method = Some(method);
            assert_eq!(profile.effective_auth_method(), method);
        }
    }

    #[test]
    fn connector_rejects_missing_and_repeated_jump_dependencies() {
        let jump = SessionProfile::blank("jump", 1);
        let mut target = SessionProfile::blank("target", 2);
        target.proxy_jump_profile_ids = vec![jump.id.clone(), jump.id.clone()];
        let error = connection::resolve_proxy_jump_profiles(&target, &[jump])
            .expect_err("repeated jump should fail");
        assert!(error.to_string().contains("more than once"));

        target.proxy_jump_profile_ids = vec!["missing".into()];
        let error = connection::resolve_proxy_jump_profiles(&target, &[])
            .expect_err("missing jump should fail");
        assert!(error.to_string().contains("no longer available"));
    }

    #[test]
    fn bridge_credential_preflight_covers_supported_references() {
        let secrets = SecretStore::new_locked_vault();

        let mut password = SessionProfile::blank("password", 1);
        password.password = "secret".into();
        validate_bridge_auth_material(&password, &secrets).expect("inline password is ready");

        let mut keyboard = SessionProfile::blank("keyboard", 2);
        keyboard.auth_method = Some(AuthMethod::KeyboardInteractive);
        keyboard.password = "secret".into();
        validate_bridge_auth_material(&keyboard, &secrets)
            .expect("saved keyboard-interactive password is ready");

        let directory = tempfile::tempdir().expect("key directory");
        let key_path = directory.path().join("id_ed25519");
        std::fs::write(&key_path, "test key placeholder").expect("write key placeholder");
        let mut key_file = SessionProfile::blank("key-file", 3);
        key_file.auth_method = Some(AuthMethod::KeyFile);
        key_file.private_key_path = key_path.to_string_lossy().into_owned();
        validate_bridge_auth_material(&key_file, &secrets).expect("existing key path is ready");

        let mut agent = SessionProfile::blank("agent", 4);
        agent.auth_method = Some(AuthMethod::Agent);
        agent.agent_identity = "ssh-ed25519 AAAA test".into();
        validate_bridge_auth_material(&agent, &secrets).expect("selected agent identity is ready");
    }

    #[test]
    fn bridge_credential_preflight_reports_recoverable_unavailability() {
        let secrets = SecretStore::new_locked_vault();

        let mut password = SessionProfile::blank("password", 1);
        password.has_stored_password = true;
        assert!(
            validate_bridge_auth_material(&password, &secrets)
                .expect_err("locked password should fail")
                .to_string()
                .contains("vault is locked")
        );

        let mut key_file = SessionProfile::blank("key-file", 2);
        key_file.auth_method = Some(AuthMethod::KeyFile);
        key_file.private_key_path = "definitely-missing-miaominal-key".into();
        assert!(
            validate_bridge_auth_material(&key_file, &secrets)
                .expect_err("missing key should fail")
                .to_string()
                .contains("unavailable")
        );

        let mut managed = SessionProfile::blank("managed", 3);
        managed.auth_method = Some(AuthMethod::ManagedKey);
        managed.managed_key_id = "managed-key".into();
        assert!(
            validate_bridge_auth_material(&managed, &secrets)
                .expect_err("locked managed key should fail")
                .to_string()
                .contains("vault is locked")
        );

        let mut agent = SessionProfile::blank("agent", 4);
        agent.auth_method = Some(AuthMethod::Agent);
        assert!(
            validate_bridge_auth_material(&agent, &secrets)
                .expect_err("missing agent identity should fail")
                .to_string()
                .contains("requires an agent identity")
        );
    }

    #[test]
    fn connector_resolves_entry_proxy_and_rejects_missing_proxy() {
        let proxy = ProxyProfile::blank("proxy", 1);
        assert_eq!(
            crate::transport::resolve_entry_proxy(Some("proxy"), std::slice::from_ref(&proxy))
                .expect("proxy lookup should succeed")
                .map(|profile| profile.id.as_str()),
            Some("proxy")
        );
        assert!(
            crate::transport::resolve_entry_proxy(Some("missing"), &[])
                .expect_err("missing proxy should fail")
                .to_string()
                .contains("no longer available")
        );
    }

    #[tokio::test]
    async fn strict_host_key_policy_accepts_only_known_matches() {
        let directory = tempfile::tempdir().expect("known-hosts directory");
        let store = KnownHostsStore::with_path(directory.path().join("known_hosts"));
        let profile = SessionProfile {
            host: "bridge-target.example".into(),
            ..SessionProfile::blank("target", 1)
        };
        let first = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
            .expect("generate first key")
            .public_key()
            .clone();
        let second = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
            .expect("generate second key")
            .public_key()
            .clone();

        let (mut unknown, _) = build_strict_client_handler(&profile, store.clone());
        let error = unknown
            .check_server_key(&first)
            .await
            .expect_err("unknown key should fail");
        assert!(error.to_string().contains("not trusted"));

        store
            .learn(&profile.host, profile.port, &first)
            .expect("learn known host");
        let (mut known, _) = build_strict_client_handler(&profile, store.clone());
        assert!(
            known
                .check_server_key(&first)
                .await
                .expect("known key check")
        );

        let (mut mismatch, _) = build_strict_client_handler(&profile, store);
        let error = mismatch
            .check_server_key(&second)
            .await
            .expect_err("mismatched key should fail");
        assert!(error.to_string().contains("has changed"));
    }
}
