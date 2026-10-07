//! Shared helpers for in-crate tests.
//!
//! Every test that touches the embedded business root shares one process-wide
//! root (the runtime fixes exactly one) and one mutex so tests running in
//! parallel cannot observe each other's files.

#![cfg(test)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::error::BackendError;
use crate::platform::{LogLevel, PlatformHost, SafEntry, SafEntryKind, SafWriteStart};

pub(crate) struct TestHost {
    values: Mutex<HashMap<String, String>>,
    invalid_after: AtomicUsize,
    /// Directory-backed stand-in for an authorized SAF tree.
    saf_root: Mutex<Option<PathBuf>>,
    /// Discard document writes instead of storing them (large-file tests).
    saf_null_writes: AtomicBool,
    /// Targets whose SAF commit is rejected (provider-refusal tests).
    fail_commit_paths: Mutex<Vec<String>>,
    /// Simulate a provider returning an invalid descriptor after creation.
    invalid_write_descriptor: AtomicBool,
}

impl TestHost {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            values: Mutex::new(HashMap::new()),
            invalid_after: AtomicUsize::new(usize::MAX),
            saf_root: Mutex::new(None),
            saf_null_writes: AtomicBool::new(false),
            fail_commit_paths: Mutex::new(Vec::new()),
            invalid_write_descriptor: AtomicBool::new(false),
        })
    }

    /// Reject commit for these tree-relative targets, like a provider that
    /// cannot safely replace an existing document.
    pub(crate) fn set_fail_commit_paths(&self, paths: Vec<String>) {
        *self.fail_commit_paths.lock().expect("fail commit lock") = paths;
    }

    pub(crate) fn set_invalid_write_descriptor(&self) {
        self.invalid_write_descriptor.store(true, Ordering::SeqCst);
    }

    /// Point the SAF stand-in at a real directory for integration tests.
    pub(crate) fn set_saf_root(&self, path: PathBuf) {
        *self.saf_root.lock().expect("saf root lock") = Some(path);
    }

    /// Discard subsequent SAF writes; documents then commit as delete-safe
    /// no-ops, which lets a large-file test stream without touching the disk.
    pub(crate) fn set_saf_null_writes(&self, enabled: bool) {
        self.saf_null_writes.store(enabled, Ordering::SeqCst);
    }

    fn resolve_saf(&self, relative_path: &str) -> Result<PathBuf, BackendError> {
        miaominal_sftp::validate_relative_path(relative_path).map_err(|error| {
            BackendError::InvalidArgument {
                detail: error.to_string(),
            }
        })?;
        let root = self
            .saf_root
            .lock()
            .expect("saf root lock")
            .clone()
            .ok_or_else(|| BackendError::Unsupported {
                capability: "no test SAF tree is configured".to_string(),
            })?;
        Ok(if relative_path.is_empty() {
            root
        } else {
            root.join(relative_path)
        })
    }

    fn saf_entry(resolved: &std::path::Path, relative: &str) -> Result<SafEntry, BackendError> {
        let metadata = std::fs::symlink_metadata(resolved).map_err(|error| {
            BackendError::Storage {
                storage_failure: error.to_string(),
            }
        })?;
        let kind = if metadata.is_dir() {
            SafEntryKind::Directory
        } else if metadata.is_file() {
            SafEntryKind::File
        } else {
            SafEntryKind::Other
        };
        let name = resolved
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| relative.to_string());
        Ok(SafEntry {
            name,
            relative_path: relative.to_string(),
            kind,
            size: metadata.is_file().then_some(metadata.len()),
        })
    }

    pub(crate) fn stored(&self, account: &str) -> Option<String> {
        self.values
            .lock()
            .expect("test credential map should lock")
            .get(account)
            .cloned()
    }

    /// Simulate a platform key invalidation after `count` successful reads.
    pub(crate) fn invalidate_after(&self, count: usize) {
        self.invalid_after.store(count, Ordering::SeqCst);
    }
}

impl PlatformHost for TestHost {
    fn log(&self, _level: LogLevel, _message: String) {}

    fn device_label(&self) -> String {
        "test host".to_string()
    }

    fn probe_secret(&self, _key: String) -> Result<Option<String>, BackendError> {
        Ok(None)
    }

    fn credential_get(&self, account: String) -> Result<Option<String>, BackendError> {
        let remaining = self.invalid_after.load(Ordering::SeqCst);
        if remaining == 0 {
            return Err(BackendError::Platform {
                platform_failure: "test platform key was invalidated".to_string(),
            });
        }
        if remaining != usize::MAX {
            self.invalid_after.store(remaining - 1, Ordering::SeqCst);
        }
        Ok(self
            .values
            .lock()
            .expect("test credential map should lock")
            .get(&account)
            .cloned())
    }

    fn credential_set(&self, account: String, value: String) -> Result<(), BackendError> {
        self.values
            .lock()
            .expect("test credential map should lock")
            .insert(account, value);
        Ok(())
    }

    fn credential_delete(&self, account: String) -> Result<(), BackendError> {
        self.values
            .lock()
            .expect("test credential map should lock")
            .remove(&account);
        Ok(())
    }

    fn store_clipboard(&self, _text: String) {}

    fn saf_list(
        &self,
        _tree_uri: String,
        relative_path: String,
    ) -> Result<Vec<SafEntry>, BackendError> {
        let resolved = self.resolve_saf(&relative_path)?;
        let mut entries = Vec::new();
        let reader = std::fs::read_dir(&resolved).map_err(|error| BackendError::Storage {
            storage_failure: format!("{}: {error}", resolved.display()),
        })?;
        for entry in reader {
            let entry = entry.map_err(|error| BackendError::Storage {
                storage_failure: error.to_string(),
            })?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = if relative_path.is_empty() {
                name.clone()
            } else {
                format!("{relative_path}/{name}")
            };
            entries.push(Self::saf_entry(&entry.path(), &relative)?);
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(entries)
    }

    fn saf_stat(&self, _tree_uri: String, relative_path: String) -> Result<SafEntry, BackendError> {
        let resolved = self.resolve_saf(&relative_path)?;
        Self::saf_entry(&resolved, &relative_path)
    }

    fn saf_create_directory(
        &self,
        _tree_uri: String,
        relative_path: String,
    ) -> Result<(), BackendError> {
        let resolved = self.resolve_saf(&relative_path)?;
        std::fs::create_dir_all(&resolved).map_err(|error| BackendError::Storage {
            storage_failure: error.to_string(),
        })
    }

    fn saf_delete(&self, _tree_uri: String, relative_path: String) -> Result<(), BackendError> {
        let resolved = self.resolve_saf(&relative_path)?;
        match std::fs::symlink_metadata(&resolved) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(BackendError::Storage {
                storage_failure: error.to_string(),
            }),
            Ok(metadata) if metadata.is_dir() => {
                std::fs::remove_dir_all(&resolved).map_err(|error| BackendError::Storage {
                    storage_failure: error.to_string(),
                })
            }
            Ok(_) => std::fs::remove_file(&resolved).map_err(|error| BackendError::Storage {
                storage_failure: error.to_string(),
            }),
        }
    }

    fn saf_open_file(&self, _tree_uri: String, relative_path: String) -> Result<i32, BackendError> {
        let resolved = self.resolve_saf(&relative_path)?;
        let file = std::fs::File::open(&resolved).map_err(|error| BackendError::Storage {
            storage_failure: error.to_string(),
        })?;
        hand_out_descriptor(file)
    }

    fn saf_begin_write(
        &self,
        _tree_uri: String,
        relative_path: String,
    ) -> Result<SafWriteStart, BackendError> {
        if self.saf_null_writes.load(Ordering::SeqCst) {
            #[cfg(unix)]
            let file = std::fs::File::create("/dev/null");
            #[cfg(windows)]
            let file = std::fs::File::create("NUL");
            #[cfg(not(any(unix, windows)))]
            let file: Result<std::fs::File, std::io::Error> = Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "no null device on this platform",
            ));
            let file = file.map_err(|error| BackendError::Storage {
                storage_failure: error.to_string(),
            })?;
            return Ok(SafWriteStart {
                fd: hand_out_descriptor(file)?,
                staging_relative: None,
            });
        }
        let resolved = self.resolve_saf(&relative_path)?;
        if let Some(parent) = resolved.parent() {
            std::fs::create_dir_all(parent).map_err(|error| BackendError::Storage {
                storage_failure: error.to_string(),
            })?;
        }
        // Emulate the Android contract: an existing target is replaced through
        // a staging document, a new target is created directly.
        let staging_relative = if resolved.exists() {
            let parent = relative_path
                .rsplit_once('/')
                .map(|(parent, _)| parent.to_string())
                .unwrap_or_default();
            let staging_name = format!(
                ".miaominal-test-{}{}",
                std::process::id(),
                resolved
                    .extension()
                    .map(|extension| format!(".{}", extension.to_string_lossy()))
                    .unwrap_or_default()
            );
            let staging = if parent.is_empty() {
                staging_name.clone()
            } else {
                format!("{parent}/{staging_name}")
            };
            Some(staging)
        } else {
            None
        };
        let write_target = staging_relative
            .as_deref()
            .and_then(|staging| self.resolve_saf(staging).ok())
            .unwrap_or(resolved);
        if let Some(parent) = write_target.parent() {
            std::fs::create_dir_all(parent).map_err(|error| BackendError::Storage {
                storage_failure: error.to_string(),
            })?;
        }
        let file = std::fs::File::create(&write_target).map_err(|error| BackendError::Storage {
            storage_failure: error.to_string(),
        })?;
        let fd = if self.invalid_write_descriptor.load(Ordering::SeqCst) {
            drop(file);
            -1
        } else {
            hand_out_descriptor(file)?
        };
        Ok(SafWriteStart {
            fd,
            staging_relative,
        })
    }

    fn saf_finish_write(
        &self,
        _tree_uri: String,
        target_relative: String,
        staging_relative: Option<String>,
        commit: bool,
    ) -> Result<(), BackendError> {
        if self.saf_null_writes.load(Ordering::SeqCst) {
            return Ok(());
        }
        if !commit {
            // Only the incomplete document created by this task is removed.
            let relative = staging_relative.unwrap_or(target_relative);
            return self.saf_delete(String::new(), relative);
        }
        if self
            .fail_commit_paths
            .lock()
            .expect("fail commit lock")
            .iter()
            .any(|path| path == &target_relative)
        {
            return Err(BackendError::Unsupported {
                capability: format!("the test provider cannot safely replace {target_relative}"),
            });
        }
        if let Some(staging) = staging_relative {
            let target = self.resolve_saf(&target_relative)?;
            let staging_path = self.resolve_saf(&staging)?;
            std::fs::rename(&staging_path, &target).map_err(|error| BackendError::Storage {
                storage_failure: error.to_string(),
            })?;
        }
        Ok(())
    }
}

/// Hand out a descriptor for a file the same way Kotlin's SAF adapter does:
/// ownership of the descriptor (and its handle) transfers to Rust.
fn hand_out_descriptor(file: std::fs::File) -> Result<i32, BackendError> {
    #[cfg(unix)]
    {
        use std::os::fd::IntoRawFd;
        Ok(file.into_raw_fd())
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::IntoRawHandle;
        let handle = file.into_raw_handle();
        let fd = unsafe {
            libc::open_osfhandle(handle as libc::intptr_t, libc::O_RDWR | libc::O_BINARY)
        };
        if fd < 0 {
            return Err(BackendError::Storage {
                storage_failure: std::io::Error::last_os_error().to_string(),
            });
        }
        Ok(fd)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Err(BackendError::Unsupported {
            capability: "the host test SAF tree cannot hand out descriptors".to_string(),
        })
    }
}

pub(crate) fn business_root() -> &'static PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let temp = tempfile::tempdir().expect("test root should be created");
        let path = temp.path().join("business");
        std::mem::forget(temp);
        path
    })
}

pub(crate) fn cache_root() -> &'static PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let temp = tempfile::tempdir().expect("test cache root should be created");
        let path = temp.path().join("cache");
        std::mem::forget(temp);
        path
    })
}

/// Serialize tests that read or write the shared embedded root.
pub(crate) fn shared_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Initialize the embedded runtime for the shared test root.
pub(crate) fn initialize_test_runtime() {
    miaominal_paths::initialize_embedded_runtime(business_root(), cache_root())
        .expect("the embedded test runtime should initialize");
}

/// Remove every business file between tests.
///
/// The embedded runtime is initialized first so a test can never fall back to
/// the developer's desktop configuration directory.
pub(crate) fn clear_test_root() {
    initialize_test_runtime();
    assert_eq!(
        miaominal_paths::runtime_mode().expect("runtime mode should be available"),
        miaominal_paths::RuntimeMode::Embedded,
        "tests must only clear the embedded test root"
    );
    miaominal_paths::clear_active_data_dir().expect("the test root should clear");
}

/// Point built-in test helpers at a test host. Kept separate so callers can
/// decide when to construct the data layer.
pub(crate) fn test_host() -> Arc<TestHost> {
    TestHost::new()
}
