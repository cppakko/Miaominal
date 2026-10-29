//! Local file system abstraction for the SFTP transfer engine.
//!
//! Desktop transfers use ordinary paths. The Android client registers a
//! duplicated file descriptor for each user-selected document and performs
//! directory operations through the platform adapter; the engine only sees
//! this trait. Paths handled here are always **relative** to the transfer root
//! and are validated so a recursive transfer cannot escape the scope the user
//! chose (no `..`, no absolute paths, no separators inside entry names).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use futures::future::BoxFuture;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

use tempfile::{Builder as TempFileBuilder, TempPath};

pub type LocalFuture<'a, T> = BoxFuture<'a, Result<T>>;

/// Minimum transfer chunk used by the shared engine.
pub const LOCAL_TEMP_PREFIX: &str = ".miaominal-transfer-";
pub const LOCAL_TEMP_SUFFIX: &str = ".part";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalEntry {
    pub name: String,
    pub kind: LocalEntryKind,
    pub size: Option<u64>,
}

impl LocalEntry {
    pub fn is_dir(&self) -> bool {
        self.kind == LocalEntryKind::Directory
    }

    pub fn is_file(&self) -> bool {
        self.kind == LocalEntryKind::File
    }
}

/// Reject any relative path that could escape the transfer root.
///
/// An empty path refers to the root itself and is valid.
pub fn validate_relative_path(path: &str) -> Result<()> {
    if path.contains('\0') {
        bail!("local path contains a NUL byte");
    }
    if path.is_empty() {
        return Ok(());
    }
    if path.starts_with('/') || path.starts_with('\\') {
        bail!("local path must be relative to the transfer root: {path}");
    }
    for component in path.split(['/', '\\']) {
        match component {
            "" => bail!("local path contains an empty component: {path}"),
            "." | ".." => bail!("local path escapes the transfer root: {path}"),
            _ => {}
        }
    }
    Ok(())
}

/// Append one validated entry name to a relative transfer path.
pub fn join_local_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// Reject entry names that cannot be safely appended to a transfer path.
pub fn validate_transfer_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        bail!("entry name {name:?} is not valid for a transfer");
    }
    Ok(())
}

/// Finalize handle for one local write.
///
/// The engine streams into the writer, calls [`LocalWriteSession::commit`] on
/// success, and calls [`LocalWriteSession::discard`] on failure or cancel. A
/// platform backend may need the writer closed before the final step (for
/// example a SAF provider that only renames a document after its descriptor is
/// gone), which is why commit/discard are separate from the stream.
///
/// Both methods are idempotent: a backend must mark a successful commit so a
/// later discard cannot remove finished data, and a discard must remove only
/// the incomplete document this task created. Implementations must also clean
/// up from `Drop` when possible: an aborted transfer task drops the session
/// without running the explicit discard path.
pub trait LocalWriteFinalize: Send {
    fn commit(&mut self) -> LocalFuture<'_, ()>;
    fn discard(&mut self) -> LocalFuture<'_, ()>;
}

pub struct LocalWriteSession {
    pub writer: Box<dyn AsyncWrite + Send + Unpin>,
    finalize: Box<dyn LocalWriteFinalize>,
}

impl LocalWriteSession {
    pub fn new(
        writer: Box<dyn AsyncWrite + Send + Unpin>,
        finalize: Box<dyn LocalWriteFinalize>,
    ) -> Self {
        Self { writer, finalize }
    }

    /// Flush and close the writer, then finalize the local document.
    ///
    /// Any failure on the way to a successful commit discards the incomplete
    /// document, so a broken download never leaves staging data behind.
    pub async fn commit(self) -> Result<()> {
        let LocalWriteSession {
            mut writer,
            mut finalize,
        } = self;
        let flush_result = writer.flush().await;
        let shutdown_result = if flush_result.is_ok() {
            writer.shutdown().await
        } else {
            Ok(())
        };
        drop(writer);
        if let Err(error) = flush_result {
            let _ = finalize.discard().await;
            return Err(anyhow::Error::from(error).context("failed to flush local file"));
        }
        if let Err(error) = shutdown_result {
            let _ = finalize.discard().await;
            return Err(anyhow::Error::from(error).context("failed to close local file"));
        }
        if let Err(error) = finalize.commit().await {
            // The backend rolls back an interrupted replacement; discard makes
            // sure the incomplete document itself is gone as well.
            let _ = finalize.discard().await;
            return Err(error);
        }
        Ok(())
    }

    /// Remove the incomplete local document created by this task.
    pub async fn discard(self) {
        let LocalWriteSession {
            writer,
            mut finalize,
        } = self;
        drop(writer);
        if let Err(error) = finalize.discard().await {
            log::warn!("failed to remove incomplete local file: {error:#}");
        }
    }
}

/// Local side of one transfer (path, Android document, or SAF tree).
pub trait LocalFileSystem: Send + Sync {
    /// Human-readable root label used in events and logs.
    fn display_root(&self) -> String;

    fn inspect<'a>(&'a self, path: &'a str) -> LocalFuture<'a, LocalEntry>;

    fn list<'a>(&'a self, path: &'a str) -> LocalFuture<'a, Vec<LocalEntry>>;

    fn create_dir_all<'a>(&'a self, path: &'a str) -> LocalFuture<'a, ()>;

    fn open_read<'a>(&'a self, path: &'a str)
    -> LocalFuture<'a, Box<dyn AsyncRead + Send + Unpin>>;

    /// Open a write session for `path`, replacing any existing file only at
    /// commit time when the backend supports it.
    fn begin_write<'a>(&'a self, path: &'a str) -> LocalFuture<'a, LocalWriteSession>;
}

/// Path-based local file system (desktop and Rust tests).
pub struct PathLocalFileSystem {
    base: PathBuf,
}

impl PathLocalFileSystem {
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    pub fn base(&self) -> &Path {
        &self.base
    }

    fn resolve(&self, path: &str) -> Result<PathBuf> {
        validate_relative_path(path)?;
        Ok(if path.is_empty() {
            self.base.clone()
        } else {
            self.base.join(path)
        })
    }
}

struct PathWriteFinalize {
    // A private duplicate handle kept for the durability step; the engine's
    // writer is closed before `commit` runs.
    file: std::fs::File,
    temporary: Option<TempPath>,
    target: PathBuf,
}

impl LocalWriteFinalize for PathWriteFinalize {
    fn commit(&mut self) -> LocalFuture<'_, ()> {
        Box::pin(async move {
            let Some(temporary) = self.temporary.take() else {
                // Already finalized (or discarded): keep the operation idempotent.
                return Ok(());
            };
            let handle = tokio::fs::File::from_std(self.file.try_clone().with_context(|| {
                format!(
                    "failed to duplicate the temporary file for {}",
                    self.target.display()
                )
            })?);
            if let Err(error) = handle
                .sync_all()
                .await
                .with_context(|| format!("failed to sync temporary file {}", temporary.display()))
            {
                // `temporary` is dropped here, which removes the incomplete file.
                return Err(error);
            }
            drop(handle);
            if let Err(error) = temporary.persist(&self.target) {
                // A failed persist consumed the `TempPath`; remove the
                // leftover temporary file explicitly.
                if let Err(cleanup) = std::fs::remove_file(&error.path) {
                    log::warn!(
                        "failed to remove the temporary file {} after a failed replace: {cleanup}",
                        error.path.display()
                    );
                }
                return Err(anyhow::Error::from(error.error).context(format!(
                    "failed to atomically replace {}",
                    self.target.display()
                )));
            }
            Ok(())
        })
    }

    fn discard(&mut self) -> LocalFuture<'_, ()> {
        Box::pin(async move {
            // Dropping the retained `TempPath` removes the incomplete file.
            drop(self.temporary.take());
            Ok(())
        })
    }
}

impl LocalFileSystem for PathLocalFileSystem {
    fn display_root(&self) -> String {
        self.base.display().to_string()
    }

    fn inspect<'a>(&'a self, path: &'a str) -> LocalFuture<'a, LocalEntry> {
        Box::pin(async move {
            let resolved = self.resolve(path)?;
            let metadata = tokio::fs::symlink_metadata(&resolved)
                .await
                .with_context(|| format!("failed to read metadata for {}", resolved.display()))?;
            let file_type = metadata.file_type();
            let kind = if file_type.is_symlink() {
                LocalEntryKind::Symlink
            } else if file_type.is_dir() {
                LocalEntryKind::Directory
            } else if file_type.is_file() {
                LocalEntryKind::File
            } else {
                LocalEntryKind::Other
            };
            Ok(LocalEntry {
                name: resolved
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                kind,
                size: kind.eq(&LocalEntryKind::File).then_some(metadata.len()),
            })
        })
    }

    fn list<'a>(&'a self, path: &'a str) -> LocalFuture<'a, Vec<LocalEntry>> {
        Box::pin(async move {
            let resolved = self.resolve(path)?;
            let mut entries = Vec::new();
            let mut reader = tokio::fs::read_dir(&resolved)
                .await
                .with_context(|| format!("failed to read {}", resolved.display()))?;
            while let Some(entry) = reader
                .next_entry()
                .await
                .with_context(|| format!("failed to iterate {}", resolved.display()))?
            {
                let name = entry.file_name().to_string_lossy().into_owned();
                let metadata = entry
                    .metadata()
                    .await
                    .with_context(|| format!("failed to read metadata for {name}"))?;
                let file_type = metadata.file_type();
                let kind = if file_type.is_symlink() {
                    LocalEntryKind::Symlink
                } else if file_type.is_dir() {
                    LocalEntryKind::Directory
                } else if file_type.is_file() {
                    LocalEntryKind::File
                } else {
                    LocalEntryKind::Other
                };
                entries.push(LocalEntry {
                    name,
                    kind,
                    size: kind.eq(&LocalEntryKind::File).then_some(metadata.len()),
                });
            }
            entries.sort_by(|left, right| left.name.cmp(&right.name));
            Ok(entries)
        })
    }

    fn create_dir_all<'a>(&'a self, path: &'a str) -> LocalFuture<'a, ()> {
        Box::pin(async move {
            let resolved = self.resolve(path)?;
            if resolved.as_os_str().is_empty() {
                return Ok(());
            }
            tokio::fs::create_dir_all(&resolved)
                .await
                .with_context(|| format!("failed to create {}", resolved.display()))
        })
    }

    fn open_read<'a>(
        &'a self,
        path: &'a str,
    ) -> LocalFuture<'a, Box<dyn AsyncRead + Send + Unpin>> {
        Box::pin(async move {
            let resolved = self.resolve(path)?;
            let file = tokio::fs::File::open(&resolved)
                .await
                .with_context(|| format!("failed to open {} for reading", resolved.display()))?;
            Ok(Box::new(file) as Box<dyn AsyncRead + Send + Unpin>)
        })
    }

    fn begin_write<'a>(&'a self, path: &'a str) -> LocalFuture<'a, LocalWriteSession> {
        Box::pin(async move {
            let resolved = self.resolve(path)?;
            let parent = resolved
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.base.clone());
            tokio::fs::create_dir_all(&parent)
                .await
                .with_context(|| format!("failed to create {}", parent.display()))?;
            let temporary = TempFileBuilder::new()
                .prefix(LOCAL_TEMP_PREFIX)
                .suffix(LOCAL_TEMP_SUFFIX)
                .tempfile_in(&parent)
                .with_context(|| {
                    format!(
                        "failed to create temporary download file in {}",
                        parent.display()
                    )
                })?;
            let (file, temporary) = temporary.into_parts();
            let writer = file.try_clone().with_context(|| {
                format!(
                    "failed to duplicate temporary file for {}",
                    resolved.display()
                )
            })?;
            Ok(LocalWriteSession::new(
                Box::new(tokio::fs::File::from_std(writer)),
                Box::new(PathWriteFinalize {
                    file,
                    temporary: Some(temporary),
                    target: resolved,
                }),
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn relative_paths_reject_escapes() {
        validate_relative_path("").expect("root path is valid");
        validate_relative_path("folder/child.txt").expect("nested path is valid");
        validate_relative_path("../escape").expect_err("parent traversal must be rejected");
        validate_relative_path("folder/../escape")
            .expect_err("embedded traversal must be rejected");
        validate_relative_path("/absolute").expect_err("absolute path must be rejected");
        validate_relative_path("folder//child").expect_err("empty component must be rejected");
        validate_relative_path("folder/./child").expect_err("dot component must be rejected");
    }

    #[test]
    fn transfer_names_reject_separators_and_traversal() {
        validate_transfer_name("report.txt").expect("plain name is valid");
        validate_transfer_name("..").expect_err("parent name must be rejected");
        validate_transfer_name(".").expect_err("dot name must be rejected");
        validate_transfer_name("").expect_err("empty name must be rejected");
        validate_transfer_name("nested/child").expect_err("separator must be rejected");
        validate_transfer_name("nested\\child").expect_err("backslash separator must be rejected");
    }

    #[test]
    fn path_backend_writes_and_commits() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime")
            .block_on(async {
                let directory = tempfile::tempdir().expect("create test directory");
                let fs = PathLocalFileSystem::new(directory.path());
                let mut session = fs
                    .begin_write("nested/report.txt")
                    .await
                    .expect("begin write");
                session
                    .writer
                    .write_all(b"hello")
                    .await
                    .expect("write contents");
                session.commit().await.expect("commit write");

                let mut reader = fs.open_read("nested/report.txt").await.expect("open read");
                let mut contents = String::new();
                reader
                    .read_to_string(&mut contents)
                    .await
                    .expect("read contents");
                assert_eq!(contents, "hello");

                let entry = fs.inspect("nested/report.txt").await.expect("inspect file");
                assert_eq!(entry.kind, LocalEntryKind::File);
                assert_eq!(entry.size, Some(5));
            });
    }

    #[test]
    fn discarded_write_leaves_no_temporary_file() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime")
            .block_on(async {
                let directory = tempfile::tempdir().expect("create test directory");
                let fs = PathLocalFileSystem::new(directory.path());
                let mut session = fs.begin_write("report.txt").await.expect("begin write");
                session
                    .writer
                    .write_all(b"partial")
                    .await
                    .expect("write contents");
                session.discard().await;

                assert!(!directory.path().join("report.txt").exists());
                let entries = fs.list("").await.expect("list root");
                assert!(
                    entries.is_empty(),
                    "temporary files must be removed: {entries:?}"
                );
            });
    }

    /// Counters shared by a recording finalize and its test.
    #[derive(Default)]
    struct RecordingCounters {
        commits: std::sync::atomic::AtomicUsize,
        discards: std::sync::atomic::AtomicUsize,
        dropped_unfinished: std::sync::atomic::AtomicUsize,
        finished: std::sync::atomic::AtomicBool,
    }

    /// Records commit/discard calls and optionally fails the finalize step.
    struct RecordingFinalize {
        counters: std::sync::Arc<RecordingCounters>,
        fail_commit: bool,
    }

    impl RecordingFinalize {
        fn new(counters: std::sync::Arc<RecordingCounters>, fail_commit: bool) -> Self {
            Self {
                counters,
                fail_commit,
            }
        }
    }

    impl LocalWriteFinalize for RecordingFinalize {
        fn commit(&mut self) -> LocalFuture<'_, ()> {
            Box::pin(async move {
                use std::sync::atomic::Ordering;
                self.counters.commits.fetch_add(1, Ordering::SeqCst);
                if self.fail_commit {
                    return Err(anyhow::anyhow!("injected finalize failure"));
                }
                self.counters.finished.store(true, Ordering::SeqCst);
                Ok(())
            })
        }

        fn discard(&mut self) -> LocalFuture<'_, ()> {
            Box::pin(async move {
                use std::sync::atomic::Ordering;
                self.counters.discards.fetch_add(1, Ordering::SeqCst);
                self.counters.finished.store(true, Ordering::SeqCst);
                Ok(())
            })
        }
    }

    impl Drop for RecordingFinalize {
        fn drop(&mut self) {
            use std::sync::atomic::Ordering;
            if !self.counters.finished.load(Ordering::SeqCst) {
                self.counters
                    .dropped_unfinished
                    .fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    /// Writer whose flush always fails.
    struct FailingFlushWriter;

    impl tokio::io::AsyncWrite for FailingFlushWriter {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buffer: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::task::Poll::Ready(Ok(buffer.len()))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Err(std::io::Error::other("injected flush failure")))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime")
    }

    #[test]
    fn a_failed_finalize_discards_the_incomplete_document() {
        runtime().block_on(async {
            use std::sync::atomic::Ordering;
            let counters = std::sync::Arc::new(RecordingCounters::default());
            let session = LocalWriteSession::new(
                Box::new(Vec::new()),
                Box::new(RecordingFinalize::new(counters.clone(), true)),
            );

            let error = session.commit().await.expect_err("commit must fail");

            assert!(error.to_string().contains("injected finalize failure"));
            assert_eq!(counters.commits.load(Ordering::SeqCst), 1);
            assert_eq!(
                counters.discards.load(Ordering::SeqCst),
                1,
                "a failed finalize must clean up the incomplete document"
            );
        });
    }

    #[test]
    fn a_flush_failure_discards_the_incomplete_document() {
        runtime().block_on(async {
            use std::sync::atomic::Ordering;
            let counters = std::sync::Arc::new(RecordingCounters::default());
            let session = LocalWriteSession::new(
                Box::new(FailingFlushWriter),
                Box::new(RecordingFinalize::new(counters.clone(), false)),
            );

            session.commit().await.expect_err("flush failure must fail");

            assert_eq!(
                counters.discards.load(Ordering::SeqCst),
                1,
                "a flush failure must clean up the incomplete document"
            );
        });
    }

    #[test]
    fn a_successful_commit_does_not_discard() {
        runtime().block_on(async {
            use std::sync::atomic::Ordering;
            let counters = std::sync::Arc::new(RecordingCounters::default());
            let session = LocalWriteSession::new(
                Box::new(Vec::new()),
                Box::new(RecordingFinalize::new(counters.clone(), false)),
            );

            session.commit().await.expect("commit");

            assert_eq!(counters.commits.load(Ordering::SeqCst), 1);
            assert_eq!(counters.discards.load(Ordering::SeqCst), 0);
        });
    }

    #[test]
    fn an_abandoned_session_reports_the_unfinished_document() {
        runtime().block_on(async {
            use std::sync::atomic::Ordering;
            let counters = std::sync::Arc::new(RecordingCounters::default());
            let session = LocalWriteSession::new(
                Box::new(Vec::new()),
                Box::new(RecordingFinalize::new(counters.clone(), false)),
            );

            // Simulate an aborted transfer task dropping the session.
            drop(session);

            assert_eq!(
                counters.dropped_unfinished.load(Ordering::SeqCst),
                1,
                "dropping an unfinished session must reach the cleanup hook"
            );
        });
    }

    #[test]
    fn inspection_reports_symlinks_without_following_them() {
        #[cfg(unix)]
        {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build runtime")
                .block_on(async {
                    let directory = tempfile::tempdir().expect("create test directory");
                    std::fs::write(directory.path().join("target.txt"), b"data")
                        .expect("write target");
                    std::os::unix::fs::symlink("target.txt", directory.path().join("link.txt"))
                        .expect("create symlink");
                    let fs = PathLocalFileSystem::new(directory.path());
                    let entry = fs.inspect("link.txt").await.expect("inspect symlink");
                    assert_eq!(entry.kind, LocalEntryKind::Symlink);
                });
        }
    }
}
