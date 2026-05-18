use std::sync::OnceLock;
use tokio::runtime::{Handle as TokioHandle, Runtime};

static TOKIO_HANDLE: OnceLock<TokioHandle> = OnceLock::new();

pub(crate) fn start_tokio() -> TokioHandle {
    if let Some(handle) = TOKIO_HANDLE.get() {
        return handle.clone();
    }

    let (sender, receiver) = std::sync::mpsc::sync_channel::<TokioHandle>(0);

    std::thread::Builder::new()
        .name("tokio-runtime".into())
        .spawn(move || {
            let runtime = Runtime::new().expect("failed to start tokio runtime");
            sender
                .send(runtime.handle().clone())
                .expect("failed to send tokio handle");

            runtime.block_on(async {
                std::future::pending::<()>().await;
            });
        })
        .expect("failed to spawn tokio thread");

    let handle = receiver.recv().expect("failed to receive tokio handle");
    TOKIO_HANDLE
        .set(handle.clone())
        .expect("tokio handle initialized twice");
    handle
}
