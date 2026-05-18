use super::session::SftpEvent;
use crate::domain::sftp::{SftpEntry, SftpEntryKind};
use anyhow::{Context, Result, anyhow};
use futures::channel::mpsc::UnboundedSender as FuturesUnboundedSender;
use russh_sftp::{client::SftpSession, protocol::FileType};

pub(super) async fn canonical_remote_path(sftp: &SftpSession, path: &str) -> Result<String> {
    let path = if path.trim().is_empty() { "." } else { path };
    sftp.canonicalize(path)
        .await
        .with_context(|| format!("failed to canonicalize remote path {path}"))
}

pub(super) async fn list_directory_entries(
    sftp: &SftpSession,
    path: &str,
) -> Result<Vec<SftpEntry>> {
    let mut entries = Vec::new();
    for entry in sftp
        .read_dir(path)
        .await
        .with_context(|| format!("failed to read remote directory {path}"))?
    {
        let filename = entry.file_name();
        let metadata = entry.metadata();
        let kind = match entry.file_type() {
            FileType::Dir => SftpEntryKind::Directory,
            FileType::File => SftpEntryKind::File,
            FileType::Symlink => SftpEntryKind::Symlink,
            FileType::Other => SftpEntryKind::Other,
        };
        let modified = metadata.modified().ok();
        let attributes = metadata
            .permissions
            .map(|_| metadata.permissions().to_string());
        let owner = match (metadata.uid, metadata.gid) {
            (Some(uid), Some(gid)) => Some(format!("{uid}:{gid}")),
            (Some(uid), None) => Some(uid.to_string()),
            (None, Some(gid)) => Some(gid.to_string()),
            (None, None) => None,
        };

        entries.push(SftpEntry {
            path: join_remote_path(path, &filename),
            filename,
            kind,
            size: metadata.size,
            modified,
            attributes,
            owner,
        });
    }

    entries.sort_by(|left, right| {
        let left_rank = matches!(left.kind, SftpEntryKind::Directory) as u8;
        let right_rank = matches!(right.kind, SftpEntryKind::Directory) as u8;
        right_rank.cmp(&left_rank).then_with(|| {
            left.filename
                .to_lowercase()
                .cmp(&right.filename.to_lowercase())
        })
    });

    Ok(entries)
}

pub(super) fn join_remote_path(base: &str, filename: &str) -> String {
    if base.is_empty() || base == "." {
        return filename.to_string();
    }

    if base == "/" {
        return format!("/{}", filename.trim_start_matches('/'));
    }

    format!("{}/{}", base.trim_end_matches('/'), filename)
}

pub(super) fn emit_directory_listing(
    event_sender: &FuturesUnboundedSender<SftpEvent>,
    path: String,
    entries: Vec<SftpEntry>,
) -> Result<()> {
    if event_sender
        .unbounded_send(SftpEvent::DirectoryListing { path, entries })
        .is_err()
    {
        return Err(anyhow!("SFTP event receiver is closed"));
    }

    Ok(())
}

pub(super) fn emit_subdirectory_listing(
    event_sender: &FuturesUnboundedSender<SftpEvent>,
    parent_path: String,
    entries: Vec<SftpEntry>,
) -> Result<()> {
    if event_sender
        .unbounded_send(SftpEvent::SubdirectoryListing {
            parent_path,
            entries,
        })
        .is_err()
    {
        return Err(anyhow!("SFTP event receiver is closed"));
    }

    Ok(())
}
