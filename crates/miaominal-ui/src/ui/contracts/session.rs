//! Immutable Session results and runtime ports exposed to desktop consumers.
use crate::ui::contracts::workspace::{TabId, TabKindTag, TabPlacement};
pub(in crate::ui) use crate::ui::features::session::{
    SessionQueryPort, SessionTerminalPort, SessionTerminalTarget, TerminalLease,
    TerminalLeaseError, TerminalLeaseGrant,
};
use miaominal_core::profile::SessionProfile;
#[derive(Clone, Debug)]
pub(in crate::ui) enum SessionNotificationTone {
    Success,
    Error,
}

#[derive(Clone, Debug)]
pub(in crate::ui) struct SessionNotificationRequest {
    pub(in crate::ui) tone: SessionNotificationTone,
    pub(in crate::ui) title: String,
    pub(in crate::ui) message: String,
    pub(in crate::ui) id: String,
}

#[derive(Clone, Debug)]
pub(in crate::ui) enum SessionEventTabRemoval {
    ConnectionTest { status_message: String },
}

#[derive(Clone, Debug)]
pub(in crate::ui) struct SessionEventOutcome {
    pub(in crate::ui) connected_profile: Option<(String, u64)>,
    pub(in crate::ui) tab_status: Option<String>,
    pub(in crate::ui) clipboard_writes: Vec<String>,
    pub(in crate::ui) notification: Option<SessionNotificationRequest>,
    pub(in crate::ui) removal: Option<SessionEventTabRemoval>,
    pub(in crate::ui) schedule_reconnect_error: Option<String>,
    pub(in crate::ui) refresh_monitoring_profile: Option<String>,
    pub(in crate::ui) should_notify: bool,
}

pub(in crate::ui) struct ClosedSessionTabState {
    pub(in crate::ui) tab_id: TabId,
    pub(in crate::ui) profile: SessionProfile,
    pub(in crate::ui) hidden_from_topbar: bool,
}

/// Immutable Session description offered to Workspace when a live payload opens.
/// Workspace owns registration, placement changes and subsequent title/status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionTabDescriptor {
    id: TabId,
    title: String,
    status: String,
    kind: TabKindTag,
    placement: TabPlacement,
}

impl SessionTabDescriptor {
    pub(crate) fn new(
        id: TabId,
        title: String,
        status: String,
        kind: TabKindTag,
        placement: TabPlacement,
    ) -> Self {
        Self {
            id,
            title,
            status,
            kind,
            placement,
        }
    }
    pub(crate) fn id(&self) -> TabId {
        self.id
    }
    pub(crate) fn title(&self) -> &str {
        &self.title
    }
    pub(crate) fn status(&self) -> &str {
        &self.status
    }
    pub(crate) fn kind(&self) -> TabKindTag {
        self.kind
    }
    pub(crate) fn placement(&self) -> TabPlacement {
        self.placement
    }
    pub(crate) fn with_status(mut self, status: String) -> Self {
        self.status = status;
        self
    }
    pub(crate) fn with_placement(mut self, placement: TabPlacement) -> Self {
        self.placement = placement;
        self
    }
}
