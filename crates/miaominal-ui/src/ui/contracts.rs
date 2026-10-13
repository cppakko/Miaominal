pub(in crate::ui) mod catalog;
mod forwarding;
pub(crate) mod navigation;
pub(crate) mod snippets;

pub(crate) use catalog::{
    CatalogQueryPort, CatalogReader, ProfileCommitOperation, ProfileCommitPort, ProfileCommitter,
};
pub(crate) use forwarding::{ForwardingRuntime, ForwardingRuntimePort};

pub(in crate::ui) mod commands;
pub(in crate::ui) mod security;
pub(in crate::ui) mod session;
pub(in crate::ui) mod sftp;
pub(in crate::ui) mod workspace;
pub(in crate::ui) use commands::{AppCommand, TabOpenRequest};
pub(in crate::ui) use navigation::PageEditorSidebarKind;
pub(in crate::ui) use session::{ClosedSessionTabState, SessionTabDescriptor};
pub(in crate::ui) use workspace::{PaneId, SplitAxis, TabId, TerminalWorkspaceRequest};

pub(in crate::ui) mod auto_sync;
pub(in crate::ui) mod keychain;
pub(in crate::ui) mod overlay;
pub(in crate::ui) mod preferences;
pub(in crate::ui) mod proxies;
pub(in crate::ui) mod settings;
