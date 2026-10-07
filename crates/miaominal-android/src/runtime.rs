//! Controlled Tokio runtime for all native asynchronous work.
//!
//! Android never inherits a runtime from the caller: the process owns exactly one
//! runtime and all asynchronous bridge entry points hop onto it.

use std::sync::OnceLock;
use tokio::runtime::{Builder, Handle, Runtime};

use crate::error::BackendError;

/// The plan fixes the worker count for the mobile runtime.
const WORKER_THREADS: usize = 2;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

pub fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .worker_threads(WORKER_THREADS)
            .thread_name("miaominal-ffi")
            .enable_all()
            .build()
            .expect("the Miaominal native runtime should always be constructible")
    })
}

#[allow(dead_code)]
pub fn handle() -> Handle {
    runtime().handle().clone()
}

/// Run `work` on the process runtime and await its result.
///
/// UniFFI polls exported futures on a thread without a Tokio reactor, so any
/// future that uses timers, I/O, or `tokio::sync` waits must be spawned on the
/// process runtime instead of being awaited directly.
///
/// The spawned task is aborted if the awaiting future is dropped (for example
/// when Kotlin cancels the coroutine); dropping a `JoinHandle` alone would only
/// detach it.
pub async fn run_task<F>(work: F) -> Result<F::Output, BackendError>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    AbortOnDrop(runtime().spawn(work))
        .await
        .map_err(|error| BackendError::RuntimeUnavailable {
            runtime_failure: format!("native task failed: {error}"),
        })
}

struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl<T> std::future::Future for AbortOnDrop<T> {
    type Output = Result<T, tokio::task::JoinError>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.0).poll(context)
    }
}

/// Run blocking work on the process runtime and await its result.
pub async fn offload<F, T>(work: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    runtime()
        .spawn_blocking(work)
        .await
        .expect("blocking work on the Miaominal native runtime should not panic")
}
