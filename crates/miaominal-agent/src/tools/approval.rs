use crate::channel::ToolOutput;
use crate::error::AgentResult;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ApprovalArgs {
    pub message: Option<String>,
    pub operation_hash: Option<String>,
}

pub fn approval(args: ApprovalArgs) -> AgentResult<ToolOutput> {
    Ok(ToolOutput::Approval {
        message: args
            .message
            .unwrap_or_else(|| "approval requested".to_string()),
        operation_hash: args.operation_hash,
    })
}
