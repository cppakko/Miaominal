use crate::RdpEndpoint;
use anyhow::Result;

#[cfg(windows)]
mod platform {
    use super::*;
    use anyhow::Context;
    use std::{
        ptr,
        sync::{Mutex, MutexGuard, OnceLock},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, ERROR_NOT_FOUND, GetLastError, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0,
        },
        Security::Credentials::*,
        System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
    };
    use zeroize::Zeroizing;

    const PREFIX: &str = "Miaominal temporary RDP credential ";
    /// Leases are written as domain-password credentials, the type mstsc itself saves.
    /// Unlike generic credentials, other processes of the same user cannot read the
    /// password blob back with `CredReadW`.
    const LEASE_TYPE: CRED_TYPE = CRED_TYPE_DOMAIN_PASSWORD;
    /// Leases older than this are removed by startup cleanup.
    const LEASE_TTL: Duration = Duration::from_secs(30);
    /// Startup cleanup tolerates this much delay between the marker timestamp and the
    /// credential's LastWritten time; anything written later belongs to someone else.
    const WRITE_SLACK_SECS: u64 = 120;
    const ACQUIRE_WAIT_MS: u32 = 3_000;
    const DELETE_WAIT_MS: u32 = 5_000;
    const DELETE_ATTEMPTS: u32 = 6;

    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    fn local_lock() -> MutexGuard<'static, ()> {
        // The guarded data is `()`; a panic elsewhere cannot leave it inconsistent.
        LOCK.get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Cross-process lock shared by every Miaominal instance of this user session.
    struct ProcessLock(HANDLE);
    impl ProcessLock {
        fn acquire(wait_ms: u32) -> Option<Self> {
            let name = wide("Local\\Miaominal.Rdp.Credentials.v1");
            let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
            if handle.is_null() {
                return None;
            }
            match unsafe { WaitForSingleObject(handle, wait_ms) } {
                WAIT_OBJECT_0 | WAIT_ABANDONED => Some(Self(handle)),
                _ => {
                    unsafe {
                        CloseHandle(handle);
                    }
                    None
                }
            }
        }
    }
    impl Drop for ProcessLock {
        fn drop(&mut self) {
            unsafe {
                ReleaseMutex(self.0);
                CloseHandle(self.0);
            }
        }
    }
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    unsafe fn string(value: *const u16) -> String {
        if value.is_null() {
            return String::new();
        }
        let mut len = 0;
        unsafe {
            while *value.add(len) != 0 {
                len += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(value, len))
        }
    }
    /// mstsc releases differ in whether the lookup target for a non-default port keeps
    /// the port. Write both forms; callers must therefore use a host name that is unique
    /// to this launch (tunnels use a per-session loopback address).
    fn targets(endpoint: &RdpEndpoint) -> Vec<String> {
        let mut targets = vec![format!("TERMSRV/{}", endpoint.host())];
        if endpoint.port() != 3389 {
            targets.push(format!("TERMSRV/{}", endpoint.address()));
        }
        targets
    }
    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
    fn filetime(value: &windows_sys::Win32::Foundation::FILETIME) -> u64 {
        (value.dwHighDateTime as u64) << 32 | value.dwLowDateTime as u64
    }

    /// Whether any credential type already exists for `target`. Errors other than
    /// "not found" are treated as existing so a user's credential is never shadowed.
    fn exists(target: &[u16]) -> bool {
        for kind in [CRED_TYPE_GENERIC, CRED_TYPE_DOMAIN_PASSWORD] {
            let mut existing = ptr::null_mut();
            if unsafe { CredReadW(target.as_ptr(), kind, 0, &mut existing) } != 0 {
                unsafe {
                    CredFree(existing.cast());
                }
                return true;
            }
            if unsafe { GetLastError() } != ERROR_NOT_FOUND {
                return true;
            }
        }
        false
    }

    /// Reads the comment and LastWritten of our lease type for `target`.
    fn read_lease(target: &[u16]) -> Option<(String, u64)> {
        let mut stored = ptr::null_mut();
        if unsafe { CredReadW(target.as_ptr(), LEASE_TYPE, 0, &mut stored) } == 0 {
            return None;
        }
        let result = unsafe { (string((*stored).Comment), filetime(&(*stored).LastWritten)) };
        unsafe {
            CredFree(stored.cast());
        }
        Some(result)
    }

    struct Written {
        target: Vec<u16>,
        written: u64,
    }

    /// Deletes each target only while it still carries this lease's marker and write time.
    fn delete_owned(entries: &[Written], marker: &str) {
        for entry in entries {
            if read_lease(&entry.target)
                .is_some_and(|(comment, written)| comment == marker && written == entry.written)
            {
                unsafe {
                    CredDeleteW(entry.target.as_ptr(), LEASE_TYPE, 0);
                }
            }
        }
    }

    pub(crate) struct CredentialLease {
        entries: Vec<Written>,
        marker: String,
    }
    impl CredentialLease {
        pub(crate) fn acquire(
            endpoint: &RdpEndpoint,
            username: &str,
            password: &str,
        ) -> Result<Option<Self>> {
            let _guard = local_lock();
            let Some(_process_lock) = ProcessLock::acquire(ACQUIRE_WAIT_MS) else {
                return Ok(None);
            };
            let targets = targets(endpoint)
                .iter()
                .map(|target| wide(target))
                .collect::<Vec<_>>();
            // Credentials owned by the user or another launch must never be overwritten.
            if targets.iter().any(|target| exists(target)) {
                return Ok(None);
            }
            let marker = format!("{PREFIX}{} {}", now(), uuid::Uuid::new_v4());
            let comment = wide(&marker);
            let username = wide(username);
            let mut password = Zeroizing::new(password.encode_utf16().collect::<Vec<u16>>());
            let blob_size =
                u32::try_from(password.len() * 2).context("RDP password is too long")?;
            let mut entries = Vec::new();
            for target in targets {
                let credential = CREDENTIALW {
                    Type: LEASE_TYPE,
                    TargetName: target.as_ptr().cast_mut(),
                    Comment: comment.as_ptr().cast_mut(),
                    CredentialBlobSize: blob_size,
                    CredentialBlob: password.as_mut_ptr().cast(),
                    Persist: CRED_PERSIST_SESSION,
                    UserName: username.as_ptr().cast_mut(),
                    ..unsafe { std::mem::zeroed() }
                };
                // Policy restrictions on saved credentials fall back to a client prompt.
                let written = if unsafe { CredWriteW(&credential, 0) } != 0 {
                    read_lease(&target)
                        .filter(|(stored, _)| *stored == marker)
                        .map(|(_, written)| written)
                } else {
                    None
                };
                match written {
                    Some(written) => entries.push(Written { target, written }),
                    None => {
                        // Without a verifiable write time the lease could never be removed
                        // safely; delete what is ours now and let the client prompt.
                        if read_lease(&target).is_some_and(|(stored, _)| stored == marker) {
                            unsafe {
                                CredDeleteW(target.as_ptr(), LEASE_TYPE, 0);
                            }
                        }
                        delete_owned(&entries, &marker);
                        return Ok(None);
                    }
                }
            }
            Ok(Some(Self { entries, marker }))
        }
    }
    impl Drop for CredentialLease {
        fn drop(&mut self) {
            let entries = std::mem::take(&mut self.entries);
            let marker = std::mem::take(&mut self.marker);
            {
                let guard = match LOCK.get_or_init(Mutex::default).try_lock() {
                    Ok(guard) => Some(guard),
                    Err(std::sync::TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
                    Err(std::sync::TryLockError::WouldBlock) => None,
                };
                if let Some(_guard) = guard
                    && let Some(_process_lock) = ProcessLock::acquire(0)
                {
                    delete_owned(&entries, &marker);
                    return;
                }
            }
            // Another instance holds the lock. Never block the caller (often an async
            // worker); retry on a short-lived thread so the password does not linger.
            let _ = std::thread::Builder::new()
                .name("rdp-credential-cleanup".into())
                .spawn(move || {
                    for _ in 0..DELETE_ATTEMPTS {
                        let _guard = local_lock();
                        if let Some(_process_lock) = ProcessLock::acquire(DELETE_WAIT_MS) {
                            delete_owned(&entries, &marker);
                            return;
                        }
                    }
                });
        }
    }
    pub(crate) fn cleanup_stale() -> Result<()> {
        let _guard = local_lock();
        let Some(_process_lock) = ProcessLock::acquire(DELETE_WAIT_MS) else {
            return Ok(());
        };
        let filter = wide("TERMSRV/*");
        let mut count = 0;
        let mut credentials = ptr::null_mut();
        if unsafe { CredEnumerateW(filter.as_ptr(), 0, &mut count, &mut credentials) } == 0 {
            return Ok(());
        }
        let now = now();
        for index in 0..count as usize {
            let credential = unsafe { &**credentials.add(index) };
            let comment = unsafe { string(credential.Comment) };
            // Leases from earlier builds were generic credentials; clean up both kinds.
            if !matches!(
                credential.Type,
                CRED_TYPE_GENERIC | CRED_TYPE_DOMAIN_PASSWORD
            ) || !expired_marker(&comment, now)
            {
                continue;
            }
            // LastWritten is in 100 ns ticks since 1601. Entries rewritten long after the
            // lease was created belong to someone else and are preserved.
            let written_unix =
                (filetime(&credential.LastWritten) / 10_000_000).saturating_sub(11_644_473_600);
            let created = marker_time(&comment).unwrap_or(0);
            if written_unix <= created.saturating_add(WRITE_SLACK_SECS) {
                unsafe {
                    CredDeleteW(credential.TargetName, credential.Type, 0);
                }
            }
        }
        unsafe {
            CredFree(credentials.cast());
        }
        Ok(())
    }
    fn marker_time(comment: &str) -> Option<u64> {
        comment
            .strip_prefix(PREFIX)?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    }
    fn expired_marker(comment: &str, now: u64) -> bool {
        comment
            .strip_prefix(PREFIX)
            .and_then(|s| s.split_whitespace().nth(1))
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .is_some()
            && marker_time(comment)
                .is_some_and(|created| now.saturating_sub(created) >= LEASE_TTL.as_secs())
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn cleanup_requires_our_valid_expired_marker() {
            let marker = format!("{PREFIX}100 {}", uuid::Uuid::new_v4());
            assert!(!expired_marker(&marker, 129));
            assert!(expired_marker(&marker, 130));
            assert!(!expired_marker("user credential", 1000));
            assert!(!expired_marker(&format!("{PREFIX}100 invalid"), 1000));
        }
        #[test]
        fn targets_cover_hostname_and_port_forms() {
            let default = RdpEndpoint::new("host.example", 3389).unwrap();
            assert_eq!(targets(&default), vec!["TERMSRV/host.example"]);
            let tunnel = RdpEndpoint::new("127.77.0.2", 34001).unwrap();
            assert_eq!(
                targets(&tunnel),
                vec!["TERMSRV/127.77.0.2", "TERMSRV/127.77.0.2:34001"]
            );
        }
        #[test]
        fn leases_preserve_conflicts_and_only_remove_their_own_credential() {
            let endpoint =
                RdpEndpoint::new(format!("{}.invalid", uuid::Uuid::new_v4()), 34001).unwrap();
            let lease = CredentialLease::acquire(&endpoint, "test-user", "test-password")
                .unwrap()
                .expect("temporary test credential");
            assert_eq!(lease.entries.len(), 2);
            assert!(
                CredentialLease::acquire(&endpoint, "other-user", "other-password")
                    .unwrap()
                    .is_none()
            );
            let targets = lease
                .entries
                .iter()
                .map(|entry| entry.target.clone())
                .collect::<Vec<_>>();
            drop(lease);
            for target in &targets {
                assert!(!exists(target));
            }

            // A credential rewritten by its new owner after our lease must survive.
            let lease = CredentialLease::acquire(&endpoint, "test-user", "test-password")
                .unwrap()
                .unwrap();
            let replacement = wide("Miaominal test: replacement owner");
            let user = wide("test-user");
            for target in &targets {
                let credential = CREDENTIALW {
                    Type: LEASE_TYPE,
                    TargetName: target.as_ptr().cast_mut(),
                    Comment: replacement.as_ptr().cast_mut(),
                    Persist: CRED_PERSIST_SESSION,
                    UserName: user.as_ptr().cast_mut(),
                    ..unsafe { std::mem::zeroed() }
                };
                assert_ne!(unsafe { CredWriteW(&credential, 0) }, 0);
            }
            drop(lease);
            for target in &targets {
                assert_eq!(
                    read_lease(target).map(|(comment, _)| comment).as_deref(),
                    Some("Miaominal test: replacement owner")
                );
                unsafe {
                    CredDeleteW(target.as_ptr(), LEASE_TYPE, 0);
                }
            }
        }
        #[test]
        fn process_lock_declines_a_competing_thread() {
            let _serial = local_lock();
            let _owner = ProcessLock::acquire(0).unwrap();
            assert!(
                std::thread::spawn(|| ProcessLock::acquire(0).is_none())
                    .join()
                    .unwrap()
            );
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub(crate) struct CredentialLease;
    impl CredentialLease {
        pub(crate) fn acquire(_: &RdpEndpoint, _: &str, _: &str) -> Result<Option<Self>> {
            Ok(None)
        }
    }
    pub(crate) fn cleanup_stale() -> Result<()> {
        Ok(())
    }
}
pub(crate) use platform::{CredentialLease, cleanup_stale};
