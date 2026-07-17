use gpui::Context;

use super::{SessionConnectionState, SessionController, SessionPurpose};
use crate::ui::{
    i18n,
    shell::{SessionProfile, TabId},
};

impl SessionController {
    pub(in crate::ui::shell) fn tab_purpose(&self, tab_id: TabId) -> Option<SessionPurpose> {
        self.tab(tab_id).map(|session| session.purpose)
    }

    pub(in crate::ui::shell) fn resolved_profile_for_tab(
        &self,
        tab_id: TabId,
    ) -> Option<SessionProfile> {
        let session = self.tab(tab_id)?;
        let profile_id = session.profile_id.clone();
        let pending_profile = session.pending_profile.clone();
        drop(session);

        self.profiles
            .borrow()
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
            .or(pending_profile)
    }

    pub(in crate::ui::shell) fn reopen_profile_for_tab(
        &self,
        tab_id: TabId,
    ) -> Option<SessionProfile> {
        let session = self.tab(tab_id)?;
        let profile_id = session.profile_id.clone();
        let pending_profile = session.pending_profile.clone();
        drop(session);

        pending_profile.or_else(|| {
            self.profiles
                .borrow()
                .iter()
                .find(|profile| profile.id == profile_id)
                .cloned()
        })
    }

    pub(in crate::ui::shell) fn clear_tab_activity(&self, tab_id: TabId) -> bool {
        let Some(mut session) = self.tab_mut(tab_id) else {
            return false;
        };
        let changed = session.has_activity;
        session.has_activity = false;
        changed
    }

    pub(in crate::ui::shell) fn retire_tab_resources(
        &self,
        tab_id: TabId,
    ) -> Option<(SessionPurpose, String)> {
        let session = self.tab(tab_id)?;
        let purpose = session.purpose;
        let profile_id = session.profile_id.clone();
        let commands = session.commands.clone();
        drop(session);

        self.terminal_port().close_session(tab_id);
        if let Some(commands) = commands
            && let Err(error) = commands.close()
        {
            log::debug!("failed to close tab {tab_id} cleanly: {error:?}");
        }
        Some((purpose, profile_id))
    }

    pub(in crate::ui::shell) fn schedule_reconnect(
        &mut self,
        tab_id: TabId,
        error: String,
        cx: &mut Context<Self>,
    ) {
        const MAX_RECONNECT_ATTEMPTS: u32 = 10;
        const RECONNECT_DELAYS_SECS: &[u64] = &[1, 2, 4, 8, 16, 30];

        let Some(next_attempt) = self
            .tab(tab_id)
            .map(|session| session.reconnect_attempt.saturating_add(1))
        else {
            return;
        };

        if next_attempt > MAX_RECONNECT_ATTEMPTS {
            if let Some(mut session) = self.tab_mut(tab_id) {
                session.set_connection_state(SessionConnectionState::Failed {
                    error,
                    status: None,
                });
                session.reconnect_attempt = 0;
            }
            cx.notify();
            return;
        }

        if let Some(mut session) = self.tab_mut(tab_id) {
            session.reconnect_attempt = next_attempt;
            session.set_connection_state(SessionConnectionState::Reconnecting {
                error: error.clone(),
                attempt: next_attempt,
            });
        }

        let delay_secs = RECONNECT_DELAYS_SECS
            .get(next_attempt.saturating_sub(1) as usize)
            .copied()
            .unwrap_or(30);
        let delay = std::time::Duration::from_secs(delay_secs);
        let reconnect_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            if this
                .update(cx, |this, cx| {
                    let Some(profile_id) =
                        this.tab(tab_id).map(|session| session.profile_id.clone())
                    else {
                        return;
                    };
                    let profile = this
                        .profiles
                        .borrow()
                        .iter()
                        .find(|profile| profile.id == profile_id)
                        .cloned();
                    if let Some(mut session) = this.tab_mut(tab_id) {
                        if let Some(profile) = profile {
                            session.commands = None;
                            session.pending_profile = Some(profile);
                            session.set_connection_state(SessionConnectionState::Connecting);
                            session.terminal.push_text(&i18n::string_args(
                                "session.terminal.reconnecting_attempt_marker",
                                &[("attempt", &next_attempt.to_string())],
                            ));
                        } else {
                            session.set_connection_state(SessionConnectionState::Failed {
                                error: error.clone(),
                                status: None,
                            });
                            session.reconnect_attempt = 0;
                        }
                        session.reconnect_task = None;
                    }
                    cx.notify();
                })
                .is_err()
            {
                log::debug!("reconnect task: SessionController entity was dropped");
            }
        });

        if let Some(mut session) = self.tab_mut(tab_id) {
            session.reconnect_task = Some(reconnect_task);
        }
        cx.notify();
    }
}
