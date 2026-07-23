use super::*;
#[cfg(test)]
use crate::ui::i18n;
use std::time::Instant;

#[derive(Debug, Clone)]
pub(in crate::ui::shell) enum DialogOverlaySnapshot {
    HostKey {
        tab_id: TabId,
        prompt: HostKeyPrompt,
    },
    KeyboardInteractive {
        tab_id: TabId,
        challenge: KbiChallenge,
    },
    ProfileDelete(PendingProfileDeleteState),
    ProfileImportResult(PendingProfileImportResultState),
    ManagedKeyDelete(PendingManagedKeyDeleteState),
    ManagedKeyRename(PendingManagedKeyRenameState),
    KnownHostDelete(PendingKnownHostDeleteState),
    SnippetDelete(PendingSnippetDeleteState),
    PortForwardRuleDelete(PendingPortForwardRuleDeleteState),
    ChatSessionDelete(PendingChatSessionDeleteState),
    ChatSessionRename(PendingChatSessionRenameState),
    SyncDirection(PendingSyncDirectionState),
    SyncPullConfirm(PendingSyncPullConfirmState),
    LocalVaultDisableConfirm(PendingLocalVaultDisableConfirmState),
    LocalDataResetConfirm(PendingLocalDataResetConfirmState),
    LocalDataResetConfirmationPopup(PendingLocalDataResetConfirmationPopupState),
    SyncPassphraseClearConfirmPopup(PendingSyncPassphraseClearConfirmPopupState),
    SyncPassphrasePopup(PendingSyncPassphrasePopupState),
    AiProviderPopup(PendingAiProviderPopupState),
    WebSearchConfigPopup(PendingWebSearchConfigPopupState),
    SyncProviderConfigPopup(PendingSyncProviderConfigPopupState),
    LocalVaultPassphrasePopup(LocalVaultPassphrasePopupMode),
    SftpPrompt {
        tab_id: TabId,
        prompt: SftpPromptState,
    },
}

impl DialogOverlaySnapshot {
    pub(in crate::ui::shell) fn stable_key(&self) -> String {
        match self {
            Self::HostKey { .. } => "trusted-host-key".to_string(),
            Self::KeyboardInteractive { .. } => "keyboard-interactive".to_string(),
            Self::ProfileDelete(_) => "profile-delete".to_string(),
            Self::ProfileImportResult(_) => "profile-import-result".to_string(),
            Self::ManagedKeyDelete(_) => "managed-key-delete".to_string(),
            Self::ManagedKeyRename(_) => "managed-key-rename".to_string(),
            Self::KnownHostDelete(_) => "known-host-delete".to_string(),
            Self::SnippetDelete(_) => "snippet-delete".to_string(),
            Self::PortForwardRuleDelete(_) => "port-forward-rule-delete".to_string(),
            Self::ChatSessionDelete(_) => "chat-session-delete".to_string(),
            Self::ChatSessionRename(_) => "chat-session-rename".to_string(),
            Self::SyncDirection(_) => "sync-direction".to_string(),
            Self::SyncPullConfirm(_) => "sync-pull-confirm".to_string(),
            Self::LocalVaultDisableConfirm(_) => "local-vault-disable-confirm".to_string(),
            Self::LocalDataResetConfirm(_) => "local-data-reset-confirm".to_string(),
            Self::LocalDataResetConfirmationPopup(_) => "local-data-reset-confirmation".to_string(),
            Self::SyncPassphraseClearConfirmPopup(_) => "sync-passphrase-clear-confirm".to_string(),
            Self::SyncPassphrasePopup(_) => "sync-passphrase".to_string(),
            Self::AiProviderPopup(_) => "ai-provider".to_string(),
            Self::WebSearchConfigPopup(_) => "web-search-config".to_string(),
            Self::SyncProviderConfigPopup(popup) => match popup.provider {
                SyncProvider::None => "sync-provider-none".to_string(),
                SyncProvider::GithubGist => "sync-provider-gist".to_string(),
                SyncProvider::WebDav => "sync-provider-webdav".to_string(),
            },
            Self::LocalVaultPassphrasePopup(_) => "local-vault-passphrase".to_string(),
            Self::SftpPrompt { tab_id, .. } => format!("sftp-prompt-{tab_id}"),
        }
    }
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct ExitingDialogState {
    pub(in crate::ui::shell) snapshot: DialogOverlaySnapshot,
    pub(in crate::ui::shell) started_at: Instant,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(in crate::ui::shell) struct DraggedTab {
    pub(in crate::ui::shell) source_tab_id: TabId,
    pub(in crate::ui::shell) source_index: usize,
    pub(in crate::ui::shell) source_pane_id: PaneId,
    pub(in crate::ui::shell) is_active: bool,
    pub(in crate::ui::shell) title: String,
    pub(in crate::ui::shell) status_color: Option<u32>,
}

impl Render for DraggedTab {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let roles = miaominal_settings::current_theme().material.roles;

        h_flex()
            .w(px(TOPBAR_TAB_WIDTH))
            .h(px(36.0))
            .px_3()
            .gap_2()
            .items_center()
            .rounded(px(14.0))
            .bg(rgb(if self.is_active {
                roles.secondary_container
            } else {
                roles.surface_container_high
            }))
            .opacity(0.92)
            .text_size(miaominal_settings::FontSize::Body.scaled())
            .text_color(rgb(if self.is_active {
                roles.on_secondary_container
            } else {
                roles.on_surface_variant
            }))
            .child(
                h_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap_2()
                    .items_center()
                    .when_some(self.status_color, |this, color| {
                        this.child(div().size(px(7.0)).rounded(px(999.0)).bg(rgb(color)))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .child(self.title.clone()),
                    ),
            )
    }
}

#[derive(Default)]
pub(in crate::ui::shell) struct ShellState {
    pub(in crate::ui::shell) sidebar_section: SidebarSection,
    pub(in crate::ui::shell) page_editor_sidebar_transition: Option<PageEditorSidebarTransition>,
    pub(in crate::ui::shell) visible_page_editor_sidebar: Option<PageEditorSidebarKind>,
}

#[cfg(test)]
mod tests {
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
            pending_keyboard_interactive: None,
            reconnect_task: None,
            reconnect_attempt: 0,
            has_activity: false,
            monitoring: SessionMonitoringState::new(false),
            purpose,
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
    fn split_message_into_blocks_skips_blank_segments() {
        assert!(split_message_into_blocks("").is_empty());
        assert!(split_message_into_blocks("\n\n  \n\t\n").is_empty());

        assert_eq!(
            split_message_into_blocks("hello\n\n\nworld"),
            vec!["hello".to_string(), "world".to_string()]
        );
    }

    #[test]
    fn split_message_into_blocks_keeps_code_fences_together() {
        assert_eq!(
            split_message_into_blocks("before\n\n```rust\nfn main() {}\n```\n\nafter"),
            vec![
                "before".to_string(),
                "```rust\nfn main() {}\n```".to_string(),
                "after".to_string()
            ]
        );
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

    #[test]
    fn pending_tool_call_counts_as_active_and_can_be_rejected() {
        let mut state = SessionAgentState::default();
        state.push_tool_call(
            "tool-1".to_string(),
            "read".to_string(),
            "{\"path\":\"Cargo.toml\"}".to_string(),
            SessionAgentToolStatus::Pending,
        );

        assert!(state.has_active_tool_call());
        let stopped_by_user = i18n::string("workspace.panel.agent.messages.stopped_by_user");
        assert!(state.reject_active_tool_calls(&stopped_by_user));

        let tool = state.tool_call("tool-1").expect("tool should exist");
        assert_eq!(tool.status, SessionAgentToolStatus::Rejected);
        assert_eq!(
            tool.confirmation_note.as_deref(),
            Some(stopped_by_user.as_str())
        );
        assert!(!state.has_active_tool_call());
    }

    #[test]
    fn structured_tool_cancellation_is_rejected_with_the_stop_reason() {
        let mut state = SessionAgentState::default();
        state.push_tool_call(
            "tool-1".to_string(),
            "run_shell".to_string(),
            "{}".to_string(),
            SessionAgentToolStatus::InProgress,
        );
        let stopped_by_user = i18n::string("workspace.panel.agent.messages.stopped_by_user");

        state.reject_tool_call_with_message("tool-1", stopped_by_user.clone());

        let tool = state.tool_call("tool-1").expect("tool should exist");
        assert_eq!(tool.status, SessionAgentToolStatus::Rejected);
        assert_eq!(
            tool.confirmation_note.as_deref(),
            Some(stopped_by_user.as_str())
        );
    }

    #[test]
    fn unacknowledged_tool_stop_is_failed_with_unknown_status() {
        let mut state = SessionAgentState::default();
        state.push_tool_call(
            "tool-1".to_string(),
            "run_shell".to_string(),
            "{}".to_string(),
            SessionAgentToolStatus::InProgress,
        );
        let unknown = i18n::string("workspace.panel.agent.messages.tool_stop_unconfirmed");

        assert!(state.fail_active_tool_calls(&unknown));

        let tool = state.tool_call("tool-1").expect("tool should exist");
        assert_eq!(tool.status, SessionAgentToolStatus::Failed);
        assert_eq!(tool.confirmation_note.as_deref(), Some(unknown.as_str()));
    }

    #[test]
    fn failed_tool_start_no_longer_keeps_the_agent_busy() {
        let mut state = SessionAgentState::default();
        state.push_tool_call(
            "tool-1".to_string(),
            "run_shell".to_string(),
            "{\"command\":\"cargo check\"}".to_string(),
            SessionAgentToolStatus::InProgress,
        );

        state.fail_tool_call("tool-1", "execution context disappeared".to_string());

        let tool = state.tool_call("tool-1").expect("tool should exist");
        assert_eq!(tool.status, SessionAgentToolStatus::Failed);
        assert!(!state.has_active_tool_call());
        assert!(!state.is_busy());
    }

    #[test]
    fn session_agent_running_tool_execution_mode_uses_captured_context_over_current_ui_mode() {
        let mut state = SessionAgentState::default();
        state.exec_mode = AgentExecMode::Pty;
        state.active_exec_context = Some(SessionAgentExecutionContext {
            profile_id: "profile-a".to_string(),
            exec_mode: AgentExecMode::ExecChannel,
            terminal_tab_id: None,
        });

        assert_eq!(
            state.execution_mode_for_running_tools(),
            AgentExecMode::ExecChannel
        );

        state.exec_mode = AgentExecMode::ExecChannel;
        state.active_exec_context = Some(SessionAgentExecutionContext {
            profile_id: "profile-a".to_string(),
            exec_mode: AgentExecMode::Pty,
            terminal_tab_id: Some(TabId::new(42)),
        });

        assert_eq!(state.execution_mode_for_running_tools(), AgentExecMode::Pty);
    }

    #[test]
    fn session_agent_running_tool_execution_mode_falls_back_to_current_ui_mode_without_context() {
        let mut state = SessionAgentState::default();
        state.exec_mode = AgentExecMode::Pty;

        assert_eq!(state.execution_mode_for_running_tools(), AgentExecMode::Pty);

        state.exec_mode = AgentExecMode::ExecChannel;

        assert_eq!(
            state.execution_mode_for_running_tools(),
            AgentExecMode::ExecChannel
        );
    }

    #[test]
    fn waiting_confirmation_tool_call_is_detected_separately() {
        let mut state = SessionAgentState::default();
        state.push_tool_call(
            "tool-1".to_string(),
            "run_shell".to_string(),
            "{\"command\":\"cargo test\"}".to_string(),
            SessionAgentToolStatus::InProgress,
        );

        assert!(!state.has_tool_call_waiting_for_confirmation());

        state.require_tool_call_confirmation("tool-1", "approval required".to_string());

        assert!(state.has_active_tool_call());
        assert!(state.has_tool_call_waiting_for_confirmation());
    }

    #[test]
    fn realtime_session_agent_messages_receive_enter_motion_keys() {
        let mut state = SessionAgentState::default();

        state.push_message_with_enter_motion(SessionAgentMessage::user("hello"));
        state.append_assistant_delta("hi");
        state.append_thinking_delta("checking");
        state.push_tool_call(
            "tool-1".to_string(),
            "read".to_string(),
            "{\"path\":\"Cargo.toml\"}".to_string(),
            SessionAgentToolStatus::InProgress,
        );

        let keys = state
            .messages
            .iter()
            .map(|message| message.motion.enter_key)
            .collect::<Vec<_>>();
        assert_eq!(keys, vec![Some(1), Some(2), Some(3), Some(4)]);
    }

    #[test]
    fn session_agent_streaming_deltas_reuse_existing_message_enter_motion_key() {
        let mut state = SessionAgentState::default();

        state.append_assistant_delta("hello");
        let assistant_key = state.messages[0].motion.enter_key;
        state.append_assistant_delta(" world");

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].content, "hello world");
        assert_eq!(state.messages[0].motion.enter_key, assistant_key);

        state.append_thinking_delta("reason");
        let thinking_key = state.messages[1].motion.enter_key;
        state.append_thinking_delta("ing");

        assert_eq!(state.messages.len(), 2);
        assert_eq!(state.messages[1].content, "reasoning");
        assert_eq!(state.messages[1].motion.enter_key, thinking_key);
    }

    #[test]
    fn stopped_turn_replaces_empty_assistant_placeholder() {
        let mut state = SessionAgentState::default();
        state.messages.push(SessionAgentMessage::user("hello"));
        state.messages.push(SessionAgentMessage::assistant_raw(""));

        state.finish_stopped_turn();

        assert_eq!(state.messages.len(), 2);
        assert_eq!(state.messages[1].role, SessionAgentMessageRole::Assistant);
        assert_eq!(
            state.messages[1].content,
            i18n::string("workspace.panel.agent.messages.stopped_by_user")
        );
    }
}
