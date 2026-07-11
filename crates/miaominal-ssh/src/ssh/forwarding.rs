use super::session::{
    ClientHandler, ConnectedSession, SessionCommand, SessionCommandSender, SessionConnection,
    SessionEvent, SessionEventSender, connect_authenticated_session_internal,
    session_event_channel,
};
use anyhow::Result;
use miaominal_core::profile::{PortForwardKind, PortForwardRule, SessionProfile};
use miaominal_secrets::SecretStore;
use miaominal_storage::KnownHostsStore;
use russh::{Disconnect, client};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::copy_bidirectional;
use tokio::net::TcpListener;
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::task::JoinHandle;

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
    pub task: JoinHandle<()>,
}

pub(super) async fn emit_port_forward_notice(
    event_sender: &SessionEventSender,
    message: impl Into<String>,
) {
    let _ = event_sender
        .send(SessionEvent::PortForwardNotice(message.into()))
        .await;
}

pub(super) fn spawn_local_forward_task(
    session: Arc<client::Handle<ClientHandler>>,
    rule: PortForwardRule,
    event_sender: SessionEventSender,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let bind_address = format!("{}:{}", rule.listen_host, rule.listen_port);
        let listener = match TcpListener::bind(&bind_address).await {
            Ok(listener) => listener,
            Err(error) => {
                emit_port_forward_notice(
                    &event_sender,
                    format!(
                        "Forward {} failed to bind on {}: {}",
                        rule.label, bind_address, error
                    ),
                )
                .await;
                return;
            }
        };

        emit_port_forward_notice(
            &event_sender,
            format!("Forward {} is listening on {}", rule.label, bind_address),
        )
        .await;

        loop {
            let (mut stream, originator) = match listener.accept().await {
                Ok(connection) => connection,
                Err(error) => {
                    emit_port_forward_notice(
                        &event_sender,
                        format!(
                            "Forward {} stopped accepting connections: {}",
                            rule.label, error
                        ),
                    )
                    .await;
                    break;
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
    })
}

pub(super) async fn sync_port_forward_rules(
    session: &Arc<client::Handle<ClientHandler>>,
    desired_rules: &[PortForwardRule],
    active_local_forwards: &mut HashMap<String, ActiveLocalForward>,
    active_remote_forwards: &mut HashMap<String, ActiveRemoteForward>,
    remote_forward_targets: &RemoteForwardTargets,
    event_sender: &SessionEventSender,
) {
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

        let active_rule = rule.clone();
        let task = spawn_local_forward_task(session.clone(), rule, event_sender.clone());
        active_local_forwards.insert(
            rule_id,
            ActiveLocalForward {
                rule: active_rule,
                task,
            },
        );
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
                    emit_port_forward_notice(
                        event_sender,
                        format!(
                            "Remote forward {} returned unsupported port {}",
                            rule.label, bound_port
                        ),
                    )
                    .await;
                    continue;
                };

                if let Ok(mut targets) = remote_forward_targets.lock() {
                    targets.insert(
                        (rule.listen_host.clone(), bound_port),
                        RemoteForwardTarget {
                            label: rule.label.clone(),
                            target_host: rule.target_host.clone(),
                            target_port: rule.target_port,
                        },
                    );
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
                emit_port_forward_notice(
                    event_sender,
                    format!("Failed to start remote forward {}: {}", rule.label, error),
                )
                .await
            }
        }
    }
}

pub fn start_port_forward_session(
    runtime: &TokioHandle,
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
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

async fn run_port_forward_session(
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    mut command_receiver: UnboundedReceiver<SessionCommand>,
    event_sender: SessionEventSender,
) -> Result<()> {
    let remote_label = profile.connection_label();
    let ConnectedSession {
        session,
        mut configured_port_forward_rules,
        remote_forward_targets,
        jump_sessions,
    } = connect_authenticated_session_internal(
        profile,
        all_profiles,
        secrets,
        known_hosts,
        &mut command_receiver,
        &event_sender,
    )
    .await?;

    let mut active_local_forwards = HashMap::new();
    let mut active_remote_forwards = HashMap::new();
    sync_port_forward_rules(
        &session,
        &configured_port_forward_rules,
        &mut active_local_forwards,
        &mut active_remote_forwards,
        &remote_forward_targets,
        &event_sender,
    )
    .await;

    if event_sender
        .send(SessionEvent::Connected(remote_label))
        .await
        .is_err()
    {
        return Ok(());
    }

    while let Some(command) = command_receiver.recv().await {
        match command {
            SessionCommand::HostKeyDecision(_) => {}
            SessionCommand::KeyboardInteractiveResponse(_) => {}
            SessionCommand::SetMonitoringEnabled(_) => {}
            SessionCommand::SyncPortForwardRules(rules) => {
                configured_port_forward_rules = rules;
                sync_port_forward_rules(
                    &session,
                    &configured_port_forward_rules,
                    &mut active_local_forwards,
                    &mut active_remote_forwards,
                    &remote_forward_targets,
                    &event_sender,
                )
                .await;
            }
            SessionCommand::Close => break,
            SessionCommand::Send(_) | SessionCommand::Resize { .. } => {}
        }
    }

    sync_port_forward_rules(
        &session,
        &[],
        &mut active_local_forwards,
        &mut active_remote_forwards,
        &remote_forward_targets,
        &event_sender,
    )
    .await;

    if let Err(error) = session
        .disconnect(Disconnect::ByApplication, "", "English")
        .await
    {
        log::debug!("failed to disconnect port forwarding session cleanly: {error:?}");
    }

    for jump_session in jump_sessions.into_iter().rev() {
        if let Err(error) = jump_session
            .disconnect(Disconnect::ByApplication, "", "English")
            .await
        {
            log::debug!("failed to disconnect ProxyJump session cleanly: {error:?}");
        }
    }

    let _ = event_sender.send(SessionEvent::Closed).await;

    Ok(())
}
