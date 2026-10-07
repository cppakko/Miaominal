//! Sequenced event bus for the mobile backend.
//!
//! The bus retains a bounded window of events so that a subscriber can resume
//! from a cursor without missing anything in the common case. A consumer that
//! falls behind the retained window is told to resynchronize instead of
//! blocking the producer or growing memory without bound.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use tokio::sync::Notify;

use crate::error::BackendError;

/// Retained events kept for slow consumers before they must resynchronize.
const RETAINED_EVENTS: usize = 512;
/// Upper bound for one `next_batch` request.
const MAX_BATCH_EVENTS: u32 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ConfigArea {
    Hosts,
    Proxies,
    Snippets,
    Settings,
    Forwards,
    Credentials,
    Trust,
    Sync,
}

/// Reliable control events; terminal refreshes are pulled through
/// `SshSession::await_update`/`render_frame` and are intentionally not queued.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum BackendEvent {
    Ready,
    ConfigChanged { area: ConfigArea },
    ResyncRequired,
    Shutdown,
    SessionPhaseChanged {
        connection_id: u64,
        phase: crate::session::SshSessionPhase,
    },
    SessionRequest {
        request: crate::session::SshSessionRequest,
    },
    SessionRequestExpired {
        connection_id: u64,
        request_id: u64,
    },
    SessionExited {
        connection_id: u64,
        exit_code: u32,
        signal: Option<String>,
    },
    SftpPhaseChanged {
        connection_id: u64,
        phase: crate::sftp::SftpSessionPhase,
    },
    SftpRequest {
        request: crate::sftp::SftpRequest,
    },
    SftpRequestExpired {
        connection_id: u64,
        request_id: u64,
    },
    /// A directory listing (or listing failure) was applied; `request_id` is
    /// the id returned by `SftpSession::list_directory` (`0` for the initial
    /// listing).
    SftpDirectoryChanged {
        connection_id: u64,
        request_id: u64,
    },
    /// A transfer changed state. Progress is pulled through
    /// `SftpSession::transfers_snapshot` and intentionally not queued.
    SftpTransferChanged {
        connection_id: u64,
        transfer_id: u64,
    },
    /// A forwarding session changed phase (including reconnect state).
    ForwardPhaseChanged {
        connection_id: u64,
        profile_id: String,
        rule_id: String,
        phase: crate::forward::ForwardPhase,
    },
    /// A forwarded connection failed while the session kept running; hosts use
    /// this to refresh the pull-based snapshot without polling.
    ForwardRelayFailed {
        connection_id: u64,
        profile_id: String,
        rule_id: String,
        failure: crate::forward::ForwardFailure,
    },
    /// A forwarding session needs a host-trust or authentication answer.
    ForwardRequest {
        request: crate::forward::ForwardRequest,
    },
    ForwardRequestExpired {
        connection_id: u64,
        request_id: u64,
    },
    /// Sync phase changed (checking, pulling, pushing, paused, conflict).
    /// Progress rows are pulled through `Backend::sync_snapshot`.
    SyncPhaseChanged {
        phase: crate::sync_dto::SyncPhaseDto,
    },
    /// Local and remote content both changed; the user must pick a side.
    SyncConflictDetected {
        remote_at_ms: Option<u64>,
        reason: crate::sync_dto::SyncInterventionReasonDto,
    },
    /// Automatic sync cannot run until the user acts (vault locked, missing
    /// credentials, or a remote that failed its capability check).
    SyncAuthenticationRequired {
        detail: String,
    },
    /// A WebDAV capability probe finished; `state` is the stable verdict.
    SyncCapabilityChanged {
        state: crate::sync_dto::SyncCapabilityStateDto,
        reason: Option<crate::sync_dto::SyncCapabilityReasonDto>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SequencedEvent {
    pub sequence: u64,
    pub event: BackendEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EventBatch {
    pub events: Vec<SequencedEvent>,
    /// The requested cursor is older than the retained window; refresh the
    /// snapshot and continue from the returned sequence.
    pub resync_required: bool,
}

enum DrainOutcome {
    Events(EventBatch),
    Closed,
    Wait,
}

struct EventState {
    next_sequence: u64,
    retained: VecDeque<SequencedEvent>,
    closed: bool,
    subscribers: Vec<Weak<Notify>>,
}

pub(crate) struct EventBus {
    state: Mutex<EventState>,
}

impl EventBus {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(EventState {
                next_sequence: 1,
                retained: VecDeque::new(),
                closed: false,
                subscribers: Vec::new(),
            }),
        })
    }

    fn lock_state(&self) -> MutexGuard<'_, EventState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn publish(&self, event: BackendEvent) {
        let waiters = {
            let mut state = self.lock_state();
            if state.closed {
                log::warn!("dropping event published after the event bus closed");
                return;
            }
            let sequence = state.next_sequence;
            state.next_sequence += 1;
            state.retained.push_back(SequencedEvent { sequence, event });
            while state.retained.len() > RETAINED_EVENTS {
                state.retained.pop_front();
            }
            state.subscribers.retain(|waiter| waiter.strong_count() > 0);
            state
                .subscribers
                .iter()
                .filter_map(Weak::upgrade)
                .collect::<Vec<_>>()
        };
        for waiter in waiters {
            waiter.notify_one();
        }
    }

    /// Latest assigned sequence; `0` when nothing has been published yet.
    pub(crate) fn latest_sequence(&self) -> u64 {
        self.lock_state().next_sequence.saturating_sub(1)
    }

    /// Close the bus and wake every waiter. Repeated calls are ignored.
    pub(crate) fn close(&self) {
        let waiters = {
            let mut state = self.lock_state();
            if state.closed {
                return;
            }
            state.closed = true;
            state
                .subscribers
                .drain(..)
                .filter_map(|waiter| waiter.upgrade())
                .collect::<Vec<_>>()
        };
        for waiter in waiters {
            waiter.notify_one();
        }
    }

    pub(crate) fn subscribe(self: &Arc<Self>, since_sequence: u64) -> Arc<EventSubscription> {
        let wait = Arc::new(Notify::new());
        {
            let mut state = self.lock_state();
            state.subscribers.retain(|waiter| waiter.strong_count() > 0);
            state.subscribers.push(Arc::downgrade(&wait));
        }
        Arc::new(EventSubscription {
            inner: Arc::new(SubscriptionInner {
                bus: Arc::clone(self),
                cursor: Mutex::new(since_sequence),
                cancelled: AtomicBool::new(false),
                wait,
            }),
        })
    }
}

/// Cursor-based event subscription handed to Kotlin.
///
/// `next_batch` long-polls with a timeout and returns `Closed` once the bus is
/// closed or the subscription is cancelled. Cancellation is explicit: dropping
/// the Kotlin coroutine is not enough.
#[derive(uniffi::Object)]
pub struct EventSubscription {
    inner: Arc<SubscriptionInner>,
}

struct SubscriptionInner {
    bus: Arc<EventBus>,
    cursor: Mutex<u64>,
    cancelled: AtomicBool,
    wait: Arc<Notify>,
}

impl SubscriptionInner {
    fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::SeqCst) {
            // A stored notification wakes a not-yet-registered waiter too.
            self.wait.notify_one();
        }
    }

    async fn next_batch(
        &self,
        max_events: usize,
        timeout: Duration,
    ) -> Result<EventBatch, BackendError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.cancelled.load(Ordering::SeqCst) {
                return Err(BackendError::Closed);
            }
            let outcome = {
                let state = self.bus.lock_state();
                let mut cursor = self
                    .cursor
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                drain(&state, &mut cursor, max_events)
            };
            match outcome {
                DrainOutcome::Events(batch) => return Ok(batch),
                DrainOutcome::Closed => return Err(BackendError::Closed),
                DrainOutcome::Wait => {}
            }

            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Ok(EventBatch {
                    events: Vec::new(),
                    resync_required: false,
                });
            }
            // `notify_one` stores a permit when nobody is waiting, so a publish
            // or cancel between the state check above and this await cannot be
            // lost.
            tokio::select! {
                () = self.wait.notified() => {}
                () = tokio::time::sleep(remaining) => {}
            }
        }
    }
}

fn drain(state: &EventState, cursor: &mut u64, max_events: usize) -> DrainOutcome {
    let oldest = state
        .retained
        .front()
        .map(|event| event.sequence)
        .unwrap_or(state.next_sequence);
    let mut resync_required = false;
    if *cursor < oldest.saturating_sub(1) {
        resync_required = true;
        *cursor = oldest.saturating_sub(1);
    }
    let mut events = Vec::new();
    for event in &state.retained {
        if event.sequence <= *cursor {
            continue;
        }
        if events.len() >= max_events {
            break;
        }
        events.push(event.clone());
        *cursor = event.sequence;
    }
    if resync_required || !events.is_empty() {
        return DrainOutcome::Events(EventBatch {
            events,
            resync_required,
        });
    }
    if state.closed {
        DrainOutcome::Closed
    } else {
        DrainOutcome::Wait
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl EventSubscription {
    /// Wait for events after the subscription cursor.
    ///
    /// Returns an empty batch when `timeout_ms` elapses, `resync_required` when
    /// part of the requested range was evicted, and `Closed` after the backend
    /// shuts down or the subscription is cancelled.
    pub async fn next_batch(
        &self,
        max_events: u32,
        timeout_ms: u64,
    ) -> Result<EventBatch, BackendError> {
        let max_events = max_events.clamp(1, MAX_BATCH_EVENTS) as usize;
        let timeout = Duration::from_millis(timeout_ms.max(1));
        let inner = Arc::clone(&self.inner);
        crate::runtime::run_task(async move {
            inner.next_batch(max_events, timeout).await
        })
        .await?
    }

    /// Explicit cancellation: wakes any waiter and is idempotent.
    pub fn cancel(&self) {
        self.inner.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime;

    fn sequenced(sequence: u64, event: BackendEvent) -> SequencedEvent {
        SequencedEvent { sequence, event }
    }

    #[tokio::test]
    async fn batch_delivers_events_after_the_cursor() {
        let bus = EventBus::new();
        bus.publish(BackendEvent::Ready);
        bus.publish(BackendEvent::ConfigChanged {
            area: ConfigArea::Hosts,
        });
        let subscription = bus.subscribe(0);

        let batch = subscription.next_batch(16, 50)
            .await.unwrap();

        assert!(!batch.resync_required);
        assert_eq!(
            batch.events,
            vec![
                sequenced(1, BackendEvent::Ready),
                sequenced(
                    2,
                    BackendEvent::ConfigChanged {
                        area: ConfigArea::Hosts
                    }
                ),
            ]
        );
    }

    #[tokio::test]
    async fn cursor_resumes_without_redelivering_events() {
        let bus = EventBus::new();
        bus.publish(BackendEvent::Ready);
        let subscription = bus.subscribe(1);
        bus.publish(BackendEvent::ConfigChanged {
            area: ConfigArea::Settings,
        });

        let batch = subscription.next_batch(16, 50)
            .await.unwrap();

        assert_eq!(
            batch.events,
            vec![sequenced(
                2,
                BackendEvent::ConfigChanged {
                    area: ConfigArea::Settings
                }
            )]
        );
    }

    #[tokio::test]
    async fn next_batch_respects_the_maximum_batch_size() {
        let bus = EventBus::new();
        for _ in 0..5 {
            bus.publish(BackendEvent::Ready);
        }
        let subscription = bus.subscribe(0);

        let first = subscription.next_batch(2, 50)
            .await.unwrap();
        let second = subscription.next_batch(2, 50)
            .await.unwrap();
        let third = subscription.next_batch(2, 50)
            .await.unwrap();

        assert_eq!(first.events.len(), 2);
        assert_eq!(second.events.len(), 2);
        assert_eq!(third.events.len(), 1);
        assert_eq!(third.events[0].sequence, 5);
    }

    #[tokio::test]
    async fn slow_consumer_is_told_to_resynchronize() {
        let bus = EventBus::new();
        for _ in 0..(RETAINED_EVENTS as u64 + 8) {
            bus.publish(BackendEvent::Ready);
        }
        let subscription = bus.subscribe(0);

        let batch = subscription.next_batch(16, 50)
            .await.unwrap();

        assert!(batch.resync_required);
        assert_eq!(batch.events.len(), 16);
        // The first eight events were evicted; the window starts at sequence 9.
        assert_eq!(batch.events[0].sequence, 9);
    }

    #[tokio::test]
    async fn waiting_subscriber_is_woken_by_a_new_event() {
        let bus = EventBus::new();
        let subscription = bus.subscribe(0);
        // Spawn on the process runtime: `next_batch` borrows the subscription
        // across awaits, so it must own its handle inside the task.
        let waiter = runtime::runtime().spawn(async move {
            subscription.next_batch(16, 5_000).await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        bus.publish(BackendEvent::Ready);

        let batch = tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("subscriber should not time out")
            .expect("waiter task should not panic")
            .expect("batch should be delivered");

        assert_eq!(batch.events, vec![sequenced(1, BackendEvent::Ready)]);
    }

    #[tokio::test]
    async fn bus_close_wakes_waiters_and_is_idempotent() {
        let bus = EventBus::new();
        let subscription = bus.subscribe(0);
        let waiter = runtime::runtime().spawn(async move {
            subscription.next_batch(16, 30_000).await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        bus.close();
        bus.close();

        let result = tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("closing the bus should wake the waiter")
            .expect("waiter task should not panic");

        assert!(matches!(result, Err(BackendError::Closed)));
    }

    #[tokio::test]
    async fn published_events_are_delivered_before_the_close_is_reported() {
        let bus = EventBus::new();
        let subscription = bus.subscribe(0);
        let waiter = runtime::runtime().spawn(async move {
            subscription.next_batch(16, 30_000).await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        bus.publish(BackendEvent::Shutdown);
        bus.close();

        let result = tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("closing the bus should wake the waiter")
            .expect("waiter task should not panic")
            .expect("pending events should be delivered");

        assert_eq!(result.events, vec![sequenced(1, BackendEvent::Shutdown)]);
    }

    #[tokio::test]
    async fn cancel_wakes_waiters_and_is_idempotent() {
        let bus = EventBus::new();
        let subscription = bus.subscribe(0);
        let waiter = runtime::runtime().spawn({
            let subscription = Arc::clone(&subscription);
            async move { subscription.next_batch(16, 30_000).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        subscription.cancel();
        subscription.cancel();

        let result = tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("cancelling should wake the waiter")
            .expect("waiter task should not panic");

        assert!(matches!(result, Err(BackendError::Closed)));
        assert!(subscription.is_cancelled());
    }

    #[tokio::test]
    async fn events_published_after_close_are_dropped() {
        let bus = EventBus::new();
        let subscription = bus.subscribe(0);
        bus.close();
        bus.publish(BackendEvent::Ready);

        let result = subscription.next_batch(16, 50).await;

        assert!(matches!(result, Err(BackendError::Closed)));
        assert_eq!(bus.latest_sequence(), 0);
    }

    #[tokio::test]
    async fn empty_batch_is_returned_after_the_timeout() {
        let bus = EventBus::new();
        let subscription = bus.subscribe(0);
        let started = std::time::Instant::now();

        let batch = subscription.next_batch(16, 30).await.unwrap();

        assert!(batch.events.is_empty());
        assert!(!batch.resync_required);
        assert!(started.elapsed() >= Duration::from_millis(20));
    }

    #[tokio::test]
    async fn subscription_cursor_survives_late_events() {
        let bus = EventBus::new();
        let subscription = bus.subscribe(0);
        bus.publish(BackendEvent::Ready);
        bus.publish(BackendEvent::Shutdown);

        let batch = subscription.next_batch(16, 50)
            .await.unwrap();

        assert_eq!(batch.events.len(), 2);
        assert_eq!(batch.events[1].event, BackendEvent::Shutdown);
        assert_eq!(bus.latest_sequence(), 2);
    }
}
