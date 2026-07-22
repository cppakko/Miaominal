use super::session::{SftpDirectoryRequestId, SftpEvent, SftpEventSender, send_event};
use anyhow::{Context, Result};
use futures::future::BoxFuture;
use miaominal_core::sftp::{SftpEntry, SftpEntryKind};
use russh_sftp::{
    client::{SftpSession, error::Error as SftpClientError},
    protocol::{FileType, StatusCode},
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteDirectoryEntry {
    path: String,
    file_type: FileType,
}

trait RemoteFileOperations {
    fn read_directory(&self, path: String) -> BoxFuture<'_, Result<Vec<RemoteDirectoryEntry>>>;
    fn remove_file(&self, path: String) -> BoxFuture<'_, Result<()>>;
    fn remove_directory(&self, path: String) -> BoxFuture<'_, Result<()>>;
}

impl RemoteFileOperations for SftpSession {
    fn read_directory(&self, path: String) -> BoxFuture<'_, Result<Vec<RemoteDirectoryEntry>>> {
        Box::pin(async move {
            Ok(self
                .read_dir(path)
                .await?
                .map(|entry| RemoteDirectoryEntry {
                    path: entry.path(),
                    file_type: entry.file_type(),
                })
                .collect())
        })
    }

    fn remove_file(&self, path: String) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move { Ok(SftpSession::remove_file(self, path).await?) })
    }

    fn remove_directory(&self, path: String) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move { Ok(SftpSession::remove_dir(self, path).await?) })
    }
}

fn is_missing_remote_path(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<SftpClientError>(),
        Some(SftpClientError::Status(status)) if status.status_code == StatusCode::NoSuchFile
    )
}

fn remove_remote_directory_tree<'a, O>(operations: &'a O, path: String) -> BoxFuture<'a, Result<()>>
where
    O: RemoteFileOperations + Sync + ?Sized,
{
    Box::pin(async move {
        let entries = match operations.read_directory(path.clone()).await {
            Ok(entries) => entries,
            Err(error) if is_missing_remote_path(&error) => return Ok(()),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to read remote directory {path} during recursive deletion")
                });
            }
        };

        let mut child_directories = Vec::new();
        for entry in entries {
            match entry.file_type {
                FileType::Dir => child_directories.push(entry.path),
                FileType::File | FileType::Symlink | FileType::Other => {
                    match operations.remove_file(entry.path.clone()).await {
                        Ok(()) => {}
                        Err(error) if is_missing_remote_path(&error) => {}
                        Err(error) => {
                            return Err(error).with_context(|| {
                                format!(
                                    "failed to remove remote entry {} while deleting directory {path}",
                                    entry.path
                                )
                            });
                        }
                    }
                }
            }
        }

        for child_path in child_directories {
            remove_remote_directory_tree(operations, child_path.clone())
                .await
                .with_context(|| {
                    format!("failed to recursively delete child directory {child_path} from {path}")
                })?;
        }

        match operations.remove_directory(path.clone()).await {
            Ok(()) => {}
            Err(error) if is_missing_remote_path(&error) => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to remove remote directory {path} after deleting its contents")
                });
            }
        }

        Ok(())
    })
}

pub(super) async fn remove_directory_recursive(sftp: &SftpSession, path: &str) -> Result<()> {
    remove_remote_directory_tree(sftp, path.to_string()).await
}

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

pub(super) async fn emit_directory_listing(
    event_sender: &SftpEventSender,
    request_id: Option<SftpDirectoryRequestId>,
    path: String,
    entries: Vec<SftpEntry>,
) -> Result<()> {
    send_event(
        event_sender,
        SftpEvent::DirectoryListing {
            request_id,
            path,
            entries,
        },
    )
    .await
}

pub(super) async fn emit_subdirectory_listing(
    event_sender: &SftpEventSender,
    parent_path: String,
    entries: Vec<SftpEntry>,
) -> Result<()> {
    send_event(
        event_sender,
        SftpEvent::SubdirectoryListing {
            parent_path,
            entries,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh_sftp::protocol::Status;
    use std::{collections::HashMap, sync::Mutex};

    #[derive(Default)]
    struct MockRemoteFileOperations {
        directories: HashMap<String, Vec<RemoteDirectoryEntry>>,
        failures: HashMap<(String, String), StatusCode>,
        operations: Mutex<Vec<String>>,
    }

    impl MockRemoteFileOperations {
        fn with_directory(mut self, path: &str, entries: Vec<(&str, FileType)>) -> Self {
            self.directories.insert(
                path.to_string(),
                entries
                    .into_iter()
                    .map(|(path, file_type)| RemoteDirectoryEntry {
                        path: path.to_string(),
                        file_type,
                    })
                    .collect(),
            );
            self
        }

        fn with_failure(mut self, operation: &str, path: &str, status: StatusCode) -> Self {
            self.failures
                .insert((operation.to_string(), path.to_string()), status);
            self
        }

        fn result_for(&self, operation: &str, path: &str) -> Result<()> {
            self.operations
                .lock()
                .expect("operation log lock")
                .push(format!("{operation}:{path}"));
            match self
                .failures
                .get(&(operation.to_string(), path.to_string()))
            {
                Some(status_code) => Err(SftpClientError::Status(Status {
                    id: 1,
                    status_code: *status_code,
                    error_message: "injected failure".into(),
                    language_tag: "en".into(),
                })
                .into()),
                None => Ok(()),
            }
        }

        fn operation_log(&self) -> Vec<String> {
            self.operations.lock().expect("operation log lock").clone()
        }
    }

    impl RemoteFileOperations for MockRemoteFileOperations {
        fn read_directory(&self, path: String) -> BoxFuture<'_, Result<Vec<RemoteDirectoryEntry>>> {
            Box::pin(async move {
                self.result_for("read", &path)?;
                Ok(self.directories.get(&path).cloned().unwrap_or_default())
            })
        }

        fn remove_file(&self, path: String) -> BoxFuture<'_, Result<()>> {
            Box::pin(async move { self.result_for("file", &path) })
        }

        fn remove_directory(&self, path: String) -> BoxFuture<'_, Result<()>> {
            Box::pin(async move { self.result_for("dir", &path) })
        }
    }

    #[test]
    fn recursive_delete_removes_nested_and_hidden_entries_in_post_order() {
        let operations = MockRemoteFileOperations::default()
            .with_directory(
                "/root",
                vec![
                    ("/root/sub", FileType::Dir),
                    ("/root/.hidden", FileType::File),
                ],
            )
            .with_directory("/root/sub", vec![("/root/sub/child.txt", FileType::File)]);

        futures::executor::block_on(remove_remote_directory_tree(&operations, "/root".into()))
            .expect("recursive delete succeeds");

        assert_eq!(
            operations.operation_log(),
            vec![
                "read:/root",
                "file:/root/.hidden",
                "read:/root/sub",
                "file:/root/sub/child.txt",
                "dir:/root/sub",
                "dir:/root",
            ]
        );
    }

    #[test]
    fn recursive_delete_removes_symlink_without_traversing_it() {
        let operations = MockRemoteFileOperations::default()
            .with_directory(
                "/root",
                vec![
                    ("/root/link", FileType::Symlink),
                    ("/root/other", FileType::Other),
                ],
            )
            .with_directory(
                "/root/link",
                vec![("/root/link/target-child", FileType::File)],
            );

        futures::executor::block_on(remove_remote_directory_tree(&operations, "/root".into()))
            .expect("recursive delete succeeds");

        assert_eq!(
            operations.operation_log(),
            vec![
                "read:/root",
                "file:/root/link",
                "file:/root/other",
                "dir:/root",
            ]
        );
    }

    #[test]
    fn recursive_delete_tolerates_entries_removed_concurrently() {
        let operations = MockRemoteFileOperations::default()
            .with_directory(
                "/root",
                vec![
                    ("/root/gone.txt", FileType::File),
                    ("/root/gone", FileType::Dir),
                ],
            )
            .with_failure("file", "/root/gone.txt", StatusCode::NoSuchFile)
            .with_failure("read", "/root/gone", StatusCode::NoSuchFile);

        futures::executor::block_on(remove_remote_directory_tree(&operations, "/root".into()))
            .expect("missing children count as deleted");

        assert_eq!(
            operations.operation_log(),
            vec![
                "read:/root",
                "file:/root/gone.txt",
                "read:/root/gone",
                "dir:/root",
            ]
        );
    }

    #[test]
    fn recursive_delete_stops_at_first_failure_and_reports_stage_and_path() {
        let operations = MockRemoteFileOperations::default()
            .with_directory(
                "/root",
                vec![
                    ("/root/protected.txt", FileType::File),
                    ("/root/later.txt", FileType::File),
                ],
            )
            .with_failure("file", "/root/protected.txt", StatusCode::PermissionDenied);

        let error =
            futures::executor::block_on(remove_remote_directory_tree(&operations, "/root".into()))
                .expect_err("permission failure stops deletion");

        assert!(format!("{error:#}").contains(
            "failed to remove remote entry /root/protected.txt while deleting directory /root"
        ));
        assert_eq!(
            operations.operation_log(),
            vec!["read:/root", "file:/root/protected.txt"]
        );
    }

    #[test]
    fn recursive_delete_removes_empty_directory() {
        let operations = MockRemoteFileOperations::default().with_directory("/empty", vec![]);

        futures::executor::block_on(remove_remote_directory_tree(&operations, "/empty".into()))
            .expect("empty directory delete succeeds");

        assert_eq!(
            operations.operation_log(),
            vec!["read:/empty", "dir:/empty"]
        );
    }
}
