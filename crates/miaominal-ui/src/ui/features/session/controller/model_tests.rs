use super::*;
fn session_state(
    purpose: SessionPurpose,
    connection_state: SessionConnectionState,
) -> SessionTabState {
    SessionTabState {
        profile_id: "profile".to_string(),
        port_forward_rule_id: None,
        terminal: TerminalState::default(),
        connection_state,
        preserved_history_popup_hidden: false,
        pending_profile: None,
        commands: None,
        bytes_in: 0,
        bytes_out: 0,
        pending_host_key: None,
        pending_host_key_request_id: 0,
        pending_keyboard_interactive: None,
        pending_keyboard_interactive_request_id: 0,
        reconnect_task: None,
        reconnect_attempt: 0,
        has_activity: false,
        monitoring: SessionMonitoringState::new(false),
        purpose,
        port_forward_revision: 0,
        port_forward_log_len: 0,
        sftp_progress_layout: SessionSftpProgressLayoutState::default(),
        local_terminal: false,
        owner_route: None,
    }
}

fn monitor_snapshot(cpu_percent: f64, rx: f64, tx: f64) -> SessionMonitorSnapshot {
    monitor_snapshot_for(
        miaominal_core::forwarding::SessionMonitorPlatform::Linux,
        cpu_percent,
        rx,
        tx,
    )
}

fn monitor_snapshot_for(
    platform: miaominal_core::forwarding::SessionMonitorPlatform,
    cpu_percent: f64,
    rx: f64,
    tx: f64,
) -> SessionMonitorSnapshot {
    SessionMonitorSnapshot {
        platform,
        hostname: Some("host".into()),
        logical_cpu_count: Some(4),
        uptime_seconds: Some(60),
        cpu_percent,
        memory_percent: 25.0,
        memory_used_bytes: 1,
        memory_total_bytes: 4,
        swap_percent: 0.0,
        swap_used_bytes: 0,
        swap_total_bytes: 0,
        disk_percent: 50.0,
        disk_used_bytes: Some(1),
        disk_total_bytes: Some(2),
        network_rx_kbps: rx,
        network_tx_kbps: tx,
        load: 1.0,
    }
}

#[test]
fn monitoring_history_treats_the_first_rate_sample_as_warmup() {
    let mut state = SessionMonitoringState::new(true);
    state.apply_snapshot(monitor_snapshot(0.0, 0.0, 0.0));

    assert_eq!(state.sample_count, 1);
    assert!(state.cpu_history.is_empty());
    assert!(state.network_rx_history.is_empty());
    assert!(state.network_tx_history.is_empty());
    assert_eq!(state.memory_history.len(), 1);
    assert!(!state.cpu_sample_ready);
    assert!(!state.network_sample_ready);

    state.apply_snapshot(monitor_snapshot(42.0, 12.0, 8.0));

    assert_eq!(state.sample_count, 2);
    assert_eq!(state.cpu_history[0].value, 42.0);
    assert_eq!(state.network_rx_history[0].value, 12.0);
    assert_eq!(state.network_tx_history[0].value, 8.0);
    assert_eq!(state.memory_history.len(), 2);
    assert!(state.last_updated_at.is_some());
    assert!(state.cpu_sample_ready);
    assert!(state.network_sample_ready);
}

#[test]
fn monitoring_warmup_is_platform_aware() {
    let mut macos = SessionMonitoringState::new(true);
    macos.apply_snapshot(monitor_snapshot_for(
        miaominal_core::forwarding::SessionMonitorPlatform::Macos,
        20.0,
        0.0,
        0.0,
    ));
    assert!(macos.cpu_sample_ready);
    assert!(!macos.network_sample_ready);
    assert_eq!(macos.cpu_history.len(), 1);
    assert!(macos.network_rx_history.is_empty());

    let mut windows = SessionMonitoringState::new(true);
    windows.apply_snapshot(monitor_snapshot_for(
        miaominal_core::forwarding::SessionMonitorPlatform::Windows,
        20.0,
        4.0,
        2.0,
    ));
    assert!(windows.cpu_sample_ready);
    assert!(windows.network_sample_ready);
    assert_eq!(windows.cpu_history.len(), 1);
    assert_eq!(windows.network_rx_history.len(), 1);
}

#[test]
fn monitoring_error_rearms_the_next_unix_rate_sample() {
    let mut state = SessionMonitoringState::new(true);
    state.apply_snapshot(monitor_snapshot(0.0, 0.0, 0.0));
    state.apply_snapshot(monitor_snapshot(25.0, 10.0, 5.0));
    assert_eq!(state.cpu_history.len(), 1);
    assert_eq!(state.network_rx_history.len(), 1);

    state.report_error("failed".into());
    state.apply_snapshot(monitor_snapshot(0.0, 0.0, 0.0));
    assert_eq!(state.cpu_history.len(), 1);
    assert_eq!(state.network_rx_history.len(), 1);
    assert!(!state.cpu_sample_ready);
    assert!(!state.network_sample_ready);

    state.apply_snapshot(monitor_snapshot(30.0, 12.0, 6.0));
    assert_eq!(state.cpu_history.len(), 2);
    assert_eq!(state.network_rx_history.len(), 2);
    assert!(state.cpu_sample_ready);
    assert!(state.network_sample_ready);
}

#[test]
fn monitoring_rewarm_after_full_history_skips_only_the_rate_label() {
    let mut state = SessionMonitoringState::new(true);
    for sample in 0..=SESSION_MONITOR_HISTORY_LIMIT {
        state.apply_snapshot(monitor_snapshot(sample as f64, 1.0, 1.0));
    }
    let before_rewarm_label = (SESSION_MONITOR_HISTORY_LIMIT + 1).to_string();
    let warmup_label = (SESSION_MONITOR_HISTORY_LIMIT + 2).to_string();
    let resumed_label = (SESSION_MONITOR_HISTORY_LIMIT + 3).to_string();

    assert_eq!(state.cpu_history.len(), SESSION_MONITOR_HISTORY_LIMIT);
    assert_eq!(state.memory_history.len(), SESSION_MONITOR_HISTORY_LIMIT);
    assert_eq!(state.cpu_history.last().unwrap().label, before_rewarm_label);
    assert_eq!(
        state.memory_history.last().unwrap().label,
        before_rewarm_label
    );

    state.report_error("failed".into());
    state.apply_snapshot(monitor_snapshot(0.0, 0.0, 0.0));

    assert_eq!(state.cpu_history.len(), SESSION_MONITOR_HISTORY_LIMIT);
    assert_eq!(state.memory_history.len(), SESSION_MONITOR_HISTORY_LIMIT);
    assert_eq!(state.cpu_history.last().unwrap().label, before_rewarm_label);
    assert_eq!(state.memory_history.last().unwrap().label, warmup_label);

    state.apply_snapshot(monitor_snapshot(25.0, 2.0, 2.0));
    assert_eq!(state.cpu_history.last().unwrap().label, resumed_label);
    assert_eq!(state.memory_history.last().unwrap().label, resumed_label);
    assert!(
        !state
            .cpu_history
            .iter()
            .any(|point| point.label == warmup_label)
    );
    assert!(
        state
            .memory_history
            .iter()
            .any(|point| point.label == warmup_label)
    );
}

#[test]
fn disabling_monitoring_rearms_the_next_unix_rate_sample() {
    let mut state = SessionMonitoringState::new(true);
    state.apply_snapshot(monitor_snapshot(0.0, 0.0, 0.0));
    state.apply_snapshot(monitor_snapshot(25.0, 10.0, 5.0));

    state.set_enabled(false);
    state.set_enabled(true);
    state.apply_snapshot(monitor_snapshot(0.0, 0.0, 0.0));

    assert_eq!(state.cpu_history.len(), 1);
    assert_eq!(state.network_rx_history.len(), 1);
    assert!(!state.cpu_sample_ready);
    assert!(!state.network_sample_ready);
}

#[test]
fn retrying_enabled_monitoring_keeps_the_error_until_a_new_snapshot_arrives() {
    let mut state = SessionMonitoringState::new(true);
    state.report_error("failed".into());

    state.set_enabled(true);
    assert_eq!(state.last_error.as_deref(), Some("failed"));

    state.apply_snapshot(monitor_snapshot(10.0, 1.0, 1.0));
    assert_eq!(state.last_error, None);
}

#[test]
fn terminal_disconnected_preserves_history_and_is_read_only() {
    let session = session_state(
        SessionPurpose::Terminal,
        SessionConnectionState::Disconnected,
    );

    assert!(session.preserves_terminal_history());
    assert!(session.is_terminal_read_only());
    assert!(!session.uses_blocking_placeholder());
}

#[test]
fn terminal_failed_preserves_history_and_is_read_only() {
    let session = session_state(
        SessionPurpose::Terminal,
        SessionConnectionState::Failed {
            error: "boom".to_string(),
            status: Some(SessionFailureStatus::Closed),
        },
    );

    assert!(session.preserves_terminal_history());
    assert!(session.is_terminal_read_only());
    assert!(!session.uses_blocking_placeholder());
}

#[test]
fn terminal_exited_preserves_history_and_is_read_only() {
    let session = session_state(
        SessionPurpose::Terminal,
        SessionConnectionState::Exited {
            exit_code: 0,
            signal: None,
        },
    );

    assert!(session.preserves_terminal_history());
    assert!(session.is_terminal_read_only());
    assert!(!session.uses_blocking_placeholder());
}

#[test]
fn connecting_session_keeps_blocking_placeholder() {
    let session = session_state(SessionPurpose::Terminal, SessionConnectionState::Connecting);

    assert!(!session.preserves_terminal_history());
    assert!(!session.is_terminal_read_only());
    assert!(session.uses_blocking_placeholder());
}

#[test]
fn non_terminal_disconnected_session_does_not_preserve_history() {
    let session = session_state(
        SessionPurpose::PortForwarding,
        SessionConnectionState::Disconnected,
    );

    assert!(!session.preserves_terminal_history());
    assert!(!session.is_terminal_read_only());
    assert!(session.uses_blocking_placeholder());
}

#[test]
fn changing_connection_state_resets_hidden_history_popup() {
    let mut session = session_state(
        SessionPurpose::Terminal,
        SessionConnectionState::Disconnected,
    );
    session.hide_preserved_history_popup();

    session.set_connection_state(SessionConnectionState::Connecting);

    assert!(!session.preserved_history_popup_hidden());
    assert!(session.uses_blocking_placeholder());
}
