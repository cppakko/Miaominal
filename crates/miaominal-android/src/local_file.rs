//! Android local file descriptors and SAF-backed local file systems.
//!
//! Kotlin never sends file contents across UniFFI. It opens a
//! `ParcelFileDescriptor`, hands the raw descriptor to
//! [`Backend::register_local_file`] (or the SAF adapter opens documents on
//! demand), and Rust duplicates it before returning. The duplicate is owned by
//! the handle and closed exactly once, including on failure and cancel paths.

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use futures::future::BoxFuture;
use miaominal_sftp::local::{
    LocalEntry, LocalEntryKind, LocalFileSystem, LocalFuture, LocalWriteFinalize,
    LocalWriteSession, join_local_path, validate_relative_path,
};

use crate::error::BackendError;
use crate::platform::{PlatformHost, SafEntryKind};

fn invalid_descriptor() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "file descriptor must not be negative",
    )
}

/// Duplicate a Kotlin-owned descriptor into an owned [`std::fs::File`].
///
/// On Android this is `F_DUPFD_CLOEXEC`, so the copy is close-on-exec and can
/// never leak into a child process. The Windows implementation exists for the
/// host test suite: it duplicates the CRT descriptor's OS handle so the
/// returned file and the original descriptor close independently.
pub(crate) fn open_owned_fd(fd: i32) -> std::io::Result<std::fs::File> {
    if fd < 0 {
        return Err(invalid_descriptor());
    }
    #[cfg(unix)]
    {
        use std::os::fd::FromRawFd;
        let duplicated = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
        if duplicated < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { std::fs::File::from_raw_fd(duplicated) })
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::{FromRawHandle, RawHandle};
        let duplicate = unsafe { libc::dup(fd) };
        if duplicate < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let handle = unsafe { libc::get_osfhandle(duplicate) };
        if handle == -1 {
            unsafe {
                libc::close(duplicate);
            }
            return Err(std::io::Error::last_os_error());
        }
        let owned = duplicate_os_handle(handle as RawHandle);
        unsafe {
            libc::close(duplicate);
        }
        Ok(unsafe { std::fs::File::from_raw_handle(owned?) })
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "file descriptor registration is only available on Android",
        ))
    }
}

/// Duplicate a platform-handover descriptor and close the source.
///
/// Used when Rust itself asked the platform for a document (SAF reads and
/// writes): Kotlin detaches the descriptor and never touches it again, so Rust
/// owns both the duplicate and the original.
pub(crate) fn take_owned_fd(fd: i32) -> std::io::Result<std::fs::File> {
    take_owned_fd_with(fd, open_owned_fd)
}

fn take_owned_fd_with(
    fd: i32,
    duplicate: impl FnOnce(i32) -> std::io::Result<std::fs::File>,
) -> std::io::Result<std::fs::File> {
    if fd < 0 {
        return Err(invalid_descriptor());
    }
    struct SourceDescriptor(i32);
    impl Drop for SourceDescriptor {
        fn drop(&mut self) {
            close_source_fd(self.0);
        }
    }
    let source = SourceDescriptor(fd);
    duplicate(source.0)
}

fn close_source_fd(fd: i32) {
    #[cfg(unix)]
    {
        // Ownership was transferred to Rust. Release the source even when
        // duplication failed; a close error cannot be retried safely.
        unsafe {
            libc::close(fd);
        }
    }
    #[cfg(windows)]
    {
        unsafe {
            libc::close(fd);
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = fd;
    }
}

#[cfg(windows)]
fn duplicate_os_handle(
    handle: std::os::windows::io::RawHandle,
) -> std::io::Result<std::os::windows::io::RawHandle> {
    use std::ffi::c_void;
    use std::os::windows::io::RawHandle;

    const DUPLICATE_SAME_ACCESS: u32 = 0x0000_0002;
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn DuplicateHandle(
            source_process: *mut c_void,
            source_handle: *mut c_void,
            target_process: *mut c_void,
            target_handle: *mut *mut c_void,
            desired_access: u32,
            inherit_handle: i32,
            options: u32,
        ) -> i32;
    }

    let mut duplicated: *mut c_void = std::ptr::null_mut();
    let duplicated_ok = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            handle as *mut c_void,
            GetCurrentProcess(),
            &mut duplicated,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if duplicated_ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(duplicated as RawHandle)
}

/// Test-only helper: open a path and hand out a descriptor the same way Kotlin
/// does (`detachFd`-style ownership transfer).
#[cfg(test)]
pub(crate) fn open_test_fd(path: &std::path::Path) -> std::io::Result<i32> {
    #[cfg(unix)]
    {
        use std::os::fd::IntoRawFd;
        Ok(std::fs::File::open(path)?.into_raw_fd())
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::IntoRawHandle;
        let handle = std::fs::File::open(path)?.into_raw_handle();
        let fd = unsafe {
            libc::open_osfhandle(handle as libc::intptr_t, libc::O_RDONLY | libc::O_BINARY)
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(fd)
    }
}

struct LocalFileState {
    file: Option<std::fs::File>,
    name: String,
    size: u64,
}

/// One Kotlin-owned document registered with the native transfer engine.
///
/// Kotlin must close its original descriptor after registration; this handle
/// owns the duplicate. `close()` is idempotent and is also performed when the
/// handle is dropped, so a failed or cancelled transfer never leaks a
/// descriptor.
#[derive(uniffi::Object)]
pub struct LocalFileHandle {
    state: Mutex<LocalFileState>,
}

impl std::fmt::Debug for LocalFileHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.lock();
        formatter
            .debug_struct("LocalFileHandle")
            .field("name", &state.name)
            .field("size", &state.size)
            .field("closed", &state.file.is_none())
            .finish()
    }
}

impl LocalFileHandle {
    pub(crate) fn from_file(file: std::fs::File, name: String) -> Result<Arc<Self>, BackendError> {
        let metadata = file.metadata().map_err(|error| BackendError::Storage {
            storage_failure: format!("failed to read registered file metadata: {error}"),
        })?;
        // A document that cannot seek is not usable by the transfer engine;
        // report it instead of silently buffering the whole file.
        file.try_clone()
            .and_then(|mut probe| {
                use std::io::Seek;
                probe.seek(std::io::SeekFrom::Current(0)).map(|_| ())
            })
            .map_err(|error| BackendError::Unsupported {
                capability: format!("local document does not support seeking: {error}"),
            })?;
        Ok(Arc::new(Self {
            state: Mutex::new(LocalFileState {
                file: Some(file),
                name,
                size: metadata.len(),
            }),
        }))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LocalFileState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Duplicate the descriptor for one reader; fails once the handle is closed.
    pub(crate) fn try_clone_file(&self) -> Result<std::fs::File, BackendError> {
        let state = self.lock();
        let Some(file) = state.file.as_ref() else {
            return Err(BackendError::LocalHandleInvalid {
                handle_detail: "the registered file handle was already closed".to_string(),
            });
        };
        file.try_clone()
            .map_err(|_| BackendError::LocalHandleInvalid {
                handle_detail: "the registered file handle is no longer usable".to_string(),
            })
    }
}

#[uniffi::export]
impl LocalFileHandle {
    /// Close the Rust-owned duplicate. Idempotent; returns `true` when this
    /// call performed the close. Named `release` because the generated Kotlin
    /// class already implements `AutoCloseable.close`.
    pub fn release(&self) -> bool {
        let mut state = self.lock();
        state.file.take().is_some()
    }

    pub fn is_closed(&self) -> bool {
        self.lock().file.is_none()
    }

    pub fn name(&self) -> String {
        self.lock().name.clone()
    }

    pub fn size(&self) -> u64 {
        self.lock().size
    }
}

/// Local file system for one registered document (single-file upload).
pub(crate) struct RegisteredFileSource {
    handle: Arc<LocalFileHandle>,
    name: String,
}

impl RegisteredFileSource {
    pub(crate) fn new(handle: Arc<LocalFileHandle>) -> Self {
        let name = handle.name();
        Self { handle, name }
    }
}

impl LocalFileSystem for RegisteredFileSource {
    fn display_root(&self) -> String {
        self.name.clone()
    }

    fn inspect<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<LocalEntry>> {
        Box::pin(async move {
            if path != self.name {
                return Err(anyhow!("registered file has no entry {path}"));
            }
            Ok(LocalEntry {
                name: self.name.clone(),
                kind: LocalEntryKind::File,
                size: Some(self.handle.size()),
            })
        })
    }

    fn list<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, Result<Vec<LocalEntry>>> {
        Box::pin(async move { Err(anyhow!("a registered file has no children")) })
    }

    fn create_dir_all<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move { Err(anyhow!("a registered file cannot create directories")) })
    }

    fn open_read<'a>(
        &'a self,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>>> {
        Box::pin(async move {
            if path != self.name {
                return Err(anyhow!("registered file has no entry {path}"));
            }
            let file = self.handle.try_clone_file()?;
            Ok(Box::new(tokio::fs::File::from_std(file))
                as Box<dyn tokio::io::AsyncRead + Send + Unpin>)
        })
    }

    fn begin_write<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, Result<LocalWriteSession>> {
        Box::pin(async move { Err(anyhow!("a registered read-only file cannot be written")) })
    }
}

/// SAF-backed local file system used for directory transfers.
///
/// Directory enumeration and document creation go through [`PlatformHost`];
/// bytes always move through descriptors the platform hands over to Rust.
pub(crate) struct SafLocalFileSystem {
    host: Arc<dyn PlatformHost>,
    tree_uri: String,
    root: String,
}

impl SafLocalFileSystem {
    pub(crate) fn new(host: Arc<dyn PlatformHost>, tree_uri: String, root: String) -> Self {
        Self {
            host,
            tree_uri,
            root,
        }
    }

    fn resolve(&self, path: &str) -> Result<String> {
        validate_relative_path(path)?;
        if path.is_empty() {
            return Ok(self.root.clone());
        }
        if self.root.is_empty() {
            return Ok(path.to_string());
        }
        Ok(join_local_path(&self.root, path))
    }

    fn entry_from_saf(entry: crate::platform::SafEntry) -> LocalEntry {
        LocalEntry {
            name: entry.name,
            kind: match entry.kind {
                SafEntryKind::File => LocalEntryKind::File,
                SafEntryKind::Directory => LocalEntryKind::Directory,
                SafEntryKind::Other => LocalEntryKind::Other,
            },
            size: entry.size,
        }
    }
}

struct SafWriteFinalize {
    host: Arc<dyn PlatformHost>,
    tree_uri: String,
    target: String,
    staging: Option<String>,
    /// Set once commit succeeded (or discard ran) so cleanup cannot remove
    /// finished data and idempotent calls stay no-ops.
    finished: bool,
}

impl SafWriteFinalize {
    fn finish(&mut self, commit: bool) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        let result = self
            .host
            .saf_finish_write(
                self.tree_uri.clone(),
                self.target.clone(),
                self.staging.clone(),
                commit,
            )
            .map_err(|error| anyhow!("{error}"));
        if result.is_ok() {
            self.finished = true;
        } else if !commit {
            // Cleanup is best effort; a provider failure here must not mask
            // the original transfer error.
            log::warn!(
                "failed to clean up an incomplete SAF document for {}: {result:?}",
                self.target
            );
        }
        result
    }
}

impl LocalWriteFinalize for SafWriteFinalize {
    fn commit(&mut self) -> LocalFuture<'_, ()> {
        Box::pin(async move { self.finish(true) })
    }

    fn discard(&mut self) -> LocalFuture<'_, ()> {
        Box::pin(async move {
            // Discard never fails the caller; the session just logs leftovers.
            let _ = self.finish(false);
            Ok(())
        })
    }
}

impl Drop for SafWriteFinalize {
    fn drop(&mut self) {
        // An aborted transfer task drops the session without an explicit
        // discard; remove the incomplete document (or the staging document
        // when a replacement was in progress) as a last resort.
        if self.finished {
            return;
        }
        let _ = self
            .host
            .saf_finish_write(
                self.tree_uri.clone(),
                self.target.clone(),
                self.staging.clone(),
                false,
            )
            .map_err(|error| {
                log::warn!(
                    "failed to clean up an abandoned SAF write for {}: {error}",
                    self.target
                );
            });
        self.finished = true;
    }
}

impl LocalFileSystem for SafLocalFileSystem {
    fn display_root(&self) -> String {
        if self.root.is_empty() {
            self.tree_uri.clone()
        } else {
            format!("{}/{}", self.tree_uri, self.root)
        }
    }

    fn inspect<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<LocalEntry>> {
        Box::pin(async move {
            let relative = self.resolve(path)?;
            let entry = self
                .host
                .saf_stat(self.tree_uri.clone(), relative)
                .map_err(|error| anyhow!("{error}"))?;
            Ok(Self::entry_from_saf(entry))
        })
    }

    fn list<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Vec<LocalEntry>>> {
        Box::pin(async move {
            let relative = self.resolve(path)?;
            let entries = self
                .host
                .saf_list(self.tree_uri.clone(), relative)
                .map_err(|error| anyhow!("{error}"))?;
            Ok(entries.into_iter().map(Self::entry_from_saf).collect())
        })
    }

    fn create_dir_all<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let relative = self.resolve(path)?;
            if relative.is_empty() {
                return Ok(());
            }
            self.host
                .saf_create_directory(self.tree_uri.clone(), relative)
                .map_err(|error| anyhow!("{error}"))
        })
    }

    fn open_read<'a>(
        &'a self,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>>> {
        Box::pin(async move {
            let relative = self.resolve(path)?;
            let fd = self
                .host
                .saf_open_file(self.tree_uri.clone(), relative)
                .map_err(|error| anyhow!("{error}"))?;
            let file = take_owned_fd(fd).with_context(|| {
                format!(
                    "failed to own the SAF descriptor for {}",
                    self.display_root()
                )
            })?;
            Ok(Box::new(tokio::fs::File::from_std(file))
                as Box<dyn tokio::io::AsyncRead + Send + Unpin>)
        })
    }

    fn begin_write<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<LocalWriteSession>> {
        Box::pin(async move {
            let relative = self.resolve(path)?;
            let start = self
                .host
                .saf_begin_write(self.tree_uri.clone(), relative.clone())
                .map_err(|error| anyhow!("{error}"))?;
            // Establish cleanup before descriptor adoption, which can fail
            // after the platform has already created the staging document.
            let finalize = SafWriteFinalize {
                host: Arc::clone(&self.host),
                tree_uri: self.tree_uri.clone(),
                target: relative.clone(),
                staging: start.staging_relative,
                finished: false,
            };
            let file = take_owned_fd(start.fd).with_context(|| {
                format!("failed to own the SAF write descriptor for {relative}")
            })?;
            Ok(LocalWriteSession::new(
                Box::new(tokio::fs::File::from_std(file)),
                Box::new(finalize),
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_host;

    #[test]
    fn invalid_descriptors_are_rejected_before_dup() {
        #[cfg(unix)]
        {
            let error = open_owned_fd(-1).expect_err("negative descriptors must fail");
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        }
    }

    #[test]
    fn registered_file_handle_closes_once() {
        let directory = tempfile::tempdir().expect("create temporary directory");
        let path = directory.path().join("document.txt");
        std::fs::write(&path, b"payload").expect("write document");
        let file = std::fs::File::open(&path).expect("open document");
        let handle =
            LocalFileHandle::from_file(file, "document.txt".to_string()).expect("register file");

        assert!(!handle.is_closed());
        assert!(handle.try_clone_file().is_ok());
        assert!(handle.release(), "first close owns the descriptor");
        assert!(!handle.release(), "second close is a no-op");
        assert!(handle.is_closed());
        assert!(matches!(
            handle.try_clone_file(),
            Err(BackendError::LocalHandleInvalid { .. })
        ));
    }

    #[test]
    fn adopting_a_descriptor_closes_the_source_even_when_duplication_fails() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("document.txt");
        std::fs::write(&path, b"payload").unwrap();
        let fd = open_test_fd(&path).unwrap();
        #[cfg(windows)]
        let source_handle = unsafe { libc::get_osfhandle(fd) };

        let error =
            take_owned_fd_with(fd, |_| Err(std::io::Error::from_raw_os_error(libc::EMFILE)))
                .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::EMFILE));

        #[cfg(unix)]
        assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETFD) }, -1);
        #[cfg(windows)]
        {
            unsafe extern "system" {
                fn GetHandleInformation(handle: *mut std::ffi::c_void, flags: *mut u32) -> i32;
            }
            let mut flags = 0;
            assert_eq!(
                unsafe { GetHandleInformation(source_handle as *mut std::ffi::c_void, &mut flags) },
                0
            );
            assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(6));
        }
    }

    #[test]
    fn failed_saf_descriptor_adoption_removes_only_the_incomplete_document() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("existing.txt");
        std::fs::write(&original, b"original").unwrap();
        let host = crate::test_support::TestHost::new();
        host.set_saf_root(directory.path().to_path_buf());
        host.set_invalid_write_descriptor();
        let fs = SafLocalFileSystem::new(host, "content://tree".into(), String::new());

        crate::runtime::runtime().block_on(async {
            assert!(fs.begin_write("existing.txt").await.is_err());
            assert!(fs.begin_write("new.txt").await.is_err());
        });

        assert_eq!(std::fs::read(&original).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn registered_source_reads_without_loading_the_whole_file() {
        let directory = tempfile::tempdir().expect("create temporary directory");
        let path = directory.path().join("document.bin");
        std::fs::write(&path, vec![7u8; 4096]).expect("write document");
        let file = std::fs::File::open(&path).expect("open document");
        let handle =
            LocalFileHandle::from_file(file, "document.bin".to_string()).expect("register file");
        let source = RegisteredFileSource::new(Arc::clone(&handle));

        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime")
            .block_on(async {
                let entry = source.inspect("document.bin").await.expect("inspect");
                assert_eq!(entry.size, Some(4096));
                assert!(source.inspect("other.bin").await.is_err());
            });

        assert!(handle.release());
    }

    #[test]
    fn saf_local_file_system_rejects_escapes_before_calling_the_platform() {
        let host = test_host();
        let fs = SafLocalFileSystem::new(host, "content://tree".into(), String::new());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime");
        runtime.block_on(async {
            assert!(fs.inspect("../escape").await.is_err());
            assert!(fs.list("..").await.is_err());
        });
    }
}
