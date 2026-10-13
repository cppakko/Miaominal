use std::fmt;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct TabId(usize);

impl TabId {
    pub(crate) const fn new(raw: usize) -> Self {
        Self(raw)
    }

    pub(crate) const fn raw(self) -> usize {
        self.0
    }
}

impl fmt::Display for TabId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabKindTag {
    Hosts,
    Session,
    Sftp,
    LocalTerminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabPlacement {
    TopLevel,
    WorkspacePane { owner: TabId, pane: PaneId },
    SessionSidecar { owner: TabId },
    Background,
}

impl TabPlacement {
    pub(crate) fn owner(self) -> Option<TabId> {
        match self {
            Self::WorkspacePane { owner, .. } | Self::SessionSidecar { owner } => Some(owner),
            Self::TopLevel | Self::Background => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SplitDirection {
    Up,
    Down,
    Left,
    Right,
}

impl SplitDirection {
    pub fn axis(self) -> SplitAxis {
        match self {
            Self::Left | Self::Right => SplitAxis::Horizontal,
            Self::Up | Self::Down => SplitAxis::Vertical,
        }
    }

    pub fn places_new_before(self) -> bool {
        matches!(self, Self::Left | Self::Up)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PaneId(pub usize);

impl PaneId {
    pub fn raw(self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalWorkspaceRequest {
    OpenSearch,
    Split(SplitDirection),
    ClosePane,
}
