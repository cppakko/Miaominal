use crate::channel::{AgentExecChannel, ToolOutput};
use crate::error::AgentResult;
use crate::path_guard::{resolve_workspace_path, shell_quote};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ApplyPatchArgs {
    pub patch: String,
    #[serde(default = "default_dot")]
    pub base_dir: String,
    pub validator: Option<PatchValidator>,
}

#[derive(Debug, Deserialize)]
pub struct PatchValidator {
    pub command: String,
}

pub async fn apply_patch(
    channel: &AgentExecChannel,
    args: ApplyPatchArgs,
) -> AgentResult<ToolOutput> {
    let base_dir = resolve_workspace_path(&args.base_dir)?;
    if crate::policy::is_sensitive_path(&base_dir) {
        channel
            .policy()
            .enforce_path(crate::policy::AgentPathAccess::Edit, &base_dir, false)?;
    }
    let command = format!(
        "cd \"$HOME\" && cd {base_dir} && patch -p0 <<'MIAOMINAL_AGENT_PATCH'\n{}\nMIAOMINAL_AGENT_PATCH",
        args.patch,
        base_dir = shell_quote(&base_dir),
    );
    let patch_output = channel.exec(command).await?;

    if let Some(validator) = args.validator {
        channel.policy().enforce_command(&validator.command, true)?;
        let validation = super::run_shell::run_shell(
            channel,
            super::run_shell::RunShellArgs {
                command: validator.command,
                cwd: Some(base_dir),
                timeout_seconds: Some(60),
                max_bytes: None,
                shell: None,
            },
        )
        .await?;
        Ok(ToolOutput::Patch {
            summary: patch_output,
            validation: Some(Box::new(validation)),
        })
    } else {
        Ok(ToolOutput::Patch {
            summary: patch_output,
            validation: None,
        })
    }
}

fn default_dot() -> String {
    ".".into()
}
