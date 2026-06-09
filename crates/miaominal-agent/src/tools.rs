use crate::channel::{AgentExecChannel, AgentToolCallRequest};
use crate::error::AgentError;
use rig_core::completion::ToolDefinition;
use rig_core::tool::{Tool, ToolSet};
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::oneshot;

pub const TOOL_NAMES: &[&str] = &[
    "workspace_info",
    "read",
    "list",
    "glob",
    "grep",
    "apply_patch",
    "run_shell",
    "start_job",
    "poll_job",
    "stop_job",
    "web_search",
    "web_fetch",
    "ask_user",
    "approval",
];

#[derive(Clone)]
pub struct AgentToolSet {
    channel: AgentExecChannel,
}

impl AgentToolSet {
    pub fn for_channel(channel: AgentExecChannel) -> Self {
        Self { channel }
    }

    pub fn into_rig_tool_set(self) -> ToolSet {
        let mut toolset = ToolSet::default();
        for name in TOOL_NAMES {
            toolset.add_tool(JsonAgentTool {
                name: (*name).to_string(),
                channel: self.channel.clone(),
            });
        }
        toolset
    }

    pub async fn definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions = Vec::new();
        for name in TOOL_NAMES {
            definitions.push(tool_definition(name));
        }
        definitions
    }
}

#[derive(Clone)]
struct JsonAgentTool {
    name: String,
    channel: AgentExecChannel,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
struct JsonAgentToolArgs {
    #[serde(default)]
    arguments: Value,
    #[serde(default)]
    approved: bool,
}

impl Tool for JsonAgentTool {
    const NAME: &'static str = "miaominal_agent_tool";

    type Error = AgentError;
    type Args = JsonAgentToolArgs;
    type Output = String;

    fn name(&self) -> String {
        self.name.clone()
    }

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        tool_definition(&self.name)
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let response = call_tool_on_worker(
            self.channel.clone(),
            AgentToolCallRequest {
                tool_name: self.name.clone(),
                arguments: args.arguments,
                approved: args.approved,
                route: None,
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
        parameters: json!(schema_for!(JsonAgentToolArgs)),
    }
}

fn tool_description(name: &str) -> &'static str {
    match name {
        "workspace_info" => "Return profile workspace metadata for the current SSH exec channel.",
        "read" => "Read a file from the remote profile workspace.",
        "list" => "List entries in a remote profile workspace directory.",
        "glob" => "Find paths in the remote profile workspace by pattern.",
        "grep" => "Search remote workspace files for text.",
        "apply_patch" => "Apply a unified patch in the remote profile workspace.",
        "run_shell" => "Run a synchronous shell command in the remote profile workspace.",
        "start_job" => "Start a background shell job in the remote profile workspace.",
        "poll_job" => "Poll a background shell job.",
        "stop_job" => "Stop a background shell job.",
        "web_search" => "Search the web through the configured provider.",
        "web_fetch" => "Fetch the text content of a URL.",
        "ask_user" => "Ask the user for information or approval.",
        "approval" => "Record a user approval response.",
        _ => "Miaominal agent tool.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::profile::SessionProfile;
    use miaominal_secrets::SecretStore;
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
        let definitions = AgentToolSet::for_channel(channel).definitions().await;

        assert_eq!(definitions.len(), TOOL_NAMES.len());
        for name in TOOL_NAMES {
            assert!(
                definitions
                    .iter()
                    .any(|definition| definition.name == *name)
            );
        }
    }
}
