use crate::ui::contracts::navigation::SidebarSection;
use crate::ui::contracts::workspace::TabId;
use std::time::{Duration, Instant};
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrimaryViewKind {
    Sidebar(SidebarSection),
    Terminal(TabId),
    Sftp(TabId),
}

#[derive(Clone, Copy)]
pub(crate) struct PrimaryViewTransition {
    pub(crate) from: PrimaryViewKind,
    pub(crate) to: PrimaryViewKind,
    pub(crate) started_at: Instant,
    pub(crate) duration: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TopbarTabVisualKind {
    Hosts,
    Session,
    Sftp,
    LocalTerminal,
}

#[derive(Clone, Debug)]
pub(crate) struct TopbarTabSnapshot {
    pub(crate) tab_id: TabId,
    pub(crate) visible_index: usize,
    pub(crate) title: String,
    pub(crate) kind: TopbarTabVisualKind,
    pub(crate) status_color: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TopbarTabEnterTransition {
    pub(crate) tab_id: TabId,
    pub(crate) started_at: Instant,
    pub(crate) duration: Duration,
}

#[derive(Clone, Debug)]
pub(crate) struct TopbarTabExitTransition {
    pub(crate) snapshot: TopbarTabSnapshot,
    pub(crate) started_at: Instant,
    pub(crate) duration: Duration,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TopbarActiveTabTransition {
    pub(crate) from_tab_id: Option<TabId>,
    pub(crate) to_tab_id: Option<TabId>,
    pub(crate) started_at: Instant,
    pub(crate) duration: Duration,
}
