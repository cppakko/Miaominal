use crate::error::{AgentError, AgentResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentPolicyDecision {
    Allow,
    NeedsApproval { reason: String },
    Deny { reason: String },
}

#[derive(Debug, Clone, Default)]
pub struct AgentPolicy;

impl AgentPolicy {
    pub fn decide(&self, tool_name: &str, approved: bool) -> AgentPolicyDecision {
        match tool_name {
            "workspace_info" | "read" | "list" | "glob" | "grep" => AgentPolicyDecision::Allow,
            "apply_patch" | "run_shell" | "start_job" | "poll_job" | "stop_job" | "web_search"
            | "web_fetch" | "ask_user" | "approval" => {
                if approved {
                    AgentPolicyDecision::Allow
                } else {
                    AgentPolicyDecision::NeedsApproval {
                        reason: format!("tool `{tool_name}` can affect state or external IO"),
                    }
                }
            }
            _ => AgentPolicyDecision::Deny {
                reason: "tool is not registered in the agent policy".into(),
            },
        }
    }

    pub fn enforce(&self, tool_name: &str, approved: bool) -> AgentResult<()> {
        match self.decide(tool_name, approved) {
            AgentPolicyDecision::Allow => Ok(()),
            AgentPolicyDecision::NeedsApproval { .. } => Err(AgentError::ApprovalRequired {
                tool_name: tool_name.to_string(),
            }),
            AgentPolicyDecision::Deny { reason } => Err(AgentError::Denied {
                tool_name: tool_name.to_string(),
                reason,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_tools_are_allowed() {
        let policy = AgentPolicy;

        for tool in ["workspace_info", "read", "list", "glob", "grep"] {
            assert_eq!(policy.decide(tool, false), AgentPolicyDecision::Allow);
        }
    }

    #[test]
    fn write_command_job_web_and_approval_tools_need_approval() {
        let policy = AgentPolicy;

        for tool in [
            "apply_patch",
            "run_shell",
            "start_job",
            "poll_job",
            "stop_job",
            "web_search",
            "web_fetch",
            "ask_user",
            "approval",
        ] {
            assert!(matches!(
                policy.decide(tool, false),
                AgentPolicyDecision::NeedsApproval { .. }
            ));
            assert_eq!(policy.decide(tool, true), AgentPolicyDecision::Allow);
        }
    }

    #[test]
    fn unknown_tools_are_denied() {
        let policy = AgentPolicy;

        assert!(matches!(
            policy.decide("rm_everything", false),
            AgentPolicyDecision::Deny { .. }
        ));
    }
}
