use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, bail};
use russh_sftp::{
    client::{SftpSession, error::Error as SftpClientError},
    protocol::{FileType, OpenFlags, StatusCode},
};
use tokio::{io::AsyncWriteExt, sync::watch};

use super::paths::{canonical_remote_path, remove_directory_recursive};
use crate::validate_transfer_name;

static COPY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn join(parent: &str, name: &str) -> String {
    format!("{}/{name}", parent.trim_end_matches('/'))
}

async fn require_absent(sftp: &SftpSession, path: &str) -> Result<()> {
    match sftp.symlink_metadata(path).await {
        Err(SftpClientError::Status(status)) if status.status_code == StatusCode::NoSuchFile => {
            Ok(())
        }
        Err(error) => Err(error.into()),
        Ok(_) => bail!("destination already exists: {path}"),
    }
}

/// SFTP-only copying works on servers without shell access or copy extensions.
/// Install the completed entry by rename so failures never leave a partial target.
pub(super) async fn paste_remote(
    sftp: &SftpSession,
    from: &str,
    to: &str,
    moving: bool,
    mut cancelled: watch::Receiver<bool>,
) -> Result<()> {
    let (parent, name) = to.rsplit_once('/').unwrap_or((".", to));
    validate_transfer_name(name)?;
    let parent = canonical_remote_path(sftp, if parent.is_empty() { "/" } else { parent }).await?;
    let target = join(&parent, name);
    let kind = sftp.symlink_metadata(from).await?.file_type();
    if kind == FileType::Dir {
        let source = canonical_remote_path(sftp, from).await?;
        if parent == source || parent.starts_with(&format!("{}/", source.trim_end_matches('/'))) {
            bail!("cannot paste a directory inside itself");
        }
    }
    require_absent(sftp, &target).await?;
    if *cancelled.borrow() {
        bail!("paste cancelled");
    }
    if moving {
        return sftp
            .rename(from, target)
            .await
            .context("failed to move remote entry");
    }

    let staging = join(
        &parent,
        &format!(
            ".miaominal-copy-{}-{}",
            std::process::id(),
            COPY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ),
    );
    // Allocate exclusively before cleanup becomes responsible for this entry.
    match kind {
        FileType::Dir => sftp.create_dir(&staging).await?,
        FileType::File => {
            sftp.open_with_flags(
                &staging,
                OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
            )
            .await?
            .shutdown()
            .await?;
        }
        FileType::Symlink => {
            sftp.symlink(&staging, sftp.read_link(from).await?).await?;
        }
        _ => bail!("unsupported remote entry type"),
    }
    let copy_result: Result<()> = tokio::select! {
        result = copy_tree(sftp, from, &staging, kind) => result,
        _ = cancelled.changed() => Err(anyhow::anyhow!("paste cancelled")),
    };
    let result = async {
        copy_result?;
        if *cancelled.borrow() {
            bail!("paste cancelled");
        }
        require_absent(sftp, &target).await?;
        sftp.rename(&staging, &target)
            .await
            .context("failed to install copied remote entry")
    }
    .await;
    if result.is_err() {
        let cleanup = if kind == FileType::Dir {
            remove_directory_recursive(sftp, &staging).await
        } else {
            sftp.remove_file(&staging).await.map_err(Into::into)
        };
        if let Err(error) = cleanup {
            log::warn!("failed to remove remote copy staging entry: {error:#}");
        }
    }
    result
}

async fn copy_tree(sftp: &SftpSession, from: &str, to: &str, kind: FileType) -> Result<()> {
    let mut pending = vec![(from.to_string(), to.to_string(), kind)];
    while let Some((source, target, kind)) = pending.pop() {
        match kind {
            FileType::Dir => {
                for entry in sftp.read_dir(&source).await? {
                    let name = entry.file_name();
                    if name == "." || name == ".." {
                        continue;
                    }
                    validate_transfer_name(&name)?;
                    let source = join(&source, &name);
                    let target = join(&target, &name);
                    let child_kind = sftp.symlink_metadata(&source).await?.file_type();
                    match child_kind {
                        FileType::Dir => sftp.create_dir(&target).await?,
                        FileType::File => {
                            sftp.open_with_flags(
                                &target,
                                OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
                            )
                            .await?
                            .shutdown()
                            .await?;
                        }
                        FileType::Symlink => {
                            sftp.symlink(&target, sftp.read_link(&source).await?)
                                .await?;
                        }
                        _ => bail!("unsupported remote entry type: {source}"),
                    }
                    pending.push((source, target, child_kind));
                }
            }
            FileType::File => {
                let mut reader = sftp.open(&source).await?;
                let mut writer = sftp.open_with_flags(&target, OpenFlags::WRITE).await?;
                tokio::io::copy(&mut reader, &mut writer).await?;
                writer.shutdown().await?;
            }
            FileType::Symlink => {}
            _ => bail!("unsupported remote entry type: {source}"),
        }
    }
    Ok(())
}
