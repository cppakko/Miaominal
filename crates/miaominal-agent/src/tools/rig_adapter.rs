use super::{TOOL_NAMES, tool_description};
use crate::channel::{AgentExecChannel, AgentToolCallRequest};
use crate::chat::AgentMode;
use crate::error::AgentError;
use rig_core::completion::ToolDefinition;
use rig_core::tool::{Tool, ToolDyn, ToolSet};
use serde_json::Map;
use serde_json::{Value, json};
use tokio::sync::oneshot;

#[derive(Clone)]
pub struct AgentToolSet {
    channel: AgentExecChannel,
    mode: AgentMode,
}

impl AgentToolSet {
    pub fn for_channel(channel: AgentExecChannel, mode: AgentMode) -> Self {
        Self { channel, mode }
    }

    pub fn into_rig_tool_set(self) -> ToolSet {
        let mut toolset = ToolSet::default();
        for name in self.enabled_tool_names() {
            toolset.add_tool(JsonAgentTool {
                name: name.to_string(),
                channel: self.channel.clone(),
                mode: self.mode,
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
                }) as Box<dyn ToolDyn>
            })
            .collect()
    }

    pub async fn definitions(&self) -> Vec<ToolDefinition> {
        self.enabled_tool_names()
            .into_iter()
            .map(|name| tool_definition(name))
            .collect()
    }

    fn enabled_tool_names(&self) -> Vec<&'static str> {
        let all = TOOL_NAMES
            .iter()
            .copied()
            .filter(|name| *name != "web_search" || self.channel.web_search_enabled());
        
        match self.mode {
            AgentMode::Ask => all.filter(|name| {
                matches!(*name, "workspace_info" | "read" | "list" | "glob" | "grep" | "web_search" | "web_fetch")
            }).collect(),
            _ => all.collect(),
        }
    }
}

#[derive(Clone)]
struct JsonAgentTool {
    name: String,
    channel: AgentExecChannel,
    mode: AgentMode,
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

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let approved = match self.mode {
            AgentMode::NonBlocking | AgentMode::FullAuto => true,
            AgentMode::Ask => false,
            AgentMode::Execute => auto_approve_rig_tool(&self.name),
        };
        let skip_policy = matches!(self.mode, AgentMode::FullAuto);
        let response = call_tool_on_worker(
            self.channel.clone(),
            AgentToolCallRequest {
                tool_name: self.name.clone(),
                arguments: normalize_tool_arguments(args),
                approved,
                route: None,
                skip_policy,
            },
        )
        .await?;
        serde_json::to_string(&response)
            .map_err(|error| AgentError::InvalidArguments(error.to_string()))
    }
}

async fn call_tool_on_worker(
    channel: AgentExecChannel,
    request: AgentToolCallRequest,
) -> Result<crate::AgentToolCallResponse, AgentError> {
    let (sender, receiver) = oneshot::channel();
    std::thread::Builder::new()
        .name(format!("agent-tool-{}", request.tool_name))
        .spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| AgentError::Backend(error.into()))
                .and_then(|runtime| runtime.block_on(channel.call_tool(request)));
            let _ = sender.send(result);
        })
        .map_err(|error| AgentError::Backend(error.into()))?;

    receiver
        .await
        .map_err(|_| AgentError::Backend(anyhow::anyhow!("agent tool worker stopped")))?
}

fn tool_definition(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: tool_description(name).to_string(),
        parameters: tool_parameters(name),
    }
}

fn auto_approve_rig_tool(name: &str) -> bool {
    matches!(name, "web_search" | "web_fetch")
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

fn string_array_schema(description: &str) -> Value {
    json!({
        "type": "array",
        "items": { "type": "string" },
        "description": description,
    })
}

fn tool_parameters(name: &str) -> Value {
    match name {
        "workspace_info" | "list_jobs" | "approval" => object_schema(Vec::new(), &[]),
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
                ("shell", string_schema("Shell label; use posix-sh or fish.")),
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
                    string_schema("Question or request for the user."),
                ),
                (
                    "operation_hash",
                    string_schema("Optional operation identifier."),
                ),
            ],
            &[],
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
        let definitions = AgentToolSet::for_channel(channel, AgentMode::Execute).definitions().await;

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

        let definitions = AgentToolSet::for_channel(channel, AgentMode::Execute).definitions().await;
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
}
