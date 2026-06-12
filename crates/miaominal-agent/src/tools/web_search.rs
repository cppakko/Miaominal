use crate::channel::{AgentExecChannel, ToolOutput};
use crate::error::AgentResult;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
pub struct WebSearchArgs {
    pub query: String,
}

pub async fn web_search(
    channel: &AgentExecChannel,
    args: WebSearchArgs,
) -> AgentResult<ToolOutput> {
    let results = channel.web_search().search(&args.query).await?;
    Ok(ToolOutput::WebSearch {
        results: json!(results),
    })
}
