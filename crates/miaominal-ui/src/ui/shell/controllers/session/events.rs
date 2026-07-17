use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::StreamExt as _;
use gpui::Context;
use miaominal_ssh::{SessionEvent, SessionEventReceiver};

use super::{
    AppCommand, SessionConnectionState, SessionController, SessionEventOutcome,
    SessionEventTabRemoval, SessionFailureStatus, SessionNotificationRequest,
    SessionNotificationTone, SessionPurpose,
};
use crate::ui::{i18n, shell::TabId};

const TERMINAL_OUTPUT_BATCH_MAX_CHUNKS: usize = 64;
const TERMINAL_OUTPUT_BATCH_MAX_BYTES: usize = 256 * 1024;
const TERMINAL_OUTPUT_WAKEUP_INTERVAL: Duration = Duration::from_millis(4);

fn coalesce_session_output(
    mut chunk: Vec<u8>,
    events: &mut SessionEventReceiver,
) -> (Vec<u8>, Option<SessionEvent>) {
    let mut chunks = 1usize;

    while chunks < TERMINAL_OUTPUT_BATCH_MAX_CHUNKS && chunk.len() < TERMINAL_OUTPUT_BATCH_MAX_BYTES
    {
        match events.try_recv() {
            Ok(SessionEvent::Output(next)) => {
                chunk.extend_from_slice(&next);
                chunks += 1;
            }
            Ok(event) => return (chunk, Some(event)),
            Err(_) => break,
        }
    }

    (chunk, None)
}

impl SessionController {
    pub(in crate::ui::shell) fn spawn_session_event_loop(
        &self,
        tab_id: TabId,
        mut events: SessionEventReceiver,
        cx: &mut Context<Self>,
    ) {
        let Some(terminal) = self.terminal_state(tab_id) else {
            return;
        };

        cx.spawn(async move |this, cx| {
            let mut pending_event = None;

            loop {
                let event = if let Some(event) = pending_event.take() {
                    event
                } else {
                    let Some(event) = events.next().await else {
                        break;
                    };
                    event
                };
                let event = match event {
                    SessionEvent::Output(chunk) => {
                        let (chunk, pending) = coalesce_session_output(chunk, &mut events);
                        pending_event = pending;
                        terminal.push_bytes(&chunk);
                        cx.background_executor()
                            .timer(TERMINAL_OUTPUT_WAKEUP_INTERVAL)
                            .await;
                        while terminal.has_pending_input() {
                            cx.background_executor()
                                .timer(TERMINAL_OUTPUT_WAKEUP_INTERVAL)
                                .await;
                        }
                        SessionEvent::Output(chunk)
                    }
                    event => event,
                };

                if this
                    .update(cx, |controller, cx| {
                        controller.handle_session_event_from_worker(tab_id, event, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn handle_session_event_from_worker(
        &mut self,
        tab_id: TabId,
        event: SessionEvent,
        cx: &mut Context<Self>,
    ) {
        let (tab_title, inactive_tab) = {
            let ports = self.ports.borrow();
            let snapshot = &ports.snapshot;
            let Some(session) = snapshot.sessions.get(&tab_id) else {
                return;
            };
            (
                session.title.clone(),
                snapshot.active_terminal_tab_id != Some(tab_id),
            )
        };
        let Some(mut outcome) = self.apply_session_event(tab_id, event, inactive_tab, &tab_title)
        else {
            return;
        };

        if let Some(error) = outcome.schedule_reconnect_error.take() {
            self.schedule_reconnect(tab_id, error, cx);
        }
        if let Some(profile_id) = outcome.refresh_monitoring_profile.take() {
            let ordered_tab_ids = self.ordered_terminal_tab_ids();
            self.refresh_profile_monitoring(
                &profile_id,
                Some(tab_id),
                &ordered_tab_ids,
                self.services.auto_collect_session_monitoring.get(),
            );
        }

        if outcome.removal.is_some() {
            self.remove_tab(tab_id);
        }
        if let Some(SessionEventTabRemoval::PortForward {
            profile_id,
            status_message,
        }) = outcome.removal.as_mut()
        {
            let synced_sessions = self.sync_current_port_forward_rules_for_profile(profile_id);
            status_message.push_str(&Self::synced_sessions_suffix(synced_sessions));
        }

        cx.emit(AppCommand::SessionEventApplied { tab_id, outcome });
        cx.notify();
    }

    pub(in crate::ui::shell) fn apply_session_event(
        &self,
        tab_id: TabId,
        event: SessionEvent,
        inactive_tab: bool,
        tab_title: &str,
    ) -> Option<SessionEventOutcome> {
        let terminal_port = self.terminal_port();
        let mut outcome = SessionEventOutcome {
            tab_status: None,
            clipboard_writes: Vec::new(),
            notification: None,
            removal: None,
            schedule_reconnect_error: None,
            refresh_monitoring_profile: None,
            should_notify: true,
        };
        let mut close_connection_test_commands = None;
        let mut port_forward_rule_enabled_update: Option<(String, String, bool)> = None;
        let mut record_connected_profile_id = None;
        let mut monitor_snapshot = None;
        let mut monitor_error = None;

        {
            let mut session = self.tab_mut(tab_id)?;
            let is_port_forward_session = session.purpose == SessionPurpose::PortForwarding;
            let is_connection_test = session.purpose == SessionPurpose::ConnectionTest;

            match event {
                SessionEvent::Connected(connection_label) => {
                    session.set_connection_state(SessionConnectionState::Ready);
                    session.reconnect_attempt = 0;
                    if is_connection_test {
                        outcome.tab_status = Some(i18n::string_args(
                            "session.status.test_succeeded",
                            &[("connection", &connection_label)],
                        ));
                        outcome.notification = Some(SessionNotificationRequest {
                            tone: SessionNotificationTone::Success,
                            title: i18n::string("session.notifications.connection_succeeded_title"),
                            message: i18n::string_args(
                                "session.messages.connection_succeeded_body",
                                &[("connection", &connection_label)],
                            ),
                            id: format!("connection-test-success-{tab_id}"),
                        });
                        close_connection_test_commands = session.commands.clone();
                        outcome.removal = Some(SessionEventTabRemoval::ConnectionTest {
                            status_message: i18n::string_args(
                                "session.messages.test_connection_succeeded_for",
                                &[("connection", &connection_label)],
                            ),
                        });
                    } else if is_port_forward_session {
                        outcome.tab_status = Some(i18n::string_args(
                            "session.status.forwarding_connected",
                            &[("connection", &connection_label)],
                        ));
                        if let Some(rule_id) = session.port_forward_rule_id.clone() {
                            port_forward_rule_enabled_update =
                                Some((session.profile_id.clone(), rule_id, true));
                        }
                    } else {
                        outcome.tab_status = Some(i18n::string_args(
                            "session.status.connected",
                            &[("connection", &connection_label)],
                        ));
                        record_connected_profile_id = Some(session.profile_id.clone());
                    }
                    if inactive_tab {
                        session.has_activity = true;
                    }
                }
                SessionEvent::Output(chunk) => {
                    session.bytes_in = session.bytes_in.saturating_add(chunk.len() as u64);
                    let _ = terminal_port.forward_output(tab_id, chunk);
                    if inactive_tab {
                        outcome.should_notify = !session.has_activity;
                        session.has_activity = true;
                    }
                }
                SessionEvent::Status(message) => {
                    outcome.tab_status = Some(message);
                    if inactive_tab {
                        session.has_activity = true;
                    }
                }
                SessionEvent::MonitorUpdated(snapshot) => {
                    monitor_snapshot = Some((
                        session.profile_id.clone(),
                        session.monitoring.auto_collect_enabled,
                        snapshot,
                    ));
                }
                SessionEvent::MonitorFailed(error) => {
                    monitor_error = Some((
                        session.profile_id.clone(),
                        session.monitoring.auto_collect_enabled,
                        error,
                    ));
                }
                SessionEvent::Error(error) => {
                    let was_ready =
                        matches!(session.connection_state, SessionConnectionState::Ready);
                    let is_in_reconnect_cycle = session.reconnect_attempt > 0;
                    outcome.tab_status = Some(i18n::string("session.status.error"));
                    if !is_port_forward_session
                        && !is_connection_test
                        && (was_ready || is_in_reconnect_cycle)
                    {
                        outcome.schedule_reconnect_error = Some(error.clone());
                    } else if !is_port_forward_session && !is_connection_test && !was_ready {
                        session.set_connection_state(SessionConnectionState::Failed {
                            error: error.clone(),
                            status: None,
                        });
                    }
                    if !is_port_forward_session && !is_connection_test {
                        outcome.refresh_monitoring_profile = Some(session.profile_id.clone());
                    }
                    session.terminal.push_text(&i18n::string_args(
                        "session.terminal.error_line",
                        &[("error", &error)],
                    ));
                    let notification_title = if is_connection_test {
                        i18n::string("session.notifications.test_connection_failed_title")
                    } else if is_port_forward_session {
                        i18n::string("session.notifications.port_forwarding_failed_title")
                    } else if was_ready {
                        i18n::string("session.notifications.session_error_title")
                    } else {
                        i18n::string("session.notifications.connection_failed_title")
                    };
                    let notification_message = if tab_title.trim().is_empty() {
                        error.clone()
                    } else {
                        format!("{tab_title}: {error}")
                    };
                    outcome.notification = Some(SessionNotificationRequest {
                        tone: SessionNotificationTone::Error,
                        title: notification_title,
                        message: notification_message,
                        id: format!("session-failure-{tab_id}"),
                    });
                    if is_connection_test {
                        close_connection_test_commands = session.commands.clone();
                        outcome.removal = Some(SessionEventTabRemoval::ConnectionTest {
                            status_message: i18n::string_args(
                                "session.messages.test_connection_failed_for",
                                &[("profile", &session.profile_id)],
                            ),
                        });
                    } else if is_port_forward_session {
                        if let Some(rule_id) = session.port_forward_rule_id.clone() {
                            port_forward_rule_enabled_update =
                                Some((session.profile_id.clone(), rule_id, false));
                        }
                        outcome.removal = Some(SessionEventTabRemoval::PortForward {
                            profile_id: session.profile_id.clone(),
                            status_message: i18n::string_args(
                                "session.messages.port_forwarding_failed_for",
                                &[("title", tab_title), ("error", &error)],
                            ),
                        });
                    }
                    if inactive_tab {
                        session.has_activity = true;
                    }
                }
                SessionEvent::PortForwardNotice(message) => {
                    session.terminal.push_text(&format!(
                        "{} {message}\r\n",
                        i18n::string("session.terminal.forward_prefix")
                    ));
                    if inactive_tab {
                        session.has_activity = true;
                    }
                }
                SessionEvent::HostKeyPrompt(prompt) => {
                    outcome.tab_status = Some(if prompt.previous_fingerprint.is_some() {
                        i18n::string("session.status.host_key_mismatch")
                    } else {
                        i18n::string("session.status.verify_host_key")
                    });
                    if prompt.previous_fingerprint.is_some() {
                        session.terminal.push_text(&format!(
                            "{} {} {} {}\r\n",
                            i18n::string("session.terminal.host_key_prefix"),
                            prompt.host,
                            prompt.algorithm,
                            prompt.fingerprint
                        ));
                    }
                    session.pending_host_key = Some(prompt);
                    if inactive_tab {
                        session.has_activity = true;
                    }
                }
                SessionEvent::KeyboardInteractivePrompt(challenge) => {
                    outcome.tab_status = Some(if challenge.name.is_empty() {
                        i18n::string("prompts.authentication_challenge")
                    } else {
                        challenge.name.clone()
                    });
                    session.pending_keyboard_interactive = Some(challenge);
                    if inactive_tab {
                        session.has_activity = true;
                    }
                }
                SessionEvent::Closed => {
                    terminal_port.close_session(tab_id);
                    let already_disconnected = matches!(
                        session.connection_state,
                        SessionConnectionState::Disconnected
                    );
                    if !already_disconnected {
                        let was_ready =
                            matches!(session.connection_state, SessionConnectionState::Ready);
                        outcome.tab_status = Some(i18n::string("session.status.closed"));
                        if !is_port_forward_session && !is_connection_test && was_ready {
                            session.set_connection_state(SessionConnectionState::Disconnected);
                        } else if !is_port_forward_session
                            && !is_connection_test
                            && matches!(
                                session.connection_state,
                                SessionConnectionState::Connecting
                            )
                        {
                            let failure_message =
                                i18n::string("session.status.connection_closed_before_ready");
                            session.set_connection_state(SessionConnectionState::Failed {
                                error: failure_message.clone(),
                                status: Some(SessionFailureStatus::Closed),
                            });
                            let notification_message = if tab_title.trim().is_empty() {
                                failure_message
                            } else {
                                format!("{tab_title}: {failure_message}")
                            };
                            outcome.notification = Some(SessionNotificationRequest {
                                tone: SessionNotificationTone::Error,
                                title: i18n::string(
                                    "session.notifications.connection_closed_title",
                                ),
                                message: notification_message,
                                id: format!("session-failure-{tab_id}"),
                            });
                        }
                        session.terminal.push_text(&format!(
                            "{}\r\n",
                            i18n::string("session.terminal.closed_marker")
                        ));
                        if inactive_tab {
                            session.has_activity = true;
                        }
                    }
                    if is_connection_test {
                        if matches!(session.connection_state, SessionConnectionState::Connecting) {
                            outcome.notification = Some(SessionNotificationRequest {
                                tone: SessionNotificationTone::Error,
                                title: i18n::string(
                                    "session.notifications.test_connection_failed_title",
                                ),
                                message: i18n::string_args(
                                    "session.messages.connection_test_closed_before_complete",
                                    &[("title", tab_title)],
                                ),
                                id: format!("connection-test-failure-{tab_id}"),
                            });
                        }
                        outcome.removal = Some(SessionEventTabRemoval::ConnectionTest {
                            status_message: i18n::string_args(
                                "session.messages.finished_test_connection_for",
                                &[("title", tab_title)],
                            ),
                        });
                    } else if is_port_forward_session {
                        if let Some(rule_id) = session.port_forward_rule_id.clone() {
                            port_forward_rule_enabled_update =
                                Some((session.profile_id.clone(), rule_id, false));
                        }
                        outcome.removal = Some(SessionEventTabRemoval::PortForward {
                            profile_id: session.profile_id.clone(),
                            status_message: i18n::string_args(
                                "session.messages.port_forwarding_disconnected_for",
                                &[("title", tab_title)],
                            ),
                        });
                    } else {
                        outcome.refresh_monitoring_profile = Some(session.profile_id.clone());
                    }
                }
            }

            while let Some(emu_event) = session.terminal.try_recv_event() {
                match emu_event {
                    miaominal_terminal::TerminalEvent::ClipboardStore(content) => {
                        outcome.clipboard_writes.push(content);
                    }
                    miaominal_terminal::TerminalEvent::Bell => {
                        if inactive_tab {
                            session.has_activity = true;
                        }
                    }
                }
            }
        }

        if let Some((profile_id, enabled, snapshot)) = monitor_snapshot {
            self.apply_profile_monitor_snapshot(&profile_id, tab_id, enabled, snapshot);
        }
        if let Some((profile_id, enabled, error)) = monitor_error {
            self.apply_profile_monitor_error(&profile_id, tab_id, enabled, error);
        }
        if let Some((profile_id, rule_id, enabled)) = port_forward_rule_enabled_update
            && self
                .update_port_forward_rule_enabled_state(&profile_id, &rule_id, enabled)
                .is_some()
            && let Err(error) = self.persist_profiles()
        {
            log::warn!("failed to persist port-forward rule state: {error:?}");
        }
        if let Some(profile_id) = record_connected_profile_id {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if let Some(profile) = self
                .profiles
                .borrow_mut()
                .iter_mut()
                .find(|profile| profile.id == profile_id)
            {
                profile.last_connected_at = Some(now);
            }
            let _ = self.persist_profiles();
        }
        if let Some(commands) = close_connection_test_commands {
            let _ = commands.close();
        }

        Some(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[test]
    fn coalesce_session_output_merges_consecutive_output() {
        let (sender, receiver) = mpsc::channel(4);
        let mut receiver = SessionEventReceiver::from(receiver);
        sender
            .try_send(SessionEvent::Output(b"b".to_vec()))
            .expect("output event should send");
        sender
            .try_send(SessionEvent::Output(b"c".to_vec()))
            .expect("output event should send");

        let (chunk, pending) = coalesce_session_output(b"a".to_vec(), &mut receiver);

        assert_eq!(chunk, b"abc");
        assert!(pending.is_none());
    }

    #[test]
    fn coalesce_session_output_preserves_next_non_output_event() {
        let (sender, receiver) = mpsc::channel(4);
        let mut receiver = SessionEventReceiver::from(receiver);
        sender
            .try_send(SessionEvent::Output(b"b".to_vec()))
            .expect("output event should send");
        sender
            .try_send(SessionEvent::Status("ready".into()))
            .expect("status event should send");

        let (chunk, pending) = coalesce_session_output(b"a".to_vec(), &mut receiver);

        assert_eq!(chunk, b"ab");
        assert!(matches!(pending, Some(SessionEvent::Status(status)) if status == "ready"));
    }
}
