use crate::error::AgentError;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::watch;

#[derive(Clone)]
pub struct AgentToolCancellation {
    inner: Arc<AgentToolCancellationInner>,
}

struct AgentToolCancellationInner {
    stop: watch::Sender<bool>,
    active_workers: watch::Sender<usize>,
    state: StdMutex<AgentToolCancellationState>,
}

#[derive(Default)]
struct AgentToolCancellationState {
    cancelled: bool,
    next_worker_id: u64,
    workers: HashMap<u64, AgentToolWorkerRecord>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentToolWorkerPhase {
    Reserved,
    Started,
}

struct AgentToolWorkerRecord {
    phase: AgentToolWorkerPhase,
    abort_handle: Option<tokio::task::AbortHandle>,
}

struct AgentToolWorkerPermit {
    cancellation: AgentToolCancellation,
    worker_id: u64,
}

impl Drop for AgentToolWorkerPermit {
    fn drop(&mut self) {
        let mut state = self
            .cancellation
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.workers.remove(&self.worker_id);
        self.cancellation
            .inner
            .active_workers
            .send_replace(state.workers.len());
    }
}

impl AgentToolCancellation {
    pub(crate) fn new() -> Self {
        let (stop, _) = watch::channel(false);
        let (active_workers, _) = watch::channel(0);
        Self {
            inner: Arc::new(AgentToolCancellationInner {
                stop,
                active_workers,
                state: StdMutex::new(AgentToolCancellationState::default()),
            }),
        }
    }

    fn reserve_worker(
        &self,
    ) -> Result<(u64, AgentToolWorkerPermit, watch::Receiver<bool>), AgentError> {
        let worker_id = {
            let mut state = self.inner.state.lock().map_err(|_| {
                AgentError::Backend(anyhow::anyhow!("tool cancellation lock poisoned"))
            })?;
            if state.cancelled {
                return Err(tool_cancelled_error());
            }
            state.next_worker_id = state.next_worker_id.wrapping_add(1).max(1);
            let worker_id = state.next_worker_id;
            state.workers.insert(
                worker_id,
                AgentToolWorkerRecord {
                    phase: AgentToolWorkerPhase::Reserved,
                    abort_handle: None,
                },
            );
            self.inner.active_workers.send_replace(state.workers.len());
            worker_id
        };
        Ok((
            worker_id,
            AgentToolWorkerPermit {
                cancellation: self.clone(),
                worker_id,
            },
            self.inner.stop.subscribe(),
        ))
    }

    fn register_worker(&self, worker_id: u64, abort_handle: tokio::task::AbortHandle) {
        let should_abort = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let cancelled = state.cancelled;
            let Some(worker) = state.workers.get_mut(&worker_id) else {
                return;
            };
            worker.abort_handle = Some(abort_handle.clone());
            cancelled && worker.phase == AgentToolWorkerPhase::Reserved
        };
        if should_abort {
            abort_handle.abort();
        }
    }

    fn start_worker(&self, worker_id: u64) -> Result<(), AgentError> {
        let mut state =
            self.inner.state.lock().map_err(|_| {
                AgentError::Backend(anyhow::anyhow!("tool cancellation lock poisoned"))
            })?;
        if state.cancelled {
            return Err(tool_cancelled_error());
        }
        let worker = state.workers.get_mut(&worker_id).ok_or_else(|| {
            AgentError::Backend(anyhow::anyhow!("reserved tool worker disappeared"))
        })?;
        if worker.phase != AgentToolWorkerPhase::Reserved {
            return Err(AgentError::Backend(anyhow::anyhow!(
                "tool worker started more than once"
            )));
        }
        worker.phase = AgentToolWorkerPhase::Started;
        Ok(())
    }

    pub fn cancel(&self) {
        let abort_handles = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.cancelled = true;
            state
                .workers
                .values()
                .filter(|worker| worker.phase == AgentToolWorkerPhase::Reserved)
                .filter_map(|worker| worker.abort_handle.clone())
                .collect::<Vec<_>>()
        };
        self.inner.stop.send_replace(true);
        for abort_handle in abort_handles {
            // This prevents queued spawn_blocking work from starting. A worker which has already
            // started observes the stop watch inside its current-thread runtime.
            abort_handle.abort();
        }
    }

    pub async fn cancel_and_wait(&self) {
        let mut active_workers = self.inner.active_workers.subscribe();
        self.cancel();

        loop {
            if *active_workers.borrow_and_update() == 0 {
                return;
            }
            if active_workers.changed().await.is_err() {
                return;
            }
        }
    }

    pub(crate) async fn cancelled(&self) {
        let mut stop = self.inner.stop.subscribe();
        wait_for_tool_cancellation(&mut stop).await;
    }

    pub(crate) fn is_cancelled_runtime(&self) -> bool {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .cancelled
    }

    #[cfg(test)]
    pub(crate) fn is_cancelled(&self) -> bool {
        self.is_cancelled_runtime()
    }
}

struct AgentToolWorkerJoin<T: Send + 'static> {
    handle: Option<tokio::task::JoinHandle<T>>,
    cancellation: AgentToolCancellation,
    completed: bool,
}

impl<T: Send + 'static> AgentToolWorkerJoin<T> {
    fn new(handle: tokio::task::JoinHandle<T>, cancellation: AgentToolCancellation) -> Self {
        Self {
            handle: Some(handle),
            cancellation,
            completed: false,
        }
    }

    fn abort_handle(&self) -> tokio::task::AbortHandle {
        self.handle
            .as_ref()
            .expect("worker handle should exist before completion")
            .abort_handle()
    }

    async fn join(&mut self) -> Result<T, tokio::task::JoinError> {
        let result = self
            .handle
            .as_mut()
            .expect("worker handle should exist while joining")
            .await;
        self.completed = true;
        result
    }
}

impl<T: Send + 'static> Drop for AgentToolWorkerJoin<T> {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        self.cancellation.cancel();
        let Some(handle) = self.handle.take() else {
            return;
        };
        handle.abort();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            // Keep ownership of a started spawn_blocking worker until its cooperative stop path
            // has returned. This is a fallback for callers which drop the rig tool future without
            // first going through the chat receiver cancellation path.
            runtime.spawn(async move {
                let _ = handle.await;
            });
        }
    }
}

pub(crate) async fn run_cancellable_tool_worker<T, Build, Work>(
    cancellation: AgentToolCancellation,
    build_work: Build,
    armed: Option<tokio::sync::oneshot::Sender<()>>,
) -> Result<T, AgentError>
where
    T: Send + 'static,
    Build: FnOnce() -> Work + Send + 'static,
    Work: Future<Output = Result<T, AgentError>> + 'static,
{
    let (worker_id, worker_permit, stop) = cancellation.reserve_worker()?;
    let worker_cancellation = cancellation.clone();
    let handle = tokio::task::spawn_blocking(move || {
        let _worker_permit = worker_permit;
        execute_cancellable_tool_worker(&worker_cancellation, worker_id, stop, build_work, armed)
    });
    let mut worker = AgentToolWorkerJoin::new(handle, cancellation.clone());
    cancellation.register_worker(worker_id, worker.abort_handle());

    match worker.join().await {
        Ok(result) => result,
        Err(_error) if cancellation.is_cancelled_runtime() => Err(tool_cancelled_error()),
        Err(error) => Err(AgentError::Backend(anyhow::anyhow!(
            "agent tool worker failed: {error}"
        ))),
    }
}

fn execute_cancellable_tool_worker<T, Build, Work>(
    cancellation: &AgentToolCancellation,
    worker_id: u64,
    mut stop: watch::Receiver<bool>,
    build_work: Build,
    armed: Option<tokio::sync::oneshot::Sender<()>>,
) -> Result<T, AgentError>
where
    Build: FnOnce() -> Work,
    Work: Future<Output = Result<T, AgentError>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| AgentError::Backend(error.into()))?;
    // This transition and `cancel` use the same lock. Keep it immediately before `block_on`, after
    // all avoidable setup: if Stop wins while the worker is Reserved, return before constructing or
    // polling the side-effecting work future. Once this wins, the worker is Started and retains
    // completion-first arbitration below.
    cancellation.start_worker(worker_id)?;
    runtime.block_on(async move {
        let work = build_work();
        tokio::pin!(work);
        tokio::select! {
            biased;
            result = &mut work => result,
            _ = wait_for_tool_cancellation(&mut stop) => Err(tool_cancelled_error()),
            _ = signal_armed(armed) => unreachable!("the armed signal never completes"),
        }
    })
}

async fn signal_armed(armed: Option<tokio::sync::oneshot::Sender<()>>) {
    if let Some(armed) = armed {
        let _ = armed.send(());
    }
    std::future::pending::<()>().await
}

pub(crate) async fn wait_for_tool_cancellation(stop: &mut watch::Receiver<bool>) {
    loop {
        if *stop.borrow_and_update() {
            return;
        }
        if stop.changed().await.is_err() {
            return;
        }
    }
}

pub(crate) fn tool_cancelled_error() -> AgentError {
    AgentError::Cancelled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancellation_waits_for_running_blocking_tool_workers() {
        struct DropFlag(std::sync::Arc<std::sync::atomic::AtomicBool>);

        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::Release);
            }
        }

        let cancellation = AgentToolCancellation::new();
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (started_sender, started_receiver) = tokio::sync::oneshot::channel();
        let worker_dropped = dropped.clone();
        let worker_cancellation = cancellation.clone();
        let task = tokio::spawn(async move {
            run_cancellable_tool_worker(
                worker_cancellation,
                move || async move {
                    let _drop_flag = DropFlag(worker_dropped);
                    let _ = started_sender.send(());
                    std::future::pending::<Result<(), AgentError>>().await
                },
                None,
            )
            .await
        });
        started_receiver
            .await
            .expect("blocking tool worker should start");

        cancellation.cancel_and_wait().await;

        assert!(cancellation.is_cancelled());
        assert!(dropped.load(std::sync::atomic::Ordering::Acquire));
        let result = task.await.expect("tool wrapper task should finish");
        assert!(result.is_err());
        assert!(
            result
                .expect_err("cancelled worker should return an error")
                .to_string()
                .contains("cancelled")
        );
    }

    #[test]
    fn cancellation_between_reserve_and_first_poll_never_builds_work() {
        let cancellation = AgentToolCancellation::new();
        let (worker_id, worker_permit, stop) = cancellation
            .reserve_worker()
            .expect("worker reservation should succeed");
        let build_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        cancellation.cancel();

        let build_called_by_worker = build_called.clone();
        let result = execute_cancellable_tool_worker(
            &cancellation,
            worker_id,
            stop,
            move || {
                build_called_by_worker.store(true, std::sync::atomic::Ordering::Release);
                std::future::ready(Ok::<_, AgentError>(()))
            },
            None,
        );
        drop(worker_permit);

        assert!(matches!(result, Err(AgentError::Cancelled)));
        assert!(!build_called.load(std::sync::atomic::Ordering::Acquire));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn running_blocking_worker_cancelled_while_reserved_never_builds_work() {
        let cancellation = AgentToolCancellation::new();
        let (worker_id, worker_permit, stop) = cancellation
            .reserve_worker()
            .expect("worker reservation should succeed");
        let (entered_sender, entered_receiver) = tokio::sync::oneshot::channel();
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        let build_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let build_called_by_worker = build_called.clone();
        let worker_cancellation = cancellation.clone();
        let handle = tokio::task::spawn_blocking(move || {
            let _worker_permit = worker_permit;
            let _ = entered_sender.send(());
            release_receiver
                .recv()
                .expect("reserved worker should be released by the test");
            execute_cancellable_tool_worker(
                &worker_cancellation,
                worker_id,
                stop,
                move || {
                    build_called_by_worker.store(true, std::sync::atomic::Ordering::Release);
                    std::future::ready(Ok::<_, AgentError>(()))
                },
                None,
            )
        });
        let mut worker = AgentToolWorkerJoin::new(handle, cancellation.clone());
        cancellation.register_worker(worker_id, worker.abort_handle());
        entered_receiver
            .await
            .expect("spawn_blocking worker should already be running");

        cancellation.cancel();
        release_sender
            .send(())
            .expect("running blocking worker should still receive its release");

        let result = worker
            .join()
            .await
            .expect("a running spawn_blocking worker should not be abortable");
        assert!(matches!(result, Err(AgentError::Cancelled)));
        assert!(!build_called.load(std::sync::atomic::Ordering::Acquire));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn completed_worker_wins_stop_and_the_next_worker_never_starts() {
        struct CompleteWhenReleased {
            ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
        }

        impl std::future::Future for CompleteWhenReleased {
            type Output = Result<&'static str, AgentError>;

            fn poll(
                self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Self::Output> {
                if self.ready.load(std::sync::atomic::Ordering::Acquire) {
                    std::task::Poll::Ready(Ok("first completed"))
                } else {
                    std::task::Poll::Pending
                }
            }
        }

        let cancellation = AgentToolCancellation::new();
        let ready = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (armed_sender, armed_receiver) = tokio::sync::oneshot::channel();
        let first_cancellation = cancellation.clone();
        let first_ready = ready.clone();
        let first = tokio::spawn(async move {
            run_cancellable_tool_worker(
                first_cancellation,
                move || CompleteWhenReleased { ready: first_ready },
                Some(armed_sender),
            )
            .await
        });
        armed_receiver
            .await
            .expect("first tool worker should arm its cancellation watcher");

        // The armed signal fires only after the cancellation watcher registered its first poll, so
        // the completion below and Stop are both observable on the same worker poll. The completed
        // result wins, while the cancellation state still rejects every later reservation.
        ready.store(true, std::sync::atomic::Ordering::Release);
        cancellation.cancel();

        assert_eq!(
            first
                .await
                .expect("first tool wrapper should finish")
                .unwrap(),
            "first completed"
        );

        let second_started = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let second_started_in_worker = second_started.clone();
        let second = run_cancellable_tool_worker(
            cancellation,
            move || {
                second_started_in_worker.store(true, std::sync::atomic::Ordering::Release);
                std::future::ready(Ok::<_, AgentError>("second completed"))
            },
            None,
        )
        .await;

        assert!(matches!(second, Err(AgentError::Cancelled)));
        assert!(!second_started.load(std::sync::atomic::Ordering::Acquire));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_tool_future_cancels_and_reaps_the_blocking_worker() {
        struct DropFlag(Option<tokio::sync::oneshot::Sender<()>>);

        impl Drop for DropFlag {
            fn drop(&mut self) {
                if let Some(sender) = self.0.take() {
                    let _ = sender.send(());
                }
            }
        }

        let cancellation = AgentToolCancellation::new();
        let (started_sender, started_receiver) = tokio::sync::oneshot::channel();
        let (dropped_sender, dropped_receiver) = tokio::sync::oneshot::channel();
        let task = tokio::spawn({
            let cancellation = cancellation.clone();
            async move {
                run_cancellable_tool_worker(
                    cancellation,
                    move || async move {
                        let _drop_flag = DropFlag(Some(dropped_sender));
                        let _ = started_sender.send(());
                        std::future::pending::<Result<(), AgentError>>().await
                    },
                    None,
                )
                .await
            }
        });
        started_receiver
            .await
            .expect("blocking tool worker should start");

        task.abort();
        let _ = task.await;

        tokio::time::timeout(std::time::Duration::from_secs(1), dropped_receiver)
            .await
            .expect("dropped rig future should reap its blocking worker")
            .expect("blocking worker should report future drop");
        assert!(cancellation.is_cancelled());
    }
}
