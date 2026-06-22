use std::sync::OnceLock;
use tokio::runtime::Runtime;

pub fn agent_runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::new().expect("failed to create agent tokio runtime"))
}
