//! Opaque SFTP handoff and immutable window presentation data.
use crate::ui::contracts::workspace::TabId;
use std::any::Any;

pub(in crate::ui) struct TransferredSftpTab {
    tab_id: TabId,
    payload: Box<dyn Any>,
}
impl TransferredSftpTab {
    pub(in crate::ui) fn new<T: 'static>(tab_id: TabId, payload: T) -> Self {
        Self {
            tab_id,
            payload: Box::new(payload),
        }
    }
    pub(in crate::ui) fn tab_id(&self) -> TabId {
        self.tab_id
    }
    pub(in crate::ui) fn into_payload<T: 'static>(self) -> T {
        *self
            .payload
            .downcast::<T>()
            .expect("SFTP handoff must return to the SFTP owner")
    }
}

pub(in crate::ui) struct TransferredSftpWindowUi {
    pub(in crate::ui) tab_id: TabId,
    pub(in crate::ui) local_path_value: String,
    pub(in crate::ui) remote_path_value: String,
    pub(in crate::ui) local_path_editing: bool,
    pub(in crate::ui) remote_path_editing: bool,
    pub(in crate::ui) remote_path_submit_pending: bool,
    pub(in crate::ui) prompt_value: String,
    pub(in crate::ui) inline_rename_value: String,
}
