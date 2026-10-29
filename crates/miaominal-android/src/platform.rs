//! Platform capabilities implemented by the Android application process.

use crate::error::BackendError;

/// Bridge Rust `log` records to the platform log stream.
///
/// The Kotlin host receives explicit messages through [`PlatformHost::log`],
/// but the `log` crate needs a process-wide logger; without one every
/// `log::warn!` in this crate is dropped silently.
#[cfg(target_os = "android")]
mod log_bridge {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int};

    #[link(name = "log")]
    unsafe extern "C" {
        fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
    }

    const ANDROID_LOG_DEBUG: c_int = 3;
    const ANDROID_LOG_INFO: c_int = 4;
    const ANDROID_LOG_WARN: c_int = 5;
    const ANDROID_LOG_ERROR: c_int = 6;

    pub(super) fn write(level: super::LogLevel, message: &str) {
        let prio = match level {
            super::LogLevel::Debug => ANDROID_LOG_DEBUG,
            super::LogLevel::Info => ANDROID_LOG_INFO,
            super::LogLevel::Warn => ANDROID_LOG_WARN,
            super::LogLevel::Error => ANDROID_LOG_ERROR,
        };
        let tag = c"MiaominalNative";
        let Ok(text) = CString::new(message) else {
            return;
        };
        unsafe {
            __android_log_write(prio, tag.as_ptr(), text.as_ptr());
        }
    }
}

#[cfg(target_os = "android")]
struct Logger;

#[cfg(target_os = "android")]
impl log::Log for Logger {
    fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        let level = match record.level() {
            log::Level::Error => LogLevel::Error,
            log::Level::Warn => LogLevel::Warn,
            log::Level::Info => LogLevel::Info,
            log::Level::Debug | log::Level::Trace => LogLevel::Debug,
        };
        log_bridge::write(level, &format!("[{}] {}", record.target(), record.args()));
    }

    fn flush(&self) {}
}

#[cfg(target_os = "android")]
static LOGGER: Logger = Logger;

/// Install the Rust `log` bridge exactly once. Desktop builds (tests) keep the
/// default no-op logger so test output stays clean.
#[cfg(target_os = "android")]
pub(crate) fn install_logger() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = log::set_logger(&LOGGER);
        log::set_max_level(log::LevelFilter::Debug);
        // The default hook writes to stderr, which Android discards for app
        // processes; a panicking native task would otherwise die silently.
        std::panic::set_hook(Box::new(|info| {
            log::error!("native panic: {info}");
        }));
    });
}

#[cfg(not(target_os = "android"))]
pub(crate) fn install_logger() {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SafEntryKind {
    File,
    Directory,
    Other,
}

/// One entry of a SAF directory listing, relative to the authorized tree.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SafEntry {
    pub name: String,
    /// Path relative to the tree root, using `/` separators.
    pub relative_path: String,
    pub kind: SafEntryKind,
    pub size: Option<u64>,
}

/// Outcome of opening a SAF document for writing.
///
/// `staging_relative` is `Some` when the provider cannot replace the existing
/// document in place; the transfer writes into that staging document and the
/// final rename happens in [`PlatformHost::saf_finish_write`]. When it is
/// `None`, `fd` already points at the final document.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SafWriteStart {
    pub fd: i32,
    pub staging_relative: Option<String>,
}

/// Implemented in Kotlin and handed to [`crate::Backend::new`].
///
/// Credential methods move opaque ciphertext only: the shared stores decide
/// the account layout and the encryption format, while Kotlin owns the
/// Android Keystore key and the on-device file. The SAF methods expose
/// document directory operations; file contents always move through
/// duplicated file descriptors, never through byte arrays.
#[uniffi::export(foreign)]
pub trait PlatformHost: Send + Sync {
    fn log(&self, level: LogLevel, message: String);
    fn device_label(&self) -> String;
    fn probe_secret(&self, key: String) -> Result<Option<String>, BackendError>;
    fn credential_get(&self, account: String) -> Result<Option<String>, BackendError>;
    fn credential_set(&self, account: String, value: String) -> Result<(), BackendError>;
    fn credential_delete(&self, account: String) -> Result<(), BackendError>;
    /// OSC 52 clipboard write from the remote; the host decides where it lands.
    fn store_clipboard(&self, text: String);

    /// List one directory inside an authorized SAF tree.
    fn saf_list(
        &self,
        tree_uri: String,
        relative_path: String,
    ) -> Result<Vec<SafEntry>, BackendError>;

    /// Metadata for one document or directory inside an authorized SAF tree.
    fn saf_stat(&self, tree_uri: String, relative_path: String) -> Result<SafEntry, BackendError>;

    /// Create one directory (and any missing parent) inside a SAF tree.
    fn saf_create_directory(
        &self,
        tree_uri: String,
        relative_path: String,
    ) -> Result<(), BackendError>;

    /// Delete a document or directory inside a SAF tree. Missing entries are
    /// reported as success so cleanup paths are idempotent.
    fn saf_delete(&self, tree_uri: String, relative_path: String) -> Result<(), BackendError>;

    /// Open an existing document for reading and transfer ownership of its
    /// descriptor to Rust (Kotlin detaches it and must not close it again).
    fn saf_open_file(&self, tree_uri: String, relative_path: String) -> Result<i32, BackendError>;

    /// Open (or create) a document for writing. The returned descriptor is
    /// owned by Rust; `staging_relative` decides whether a final commit is
    /// required.
    fn saf_begin_write(
        &self,
        tree_uri: String,
        relative_path: String,
    ) -> Result<SafWriteStart, BackendError>;

    /// Commit or discard a write started with [`PlatformHost::saf_begin_write`].
    ///
    /// With `commit == true` a staging document replaces the target (and a
    /// previous version, if any, is only removed after the replacement
    /// succeeded). With `commit == false` only the incomplete document created
    /// by this task is removed.
    fn saf_finish_write(
        &self,
        tree_uri: String,
        target_relative: String,
        staging_relative: Option<String>,
        commit: bool,
    ) -> Result<(), BackendError>;
}
