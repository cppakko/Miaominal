use super::{TOOL_NAMES, tool_description};
use crate::backend::BackendRoute;
use crate::channel::{AgentExecChannel, AgentToolCallRequest, AgentToolCallResponse, ToolOutput};
use crate::chat::AgentMode;
use crate::error::AgentError;
use rig_core::completion::ToolDefinition;
use rig_core::tool::{Tool, ToolDyn, ToolSet};
use rig_core::wasm_compat::WasmCompatSend;
use serde_json::Map;
use serde_json::{Value, json};
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

#[derive(Clone)]
pub struct AgentToolSet {
    channel: AgentExecChannel,
    mode: AgentMode,
    cancellation: AgentToolCancellation,
}

impl AgentToolSet {
    pub fn for_channel(channel: AgentExecChannel, mode: AgentMode) -> Self {
        Self {
            channel,
            mode,
            cancellation: AgentToolCancellation::new(),
        }
    }

    pub fn mode(&self) -> AgentMode {
        self.mode
    }

    pub fn cancellation(&self) -> AgentToolCancellation {
        self.cancellation.clone()
    }

    pub fn into_rig_tool_set(self) -> ToolSet {
        let mut toolset = ToolSet::default();
        for name in self.enabled_tool_names() {
            toolset.add_tool(JsonAgentTool {
                name: name.to_string(),
                channel: self.channel.clone(),
                mode: self.mode,
                cancellation: self.cancellation.clone(),
            });
        }
        toolset
    }

    pub fn into_rig_tools(self) -> Vec<Box<dyn ToolDyn>> {
        self.enabled_tool_names()
            .into_iter()
            .map(|name| {
                Box::new(JsonAgentTool {
                    name: name.to_string(),
                    channel: self.channel.clone(),
                    mode: self.mode,
                    cancellation: self.cancellation.clone(),
                }) as Box<dyn ToolDyn>
            })
            .collect()
    }

    pub async fn definitions(&self) -> Vec<ToolDefinition> {
        self.enabled_tool_names()
            .into_iter()
            .map(tool_definition)
            .collect()
    }

    fn enabled_tool_names(&self) -> Vec<&'static str> {
        let all = TOOL_NAMES
            .iter()
            .copied()
            .filter(|name| *name != "web_search" || self.channel.web_search_enabled());

        match self.mode {
            AgentMode::Ask => all
                .filter(|name| {
                    matches!(
                        *name,
                        "workspace_info"
                            | "read"
                            | "list"
                            | "glob"
                            | "grep"
                            | "web_search"
                            | "web_fetch"
                            | "ask_user"
                    )
                })
                .collect(),
            _ => all.collect(),
        }
    }
}

#[derive(Clone)]
struct JsonAgentTool {
    name: String,
    channel: AgentExecChannel,
    mode: AgentMode,
    cancellation: AgentToolCancellation,
}

impl Tool for JsonAgentTool {
    const NAME: &'static str = "miaominal_agent_tool";

    type Error = AgentError;
    type Args = Value;
    type Output = String;

    fn name(&self) -> String {
        self.name.clone()
    }

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        tool_definition(&self.name)
    }

    fn call(
        &self,
        args: Self::Args,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + WasmCompatSend {
        let channel = self.channel.clone();
        let name = self.name.clone();
        let approved = initial_tool_approval(self.mode, &name);
        let skip_policy = matches!(self.mode, AgentMode::NonBlocking | AgentMode::FullAuto);
        let mode = self.mode;
        let cancellation = self.cancellation.clone();
        async move {
            if mode_requires_confirmation(mode, &name) {
                let response = AgentToolCallResponse {
                    tool_name: name.clone(),
                    route: BackendRoute::SshExec,
                    output: ToolOutput::Approval {
                        message: format!("tool `{name}` requires user approval"),
                        operation_hash: None,
                    },
                };
                return serde_json::to_string(&response)
                    .map_err(|error| AgentError::InvalidArguments(error.to_string()));
            }
            let response = match call_tool_on_worker(
                channel,
                cancellation,
                AgentToolCallRequest {
                    tool_name: name.clone(),
                    arguments: normalize_tool_arguments(args),
                    approved,
                    route: None,
                    skip_policy,
                },
            )
            .await
            {
                Ok(response) => response,
                Err(AgentError::ApprovalRequired { tool_name }) => AgentToolCallResponse {
                    tool_name: tool_name.clone(),
                    route: BackendRoute::SshExec,
                    output: ToolOutput::Approval {
                        message: format!("tool `{tool_name}` requires user approval"),
                        operation_hash: None,
                    },
                },
                Err(AgentError::Cancelled) => AgentToolCallResponse {
                    tool_name: name.clone(),
                    route: BackendRoute::SshExec,
                    output: ToolOutput::Cancelled,
                },
                Err(error) => return Err(error),
            };
            serde_json::to_string(&response)
                .map_err(|error| AgentError::InvalidArguments(error.to_string()))
        }
    }
}

async fn call_tool_on_worker(
    channel: AgentExecChannel,
    cancellation: AgentToolCancellation,
    request: AgentToolCallRequest,
) -> Result<crate::AgentToolCallResponse, AgentError> {
    run_cancellable_tool_worker(cancellation, move || async move {
        channel.call_tool(request).await
    })
    .await
}

async fn run_cancellable_tool_worker<T, Build, Work>(
    cancellation: AgentToolCancellation,
    build_work: Build,
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
        execute_cancellable_tool_worker(&worker_cancellation, worker_id, stop, build_work)
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
        }
    })
}

async fn wait_for_tool_cancellation(stop: &mut watch::Receiver<bool>) {
    loop {
        if *stop.borrow_and_update() {
            return;
        }
        if stop.changed().await.is_err() {
            return;
        }
    }
}

fn tool_cancelled_error() -> AgentError {
    AgentError::Cancelled
}

fn tool_definition(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: tool_description(name).to_string(),
        parameters: tool_parameters(name),
    }
}

fn auto_approve_rig_tool(name: &str) -> bool {
    matches!(name, "web_search")
}

fn initial_tool_approval(mode: AgentMode, name: &str) -> bool {
    match mode {
        AgentMode::FullAuto => true,
        AgentMode::Ask | AgentMode::NonBlocking => false,
        AgentMode::Execute => auto_approve_rig_tool(name),
    }
}

fn mode_requires_confirmation(mode: AgentMode, name: &str) -> bool {
    name != "ask_user" && matches!(mode, AgentMode::Ask | AgentMode::NonBlocking)
}

fn normalize_tool_arguments(arguments: Value) -> Value {
    if let Value::Object(mut object) = arguments {
        if object.len() == 1
            && let Some(arguments) = object.remove("arguments")
        {
            return arguments;
        }
        return Value::Object(object);
    }

    arguments
}

fn object_schema(properties: Vec<(&str, Value)>, required: &[&str]) -> Value {
    let properties = properties
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect::<Map<String, Value>>();
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn string_schema(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn target_schema() -> Value {
    string_schema(
        "Optional execution target from the user's @mentions, such as @Server or @Server (2). Omit to use the current session.",
    )
}

fn integer_schema(description: &str, minimum: usize) -> Value {
    json!({ "type": "integer", "minimum": minimum, "description": description })
}

fn boolean_schema(description: &str) -> Value {
    json!({ "type": "boolean", "description": description })
}

fn ask_user_choice_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "label": string_schema("Short option label shown as a selectable response."),
            "description": string_schema("Optional one-sentence explanation of this option.")
        },
        "required": ["label"],
        "additionalProperties": false,
    })
}

fn string_array_schema(description: &str) -> Value {
    json!({
        "type": "array",
        "items": { "type": "string" },
        "description": description,
    })
}

fn tool_parameters(name: &str) -> Value {
    match name {
        "workspace_info" | "list_jobs" => object_schema(vec![("target", target_schema())], &[]),
        "approval" => object_schema(Vec::new(), &[]),
        "read" => object_schema(
            vec![
                ("path", string_schema("Remote workspace file path to read.")),
                ("target", target_schema()),
                (
                    "start_line",
                    integer_schema("First 1-based line to read.", 1),
                ),
                ("end_line", integer_schema("Last 1-based line to read.", 1)),
                ("max_bytes", integer_schema("Maximum bytes to return.", 1)),
            ],
            &["path"],
        ),
        "list" => object_schema(
            vec![
                ("path", string_schema("Remote workspace directory path.")),
                ("target", target_schema()),
                ("include_hidden", boolean_schema("Include dotfiles.")),
                (
                    "max_entries",
                    integer_schema("Maximum entries to return.", 1),
                ),
            ],
            &[],
        ),
        "glob" => object_schema(
            vec![
                (
                    "root",
                    string_schema("Narrow remote workspace root to search."),
                ),
                ("target", target_schema()),
                ("pattern", string_schema("Glob-style filename pattern.")),
                ("max_results", integer_schema("Maximum matching paths.", 1)),
                ("include_hidden", boolean_schema("Include hidden paths.")),
            ],
            &["pattern"],
        ),
        "grep" => object_schema(
            vec![
                ("pattern", string_schema("Regex pattern to search for.")),
                (
                    "root",
                    string_schema("Narrow remote workspace root to search."),
                ),
                ("target", target_schema()),
                ("include", string_array_schema("File globs to include.")),
                ("max_results", integer_schema("Maximum matching lines.", 1)),
                ("max_bytes", integer_schema("Maximum bytes to return.", 1)),
                (
                    "case_insensitive",
                    boolean_schema("Use case-insensitive matching."),
                ),
            ],
            &["pattern"],
        ),
        "apply_patch" => object_schema(
            vec![
                ("patch", string_schema("Unified diff patch to apply.")),
                (
                    "base_dir",
                    string_schema("Remote workspace directory for patch."),
                ),
                ("target", target_schema()),
                (
                    "validator",
                    json!({
                        "type": "object",
                        "properties": {
                            "command": string_schema("Validation command to run after patch.")
                        },
                        "required": ["command"],
                        "additionalProperties": false,
                    }),
                ),
            ],
            &["patch"],
        ),
        "run_shell" => object_schema(
            vec![
                ("command", string_schema("Non-interactive shell command.")),
                ("target", target_schema()),
                ("cwd", string_schema("Remote workspace directory.")),
                ("timeout_seconds", integer_schema("Timeout in seconds.", 1)),
                (
                    "max_bytes",
                    integer_schema("Maximum stdout/stderr bytes.", 1),
                ),
                (
                    "shell",
                    string_schema("Shell label; use posix-sh, fish, powershell, or cmd."),
                ),
            ],
            &["command"],
        ),
        "start_job" => object_schema(
            vec![
                ("command", string_schema("Long-running shell command.")),
                ("target", target_schema()),
                ("cwd", string_schema("Remote workspace directory.")),
            ],
            &["command"],
        ),
        "poll_job" | "stop_job" => object_schema(
            vec![
                ("job_id", string_schema("Job id returned by start_job.")),
                ("target", target_schema()),
            ],
            &["job_id"],
        ),
        "web_search" => object_schema(vec![("query", string_schema("Search query."))], &["query"]),
        "web_fetch" => object_schema(
            vec![
                ("url", string_schema("URL to fetch.")),
                (
                    "max_bytes",
                    integer_schema("Maximum text bytes to return.", 1),
                ),
            ],
            &["url"],
        ),
        "ask_user" => object_schema(
            vec![
                (
                    "message",
                    string_schema("Question or request shown to the user."),
                ),
                (
                    "choices",
                    json!({
                        "type": "array",
                        "items": ask_user_choice_schema(),
                        "maxItems": 3,
                        "description": "Up to three suggested responses for the user to choose from.",
                    }),
                ),
                (
                    "operation_hash",
                    string_schema("Optional operation identifier."),
                ),
            ],
            &["message"],
        ),
        _ => object_schema(Vec::new(), &[]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::profile::SessionProfile;
    use miaominal_secrets::SecretStore;
    use miaominal_settings::{WebSearchConfig, WebSearchProviderKind};
    use miaominal_storage::known_hosts_store::KnownHostsStore;

    #[tokio::test]
    async fn all_tool_definitions_are_generated() {
        let mut profile = SessionProfile::blank("session-1", 1);
        profile.host = "example.com".into();
        profile.username = "akko".into();
        let channel = AgentExecChannel::for_profile(
            profile,
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-known-hosts-tools")),
        );
        let definitions = AgentToolSet::for_channel(channel, AgentMode::Execute)
            .definitions()
            .await;

        assert_eq!(definitions.len(), TOOL_NAMES.len() - 1);
        for name in TOOL_NAMES {
            if *name == "web_search" {
                assert!(
                    !definitions
                        .iter()
                        .any(|definition| definition.name == *name)
                );
            } else {
                assert!(
                    definitions
                        .iter()
                        .any(|definition| definition.name == *name)
                );
            }
        }
    }

    #[tokio::test]
    async fn web_search_definition_uses_real_query_schema_when_enabled() {
        let mut profile = SessionProfile::blank("session-1", 1);
        profile.host = "example.com".into();
        profile.username = "akko".into();
        let channel = AgentExecChannel::for_profile(
            profile,
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-known-hosts-web-tools")),
        )
        .with_web_search_config(
            WebSearchConfig {
                enabled: true,
                kind: WebSearchProviderKind::Tavily,
                has_api_key: true,
                max_results: 3,
                ..WebSearchConfig::default()
            },
            Some("tvly-test".into()),
        );

        let definitions = AgentToolSet::for_channel(channel, AgentMode::Execute)
            .definitions()
            .await;
        let web_search = definitions
            .iter()
            .find(|definition| definition.name == "web_search")
            .expect("web_search should be exposed when configured");

        assert!(web_search.parameters["properties"].get("query").is_some());
        assert!(
            web_search.parameters["properties"]
                .get("arguments")
                .is_none()
        );
        assert!(
            web_search.parameters["properties"]
                .get("approved")
                .is_none()
        );
    }

    #[test]
    fn execute_mode_does_not_preapprove_conditional_web_fetch_access() {
        assert!(auto_approve_rig_tool("web_search"));
        assert!(!auto_approve_rig_tool("web_fetch"));
        assert!(!initial_tool_approval(AgentMode::Execute, "web_fetch"));
        assert!(initial_tool_approval(AgentMode::FullAuto, "web_fetch"));
        assert!(initial_tool_approval(AgentMode::FullAuto, "run_shell"));
    }

    #[test]
    fn ask_and_non_blocking_require_confirmation_before_tool_execution() {
        for name in [
            "workspace_info",
            "read",
            "list",
            "glob",
            "grep",
            "web_search",
            "web_fetch",
        ] {
            assert!(mode_requires_confirmation(AgentMode::Ask, name));
            assert!(mode_requires_confirmation(AgentMode::NonBlocking, name));
        }

        assert!(!mode_requires_confirmation(AgentMode::Ask, "ask_user"));
        assert!(!mode_requires_confirmation(
            AgentMode::NonBlocking,
            "ask_user"
        ));
        assert!(!mode_requires_confirmation(AgentMode::Execute, "read"));
        assert!(!mode_requires_confirmation(AgentMode::FullAuto, "read"));
    }

    #[tokio::test]
    async fn ask_read_returns_approval_without_contacting_backend() {
        let mut profile = SessionProfile::blank("ask-no-backend", 1);
        profile.host = "unreachable.invalid".into();
        profile.username = "akko".into();
        let tool = JsonAgentTool {
            name: "read".into(),
            channel: AgentExecChannel::for_profile(
                profile,
                Vec::new(),
                SecretStore::new_locked_vault(),
                KnownHostsStore::with_path(
                    std::env::temp_dir().join("agent-known-hosts-ask-no-backend"),
                ),
            ),
            mode: AgentMode::Ask,
            cancellation: AgentToolCancellation::new(),
        };

        let result = Tool::call(&tool, json!({ "path": "README.md" }))
            .await
            .unwrap();
        let response: AgentToolCallResponse = serde_json::from_str(&result).unwrap();
        assert!(matches!(response.output, ToolOutput::Approval { .. }));
    }

    #[tokio::test]
    async fn cancelled_tool_returns_a_structured_marker_without_contacting_backend() {
        let mut profile = SessionProfile::blank("cancelled-no-backend", 1);
        profile.host = "unreachable.invalid".into();
        profile.username = "akko".into();
        let cancellation = AgentToolCancellation::new();
        cancellation.cancel();
        let tool = JsonAgentTool {
            name: "read".into(),
            channel: AgentExecChannel::for_profile(
                profile,
                Vec::new(),
                SecretStore::new_locked_vault(),
                KnownHostsStore::with_path(
                    std::env::temp_dir().join("agent-known-hosts-cancelled-no-backend"),
                ),
            ),
            mode: AgentMode::FullAuto,
            cancellation,
        };

        let result = Tool::call(&tool, json!({ "path": "README.md" }))
            .await
            .expect("cancellation should be returned as structured tool output");
        let response: AgentToolCallResponse = serde_json::from_str(&result).unwrap();

        assert!(matches!(response.output, ToolOutput::Cancelled));
    }

    #[test]
    fn normalize_tool_arguments_accepts_direct_and_legacy_wrapped_shapes() {
        assert_eq!(
            normalize_tool_arguments(json!({ "query": "rust news" })),
            json!({ "query": "rust news" })
        );
        assert_eq!(
            normalize_tool_arguments(json!({ "arguments": { "query": "rust news" } })),
            json!({ "query": "rust news" })
        );
    }

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
            run_cancellable_tool_worker(worker_cancellation, move || async move {
                let _drop_flag = DropFlag(worker_dropped);
                let _ = started_sender.send(());
                std::future::pending::<Result<(), AgentError>>().await
            })
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
        let result = execute_cancellable_tool_worker(&cancellation, worker_id, stop, move || {
            build_called_by_worker.store(true, std::sync::atomic::Ordering::Release);
            std::future::ready(Ok::<_, AgentError>(()))
        });
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
            execute_cancellable_tool_worker(&worker_cancellation, worker_id, stop, move || {
                build_called_by_worker.store(true, std::sync::atomic::Ordering::Release);
                std::future::ready(Ok::<_, AgentError>(()))
            })
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
            started: Option<tokio::sync::oneshot::Sender<()>>,
            ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
        }

        impl std::future::Future for CompleteWhenReleased {
            type Output = Result<&'static str, AgentError>;

            fn poll(
                mut self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Self::Output> {
                if let Some(started) = self.started.take() {
                    let _ = started.send(());
                }
                if self.ready.load(std::sync::atomic::Ordering::Acquire) {
                    std::task::Poll::Ready(Ok("first completed"))
                } else {
                    std::task::Poll::Pending
                }
            }
        }

        let cancellation = AgentToolCancellation::new();
        let ready = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (started_sender, started_receiver) = tokio::sync::oneshot::channel();
        let first_cancellation = cancellation.clone();
        let first_ready = ready.clone();
        let first = tokio::spawn(async move {
            run_cancellable_tool_worker(first_cancellation, move || CompleteWhenReleased {
                started: Some(started_sender),
                ready: first_ready,
            })
            .await
        });
        started_receiver
            .await
            .expect("first tool worker should begin polling");

        // Make the in-flight result and Stop observable on the same worker poll. The completed
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
        let second = run_cancellable_tool_worker(cancellation, move || {
            second_started_in_worker.store(true, std::sync::atomic::Ordering::Release);
            std::future::ready(Ok::<_, AgentError>("second completed"))
        })
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
                run_cancellable_tool_worker(cancellation, move || async move {
                    let _drop_flag = DropFlag(Some(dropped_sender));
                    let _ = started_sender.send(());
                    std::future::pending::<Result<(), AgentError>>().await
                })
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
