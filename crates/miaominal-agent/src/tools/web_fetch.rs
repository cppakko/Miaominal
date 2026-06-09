use crate::channel::{AgentExecChannel, ToolOutput};
use crate::error::AgentResult;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct WebFetchArgs {
    pub url: String,
    pub max_bytes: Option<usize>,
}

pub async fn web_fetch(channel: &AgentExecChannel, args: WebFetchArgs) -> AgentResult<ToolOutput> {
    let text = reqwest::get(&args.url)
        .await
        .map_err(anyhow::Error::from)?
        .text()
        .await
        .map_err(anyhow::Error::from)?;
    let max = args
        .max_bytes
        .unwrap_or(channel.web_fetch_config().max_bytes);
    let truncated = text.len() > max;
    let content = if truncated {
        text.chars().take(max).collect()
    } else {
        text
    };
    Ok(ToolOutput::WebFetch {
        url: args.url,
        content,
        truncated,
    })
}
