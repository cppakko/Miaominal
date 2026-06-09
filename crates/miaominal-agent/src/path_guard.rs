use crate::error::{AgentError, AgentResult};

pub fn resolve_workspace_path(path: &str) -> AgentResult<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "." {
        return Ok(".".into());
    }
    if trimmed.starts_with('/') || trimmed.starts_with('~') {
        return Err(AgentError::InvalidPath(
            "absolute paths and home expansion are outside the agent workspace".into(),
        ));
    }
    if trimmed.contains('\0') || trimmed.contains('\n') || trimmed.contains('\r') {
        return Err(AgentError::InvalidPath(
            "path contains unsupported control characters".into(),
        ));
    }

    let mut parts = Vec::new();
    for part in trimmed.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                return Err(AgentError::InvalidPath(
                    "`..` segments cannot escape the agent workspace".into(),
                ));
            }
            part => parts.push(part),
        }
    }

    if parts.is_empty() {
        Ok(".".into())
    } else {
        Ok(parts.join("/"))
    }
}

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_are_normalized() {
        assert_eq!(
            resolve_workspace_path("./src//main.rs").unwrap(),
            "src/main.rs"
        );
        assert_eq!(resolve_workspace_path(".").unwrap(), ".");
    }

    #[test]
    fn parent_and_absolute_paths_are_rejected() {
        assert!(resolve_workspace_path("../secret").is_err());
        assert!(resolve_workspace_path("src/../../secret").is_err());
        assert!(resolve_workspace_path("/etc/passwd").is_err());
        assert!(resolve_workspace_path("~/secret").is_err());
    }
}
