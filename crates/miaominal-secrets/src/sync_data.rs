use std::cell::Cell;
use std::sync::{Mutex, MutexGuard};

static SYNC_DATA_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    static SYNC_DATA_LOCK_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// Reentrant process-wide gate for mutations that participate in the sync
/// payload. Callers hold it only while reading or committing local stores and
/// never across network awaits.
pub struct SyncDataGuard {
    _outer: Option<MutexGuard<'static, ()>>,
}

impl Drop for SyncDataGuard {
    fn drop(&mut self) {
        SYNC_DATA_LOCK_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

pub fn lock_sync_data() -> SyncDataGuard {
    let nested = SYNC_DATA_LOCK_DEPTH.with(|depth| {
        let current = depth.get();
        if current > 0 {
            depth.set(current + 1);
            true
        } else {
            false
        }
    });
    if nested {
        return SyncDataGuard { _outer: None };
    }

    let outer = SYNC_DATA_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    SYNC_DATA_LOCK_DEPTH.with(|depth| depth.set(1));
    SyncDataGuard {
        _outer: Some(outer),
    }
}
