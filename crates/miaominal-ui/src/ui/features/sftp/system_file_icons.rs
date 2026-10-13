use super::*;
use crate::ui::components::system_file_icons::{
    SystemFileIconKind, render_system_file_icon as render_shared_system_file_icon,
};

pub(super) fn render_system_file_icon(
    row: &SftpBrowserTableRow,
    fallback_icon: AppIcon,
    fallback_tint: u32,
    icon_size: Pixels,
    window: &mut Window,
    cx: &mut Context<TableState<SftpBrowserTableDelegate>>,
) -> AnyElement {
    let kind = match row.kind {
        miaominal_sftp::SftpEntryKind::File => SystemFileIconKind::File,
        miaominal_sftp::SftpEntryKind::Directory => SystemFileIconKind::Directory,
        miaominal_sftp::SftpEntryKind::Symlink => SystemFileIconKind::Symlink,
        miaominal_sftp::SftpEntryKind::Other => SystemFileIconKind::Other,
    };
    render_shared_system_file_icon(
        kind,
        row.name.as_ref(),
        row.is_directory,
        fallback_icon,
        fallback_tint,
        icon_size,
        window,
        cx,
        |table, cx| table.refresh(cx),
    )
}
