use super::session::{
    ClientHandler, ConnectedSession, ConnectionCancelled, SessionCommand, SessionCommandSender,
    SessionConnection, SessionEvent, SessionEventSender, connect_authenticated_session_internal,
    session_event_channel,
};
use anyhow::{Context, Result, anyhow, bail};
use miaominal_core::profile::{PortForwardKind, PortForwardRule, SessionProfile};
use miaominal_core::proxy::ProxyProfile;
use miaominal_secrets::SecretStore;
use miaominal_storage::KnownHostsStore;
use russh::client;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::copy_bidirectional;
use tokio::net::TcpListener;
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::task::JoinHandle;

const PORT_FORWARD_HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const PORT_FORWARD_MAX_RECONNECT_ATTEMPTS: u32 = 10;
const PORT_FORWARD_RECONNECT_DELAYS_SECS: &[u64] = &[1, 2, 4, 8, 16, 30];

#[derive(Debug, Clone)]
pub(super) struct RemoteForwardTarget {
    pub label: String,
    pub target_host: String,
    pub target_port: u16,
}

pub(super) type RemoteForwardTargets = Arc<Mutex<HashMap<(String, u16), RemoteForwardTarget>>>;

#[derive(Debug, Clone)]
pub(super) struct ActiveRemoteForward {
    pub rule: PortForwardRule,
    pub label: String,
    pub listen_host: String,
    pub bound_port: u16,
}

pub(super) struct ActiveLocalForward {
    pub rule: PortForwardRule,
    pub task: JoinHandle<Result<()>>,
}

#[derive(Debug, Default)]
pub(super) struct PortForwardSyncReport {
    start_failures: HashMap<String, String>,
}

impl PortForwardSyncReport {
    fn record_start_failure(&mut self, rule_id: String, error: impl Into<String>) {
        self.start_failures.insert(rule_id, error.into());
    }

    fn start_failure(&self, rule_id: &str) -> Option<&str> {
        self.start_failures.get(rule_id).map(String::as_str)
    }
}

pub(super) async fn emit_port_forward_notice(
    event_sender: &SessionEventSender,
    message: impl Into<String>,
) {
    let _ = event_sender
        .send(SessionEvent::PortForwardNotice(message.into()))
        .await;
}

async fn start_local_forward(
    session: Arc<client::Handle<ClientHandler>>,
    rule: PortForwardRule,
    event_sender: SessionEventSender,
) -> Result<ActiveLocalForward> {
    let (listener, bind_address) = bind_local_forward_listener(&rule).await?;

    emit_port_forward_notice(
        &event_sender,
        format!("Forward {} is listening on {}", rule.label, bind_address),
    )
    .await;

    let active_rule = rule.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, originator) = match listener.accept().await {
                Ok(connection) => connection,
                Err(error) => {
                    let message = format!(
                        "Forward {} stopped accepting connections: {}",
                        rule.label, error
                    );
                    emit_port_forward_notice(&event_sender, message.clone()).await;
                    bail!(message);
                }
            };

            let session = session.clone();
            let event_sender = event_sender.clone();
            let target_host = rule.target_host.clone();
            let target_port = rule.target_port;
            let label = rule.label.clone();
            tokio::spawn(async move {
                match session
                    .channel_open_direct_tcpip(
                        target_host.clone(),
                        u32::from(target_port),
                        originator.ip().to_string(),
                        u32::from(originator.port()),
                    )
                    .await
                {
                    Ok(channel) => {
                        let mut forwarded = channel.into_stream();
                        if let Err(error) = copy_bidirectional(&mut forwarded, &mut stream).await {
                            emit_port_forward_notice(
                                &event_sender,
                                format!("Forward {} relay failed: {}", label, error),
                            )
                            .await;
                        }
                    }
                    Err(error) => {
                        emit_port_forward_notice(
                            &event_sender,
                            format!("Forward {} could not open SSH channel: {}", label, error),
                        )
                        .await
                    }
                }
            });
        }
    });

    Ok(ActiveLocalForward {
        rule: active_rule,
        task,
    })
}

async fn bind_local_forward_listener(rule: &PortForwardRule) -> Result<(TcpListener, String)> {
    let bind_address = format!("{}:{}", rule.listen_host, rule.listen_port);
    let listener = TcpListener::bind(&bind_address)
        .await
        .with_context(|| format!("Forward {} failed to bind on {}", rule.label, bind_address))?;
    Ok((listener, bind_address))
}

pub(super) async fn sync_port_forward_rules(
    session: &Arc<client::Handle<ClientHandler>>,
    desired_rules: &[PortForwardRule],
    active_local_forwards: &mut HashMap<String, ActiveLocalForward>,
    active_remote_forwards: &mut HashMap<String, ActiveRemoteForward>,
    remote_forward_targets: &RemoteForwardTargets,
    event_sender: &SessionEventSender,
) -> PortForwardSyncReport {
    let mut report = PortForwardSyncReport::default();
    let desired_local: HashMap<_, _> = desired_rules
        .iter()
        .filter(|rule| rule.enabled && rule.kind == PortForwardKind::Local)
        .map(|rule| (rule.id.clone(), rule.clone()))
        .collect();
    let desired_remote: HashMap<_, _> = desired_rules
        .iter()
        .filter(|rule| rule.enabled && rule.kind == PortForwardKind::Remote)
        .map(|rule| (rule.id.clone(), rule.clone()))
        .collect();

    let local_to_stop: Vec<_> = active_local_forwards
        .iter()
        .filter_map(|(rule_id, active)| match desired_local.get(rule_id) {
            Some(desired_rule) if active.rule == *desired_rule => None,
            Some(_) | None => Some(rule_id.clone()),
        })
        .collect();
    for rule_id in local_to_stop {
        if let Some(active) = active_local_forwards.remove(&rule_id) {
            active.task.abort();
            let _ = active.task.await;
            emit_port_forward_notice(event_sender, format!("Stopped local forward {}", rule_id))
                .await;
        }
    }

    let remote_to_stop: Vec<_> = active_remote_forwards
        .iter()
        .filter_map(|(rule_id, active)| match desired_remote.get(rule_id) {
            Some(desired_rule) if active.rule == *desired_rule => None,
            Some(_) | None => Some(rule_id.clone()),
        })
        .collect();
    for rule_id in remote_to_stop {
        if let Some(active) = active_remote_forwards.remove(&rule_id) {
            match session
                .cancel_tcpip_forward(active.listen_host.clone(), u32::from(active.bound_port))
                .await
            {
                Ok(()) => {
                    emit_port_forward_notice(
                        event_sender,
                        format!("Stopped remote forward {}", active.label),
                    )
                    .await;
                }
                Err(error) => {
                    emit_port_forward_notice(
                        event_sender,
                        format!("Failed to stop remote forward {}: {}", active.label, error),
                    )
                    .await;
                }
            }
            if let Ok(mut targets) = remote_forward_targets.lock() {
                targets.remove(&(active.listen_host, active.bound_port));
            }
        }
    }

    for (rule_id, rule) in desired_local {
        if active_local_forwards.contains_key(&rule_id) {
            continue;
        }

        match start_local_forward(session.clone(), rule.clone(), event_sender.clone()).await {
            Ok(active) => {
                active_local_forwards.insert(rule_id, active);
            }
            Err(error) => {
                let message = format!("{error:#}");
                emit_port_forward_notice(event_sender, message.clone()).await;
                report.record_start_failure(rule_id, message);
            }
        }
    }

    for (rule_id, rule) in desired_remote {
        if active_remote_forwards.contains_key(&rule_id) {
            continue;
        }

        match session
            .tcpip_forward(rule.listen_host.clone(), u32::from(rule.listen_port))
            .await
        {
            Ok(bound_port) => {
                let Ok(bound_port) = u16::try_from(bound_port) else {
                    let message = format!(
                        "Remote forward {} returned unsupported port {}",
                        rule.label, bound_port
                    );
                    emit_port_forward_notice(event_sender, message.clone()).await;
                    report.record_start_failure(rule_id, message);
                    continue;
                };

                let target_registered = remote_forward_targets.lock().map(|mut targets| {
                    targets.insert(
                        (rule.listen_host.clone(), bound_port),
                        RemoteForwardTarget {
                            label: rule.label.clone(),
                            target_host: rule.target_host.clone(),
                            target_port: rule.target_port,
                        },
                    );
                });
                if target_registered.is_err() {
                    let message = format!(
                        "Remote forward {} target registry is unavailable",
                        rule.label
                    );
                    let _ = session
                        .cancel_tcpip_forward(rule.listen_host.clone(), u32::from(bound_port))
                        .await;
                    emit_port_forward_notice(event_sender, message.clone()).await;
                    report.record_start_failure(rule_id, message);
                    continue;
                }
                active_remote_forwards.insert(
                    rule_id,
                    ActiveRemoteForward {
                        rule: rule.clone(),
                        label: rule.label.clone(),
                        listen_host: rule.listen_host.clone(),
                        bound_port,
                    },
                );
                emit_port_forward_notice(
                    event_sender,
                    format!(
                        "Remote forward {} is listening on {}:{}",
                        rule.label, rule.listen_host, bound_port
                    ),
                )
                .await;
            }
            Err(error) => {
                let message = format!("Failed to start remote forward {}: {}", rule.label, error);
                emit_port_forward_notice(event_sender, message.clone()).await;
                report.record_start_failure(rule_id, message);
            }
        }
    }

    report
}

pub fn start_port_forward_session(
    runtime: &TokioHandle,
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
) -> SessionConnection {
    let (event_sender, event_receiver) = session_event_channel();
    let (command_sender, command_receiver) = unbounded_channel();
    let runtime = runtime.clone();

    std::thread::Builder::new()
        .name(format!("ssh-forward-{}", profile.id))
        .spawn(move || {
            runtime.block_on(async move {
                if let Err(error) = run_port_forward_session(
                    profile,
                    all_profiles,
                    all_proxies,
                    secrets,
                    known_hosts,
                    command_receiver,
                    event_sender.clone(),
                )
                .await
                {
                    if event_sender
                        .send(SessionEvent::Error(error.to_string()))
                        .await
                        .is_err()
                    {
                        return;
                    }

                    let _ = event_sender.send(SessionEvent::Closed).await;
                }
            });
        })
        .expect("failed to spawn SSH port forwarding thread");

    SessionConnection::new(SessionCommandSender::new(command_sender), event_receiver)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReconnectWaitOutcome {
    Retry,
    Closed,
}

#[derive(Debug)]
enum ConnectedForwardOutcome {
    Closed,
    Lost(String),
    Fatal(anyhow::Error),
}

fn reconnect_delay_secs(attempt: u32) -> u64 {
    PORT_FORWARD_RECONNECT_DELAYS_SECS
        .get(attempt.saturating_sub(1) as usize)
        .copied()
        .unwrap_or(30)
}

fn next_reconnect_attempt(current_attempt: u32) -> Option<(u32, u64)> {
    let attempt = current_attempt.checked_add(1)?;
    (attempt <= PORT_FORWARD_MAX_RECONNECT_ATTEMPTS)
        .then(|| (attempt, reconnect_delay_secs(attempt)))
}

fn connection_was_cancelled(error: &anyhow::Error) -> bool {
    error.downcast_ref::<ConnectionCancelled>().is_some()
}

fn ensure_dedicated_forward_active(
    configured_rules: &[PortForwardRule],
    active_local_forwards: &HashMap<String, ActiveLocalForward>,
    active_remote_forwards: &HashMap<String, ActiveRemoteForward>,
    report: &PortForwardSyncReport,
) -> Result<()> {
    let mut enabled_rules = configured_rules.iter().filter(|rule| rule.enabled);
    let rule = enabled_rules
        .next()
        .ok_or_else(|| anyhow!("dedicated port forwarding session has no enabled rule"))?;
    if enabled_rules.next().is_some() {
        bail!("dedicated port forwarding session has more than one enabled rule");
    }

    let active = match rule.kind {
        PortForwardKind::Local => active_local_forwards.contains_key(&rule.id),
        PortForwardKind::Remote => active_remote_forwards.contains_key(&rule.id),
    };
    if active {
        return Ok(());
    }

    if let Some(error) = report.start_failure(&rule.id) {
        bail!("failed to start port forward {}: {}", rule.label, error);
    }
    bail!("port forward {} did not become active", rule.label)
}

async fn wait_for_reconnect_delay(
    command_receiver: &mut UnboundedReceiver<SessionCommand>,
    configured_rules: &mut Vec<PortForwardRule>,
    event_sender: &SessionEventSender,
    error: String,
    attempt: u32,
    retry_after_secs: u64,
    delay: Duration,
) -> ReconnectWaitOutcome {
    if event_sender
        .send(SessionEvent::PortForwardReconnecting {
            error,
            attempt,
            max_attempts: PORT_FORWARD_MAX_RECONNECT_ATTEMPTS,
            retry_after_secs,
        })
        .await
        .is_err()
    {
        return ReconnectWaitOutcome::Closed;
    }

    let delay = tokio::time::sleep(delay);
    tokio::pin!(delay);
    loop {
        tokio::select! {
            _ = &mut delay => return ReconnectWaitOutcome::Retry,
            command = command_receiver.recv() => {
                match command {
                    Some(SessionCommand::SyncPortForwardRules(rules)) => {
                        *configured_rules = rules;
                    }
                    Some(SessionCommand::Close) | None => {
                        return ReconnectWaitOutcome::Closed;
                    }
                    Some(
                        SessionCommand::HostKeyDecision(_)
                        | SessionCommand::KeyboardInteractiveResponse(_)
                        | SessionCommand::SetMonitoringEnabled(_)
                        | SessionCommand::Send(_)
                        | SessionCommand::Resize { .. },
                    ) => {}
                }
            }
        }
    }
}

async fn supervise_connected_forward(
    session: &Arc<client::Handle<ClientHandler>>,
    configured_rules: &mut Vec<PortForwardRule>,
    active_local_forwards: &mut HashMap<String, ActiveLocalForward>,
    active_remote_forwards: &mut HashMap<String, ActiveRemoteForward>,
    remote_forward_targets: &RemoteForwardTargets,
    command_receiver: &mut UnboundedReceiver<SessionCommand>,
    event_sender: &SessionEventSender,
) -> ConnectedForwardOutcome {
    let mut health_check = tokio::time::interval(PORT_FORWARD_HEALTH_CHECK_INTERVAL);
    health_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            command = command_receiver.recv() => {
                match command {
                    Some(SessionCommand::SyncPortForwardRules(rules)) => {
                        *configured_rules = rules;
                        let report = sync_port_forward_rules(
                            session,
                            configured_rules,
                            active_local_forwards,
                            active_remote_forwards,
                            remote_forward_targets,
                            event_sender,
                        )
                        .await;
                        if let Err(error) = ensure_dedicated_forward_active(
                            configured_rules,
                            active_local_forwards,
                            active_remote_forwards,
                            &report,
                        ) {
                            return ConnectedForwardOutcome::Fatal(error);
                        }
                    }
                    Some(SessionCommand::Close) | None => {
                        return ConnectedForwardOutcome::Closed;
                    }
                    Some(
                        SessionCommand::HostKeyDecision(_)
                        | SessionCommand::KeyboardInteractiveResponse(_)
                        | SessionCommand::SetMonitoringEnabled(_)
                        | SessionCommand::Send(_)
                        | SessionCommand::Resize { .. },
                    ) => {}
                }
            }
            _ = health_check.tick() => {
                if session.is_closed() {
                    return ConnectedForwardOutcome::Lost(
                        "SSH transport closed while port forwarding was active".into(),
                    );
                }

                let failed_rule_id = active_local_forwards
                    .iter()
                    .find_map(|(rule_id, active)| active.task.is_finished().then(|| rule_id.clone()));
                if let Some(rule_id) = failed_rule_id
                    && let Some(active) = active_local_forwards.remove(&rule_id)
                {
                    let label = active.rule.label.clone();
                    let error = match active.task.await {
                        Ok(Ok(())) => format!("Forward {label} listener stopped unexpectedly"),
                        Ok(Err(error)) => format!("{error:#}"),
                        Err(error) => format!("Forward {label} listener task failed: {error}"),
                    };
                    return ConnectedForwardOutcome::Lost(error);
                }
            }
        }
    }
}

async fn stop_active_forwards(
    route: &ConnectedSession,
    active_local_forwards: &mut HashMap<String, ActiveLocalForward>,
    active_remote_forwards: &mut HashMap<String, ActiveRemoteForward>,
    event_sender: &SessionEventSender,
) {
    let _ = sync_port_forward_rules(
        &route.session,
        &[],
        active_local_forwards,
        active_remote_forwards,
        &route.remote_forward_targets,
        event_sender,
    )
    .await;
    route.disconnect().await;
}

async fn run_port_forward_session(
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    mut command_receiver: UnboundedReceiver<SessionCommand>,
    event_sender: SessionEventSender,
) -> Result<()> {
    let remote_label = profile.connection_label();
    let mut configured_port_forward_rules = profile.port_forwarding_rules.clone();
    let mut connected_once = false;
    let mut reconnect_attempt = 0;
    let mut reconnect_error: Option<String> = None;

    loop {
        if let Some(error) = reconnect_error.take() {
            let Some((next_attempt, retry_after_secs)) = next_reconnect_attempt(reconnect_attempt)
            else {
                bail!(
                    "port forwarding reconnect failed after {} attempts: {}",
                    PORT_FORWARD_MAX_RECONNECT_ATTEMPTS,
                    error
                );
            };
            reconnect_attempt = next_attempt;
            if wait_for_reconnect_delay(
                &mut command_receiver,
                &mut configured_port_forward_rules,
                &event_sender,
                error,
                reconnect_attempt,
                retry_after_secs,
                Duration::from_secs(retry_after_secs),
            )
            .await
                == ReconnectWaitOutcome::Closed
            {
                let _ = event_sender.send(SessionEvent::Closed).await;
                return Ok(());
            }
        }

        let mut connection_profile = profile.clone();
        connection_profile.port_forwarding_rules = configured_port_forward_rules.clone();
        let route = match connect_authenticated_session_internal(
            connection_profile,
            all_profiles.clone(),
            all_proxies.clone(),
            secrets.clone(),
            known_hosts.clone(),
            &mut command_receiver,
            &event_sender,
        )
        .await
        {
            Ok(route) => route,
            Err(error) if connection_was_cancelled(&error) => {
                let _ = event_sender.send(SessionEvent::Closed).await;
                return Ok(());
            }
            Err(error) if connected_once => {
                reconnect_error = Some(format!("{error:#}"));
                continue;
            }
            Err(error) => return Err(error),
        };

        configured_port_forward_rules = route.configured_port_forward_rules.clone();
        let mut active_local_forwards = HashMap::new();
        let mut active_remote_forwards = HashMap::new();
        let report = sync_port_forward_rules(
            &route.session,
            &configured_port_forward_rules,
            &mut active_local_forwards,
            &mut active_remote_forwards,
            &route.remote_forward_targets,
            &event_sender,
        )
        .await;
        if let Err(error) = ensure_dedicated_forward_active(
            &configured_port_forward_rules,
            &active_local_forwards,
            &active_remote_forwards,
            &report,
        ) {
            stop_active_forwards(
                &route,
                &mut active_local_forwards,
                &mut active_remote_forwards,
                &event_sender,
            )
            .await;
            return Err(error);
        }

        if route.session.is_closed() {
            stop_active_forwards(
                &route,
                &mut active_local_forwards,
                &mut active_remote_forwards,
                &event_sender,
            )
            .await;
            let error = "SSH transport closed before port forwarding became ready".to_string();
            if connected_once {
                reconnect_error = Some(error);
                continue;
            }
            bail!(error);
        }

        if event_sender
            .send(SessionEvent::Connected(remote_label.clone()))
            .await
            .is_err()
        {
            stop_active_forwards(
                &route,
                &mut active_local_forwards,
                &mut active_remote_forwards,
                &event_sender,
            )
            .await;
            return Ok(());
        }
        connected_once = true;
        reconnect_attempt = 0;

        let outcome = supervise_connected_forward(
            &route.session,
            &mut configured_port_forward_rules,
            &mut active_local_forwards,
            &mut active_remote_forwards,
            &route.remote_forward_targets,
            &mut command_receiver,
            &event_sender,
        )
        .await;
        stop_active_forwards(
            &route,
            &mut active_local_forwards,
            &mut active_remote_forwards,
            &event_sender,
        )
        .await;

        match outcome {
            ConnectedForwardOutcome::Closed => {
                let _ = event_sender.send(SessionEvent::Closed).await;
                return Ok(());
            }
            ConnectedForwardOutcome::Lost(error) => {
                reconnect_error = Some(error);
            }
            ConnectedForwardOutcome::Fatal(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::profile::PortForwardRule;

    fn local_rule(listen_port: u16) -> PortForwardRule {
        PortForwardRule {
            id: "forward-a".to_string(),
            label: "Forward A".to_string(),
            kind: PortForwardKind::Local,
            listen_host: "127.0.0.1".to_string(),
            listen_port,
            target_host: "127.0.0.1".to_string(),
            target_port: 8080,
            enabled: true,
        }
    }

    #[test]
    fn reconnect_delays_follow_bounded_backoff() {
        let delays = (1..=PORT_FORWARD_MAX_RECONNECT_ATTEMPTS)
            .map(reconnect_delay_secs)
            .collect::<Vec<_>>();

        assert_eq!(delays, vec![1, 2, 4, 8, 16, 30, 30, 30, 30, 30]);
        assert_eq!(next_reconnect_attempt(0), Some((1, 1)));
        assert_eq!(next_reconnect_attempt(9), Some((10, 30)));
        assert_eq!(next_reconnect_attempt(10), None);
    }

    #[test]
    fn connection_cancellation_detection_uses_the_error_type() {
        let cancelled = anyhow::Error::new(ConnectionCancelled).context("connect failed");
        assert!(connection_was_cancelled(&cancelled));

        let same_message = anyhow!("connection cancelled");
        assert!(!connection_was_cancelled(&same_message));
    }

    #[test]
    fn dedicated_forward_requires_exactly_one_active_rule() {
        let rule = local_rule(8081);
        let rules = vec![rule.clone()];
        let active_local = HashMap::new();
        let active_remote = HashMap::new();
        let mut report = PortForwardSyncReport::default();
        report.record_start_failure(rule.id.clone(), "address already in use");

        let error = ensure_dedicated_forward_active(&rules, &active_local, &active_remote, &report)
            .expect_err("inactive dedicated rule should fail");

        assert!(error.to_string().contains("address already in use"));
    }

    #[tokio::test]
    async fn occupied_local_port_is_rejected_before_listener_task_starts() {
        let occupied = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let port = occupied
            .local_addr()
            .expect("test listener should have an address")
            .port();
        let rule = local_rule(port);

        let error = bind_local_forward_listener(&rule)
            .await
            .expect_err("occupied forwarding port should not bind");

        assert!(error.to_string().contains("failed to bind"));
        assert!(error.to_string().contains(&port.to_string()));
    }

    #[tokio::test]
    async fn reconnect_wait_emits_state_and_stops_immediately_on_close() {
        let (event_sender, mut events) = session_event_channel();
        let (commands, mut command_receiver) = unbounded_channel();
        let mut configured_rules = vec![local_rule(8082)];
        commands
            .send(SessionCommand::Close)
            .expect("close command should send");

        let outcome = wait_for_reconnect_delay(
            &mut command_receiver,
            &mut configured_rules,
            &event_sender,
            "transport closed".to_string(),
            2,
            4,
            Duration::from_secs(60),
        )
        .await;

        assert_eq!(outcome, ReconnectWaitOutcome::Closed);
        assert!(matches!(
            events.recv().await,
            Some(SessionEvent::PortForwardReconnecting {
                attempt: 2,
                max_attempts: PORT_FORWARD_MAX_RECONNECT_ATTEMPTS,
                retry_after_secs: 4,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn reconnect_wait_keeps_latest_synced_rule() {
        let (event_sender, _events) = session_event_channel();
        let (commands, mut command_receiver) = unbounded_channel();
        let mut configured_rules = vec![local_rule(8083)];
        let updated = local_rule(8084);
        commands
            .send(SessionCommand::SyncPortForwardRules(vec![updated.clone()]))
            .expect("sync command should send");
        commands
            .send(SessionCommand::Close)
            .expect("close command should send");

        let outcome = wait_for_reconnect_delay(
            &mut command_receiver,
            &mut configured_rules,
            &event_sender,
            "transport closed".to_string(),
            1,
            1,
            Duration::from_secs(60),
        )
        .await;

        assert_eq!(outcome, ReconnectWaitOutcome::Closed);
        assert_eq!(configured_rules, vec![updated]);
    }
}
