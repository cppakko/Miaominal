use crate::channel::{ToolOutput, UserQuestionChoice};
use crate::error::AgentResult;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ApprovalArgs {
    pub message: Option<String>,
    pub operation_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AskUserArgs {
    pub message: Option<String>,
    #[serde(default)]
    pub choices: Vec<AskUserChoiceArg>,
    pub allow_custom: Option<bool>,
    pub operation_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum AskUserChoiceArg {
    Text(String),
    Object(UserQuestionChoice),
}

pub fn approval(args: ApprovalArgs) -> AgentResult<ToolOutput> {
    Ok(ToolOutput::Approval {
        message: args
            .message
            .unwrap_or_else(|| "approval requested".to_string()),
        operation_hash: args.operation_hash,
    })
}

pub fn ask_user(args: AskUserArgs) -> AgentResult<ToolOutput> {
    let choices = args
        .choices
        .into_iter()
        .filter_map(|choice| match choice {
            AskUserChoiceArg::Text(label) => {
                let label = label.trim();
                (!label.is_empty()).then(|| UserQuestionChoice {
                    label: label.to_string(),
                    description: None,
                })
            }
            AskUserChoiceArg::Object(choice) => {
                let label = choice.label.trim();
                (!label.is_empty()).then(|| UserQuestionChoice {
                    label: label.to_string(),
                    description: choice
                        .description
                        .as_deref()
                        .map(str::trim)
                        .filter(|description| !description.is_empty())
                        .map(ToOwned::to_owned),
                })
            }
        })
        .take(3)
        .collect();

    Ok(ToolOutput::UserQuestion {
        message: args
            .message
            .map(|message| message.trim().to_string())
            .filter(|message| !message.is_empty())
            .unwrap_or_else(|| "Please choose an option or enter a custom response.".to_string()),
        choices,
        allow_custom: args.allow_custom.unwrap_or(true),
        operation_hash: args.operation_hash,
    })
}
