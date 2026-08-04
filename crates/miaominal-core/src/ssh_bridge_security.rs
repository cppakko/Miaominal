use serde::{Deserialize, Serialize};

pub const DEFAULT_BRIDGE_APPROVAL_TIMEOUT_SECS: u32 = 30;
pub const MIN_BRIDGE_APPROVAL_TIMEOUT_SECS: u32 = 5;
pub const MAX_BRIDGE_APPROVAL_TIMEOUT_SECS: u32 = 120;
pub const BRIDGE_SYSTEM_AUTH_TIMEOUT_SECS: u32 = 60;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BridgeSecurityLevel {
    #[default]
    Standard,
    RequireApproval {
        timeout_secs: u32,
    },
    RequireSystemAuth,
}

impl BridgeSecurityLevel {
    pub fn validate(self) -> Result<Self, &'static str> {
        match self {
            Self::RequireApproval { timeout_secs }
                if !(MIN_BRIDGE_APPROVAL_TIMEOUT_SECS..=MAX_BRIDGE_APPROVAL_TIMEOUT_SECS)
                    .contains(&timeout_secs) =>
            {
                Err("SSH Bridge approval timeout must be between 5 and 120 seconds")
            }
            level => Ok(level),
        }
    }

    pub fn approval_timeout_secs(self) -> Option<u32> {
        match self {
            Self::RequireApproval { timeout_secs } => Some(timeout_secs),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeSecurityPolicy {
    pub level: BridgeSecurityLevel,
    pub updated_at: i64,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeProcessCaptureStatus {
    Captured,
    Exited,
    AccessDenied,
    Unsupported,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeProcessIdentity {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub started_at: Option<u64>,
    pub executable_path: Option<String>,
    pub capture_status: BridgeProcessCaptureStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgePeerIdentity {
    pub pid: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub user_sid: Option<String>,
    #[serde(default)]
    pub process_chain: Vec<BridgeProcessIdentity>,
    pub capture_status: Option<BridgeProcessCaptureStatus>,
}

impl BridgePeerIdentity {
    pub fn source_path(&self) -> Option<&str> {
        self.process_chain
            .iter()
            .rev()
            .find_map(|process| process.executable_path.as_deref())
    }

    pub fn application_source_path(&self) -> Option<&str> {
        let mut processes = self
            .process_chain
            .iter()
            .filter_map(|process| process.executable_path.as_deref());
        let first = processes.next()?;
        let mut candidates = Vec::with_capacity(self.process_chain.len());
        if !is_bridge_helper_process(first) {
            candidates.push(first);
        }
        candidates.extend(processes);

        let mut shell_fallback = None;
        let mut transport_fallback = None;
        for path in candidates {
            let name = executable_name(path);
            if is_desktop_launcher(name) {
                continue;
            }
            if is_ssh_transport(name) {
                transport_fallback.get_or_insert(path);
                continue;
            }
            if is_command_shell(name) {
                shell_fallback.get_or_insert(path);
                continue;
            }
            return Some(path);
        }

        shell_fallback
            .or(transport_fallback)
            .or_else(|| self.source_path())
    }
}

fn executable_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn is_bridge_helper_process(path: &str) -> bool {
    matches!(
        executable_name(path).to_ascii_lowercase().as_str(),
        "miaominal" | "miaominal.exe" | "miaominal-ssh-bridge-helper"
    )
}

fn is_ssh_transport(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "ssh" | "ssh.exe" | "plink" | "plink.exe" | "dbclient" | "dbclient.exe"
    )
}

fn is_command_shell(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "bash"
            | "cmd"
            | "cmd.exe"
            | "fish"
            | "nu"
            | "nushell"
            | "powershell"
            | "powershell.exe"
            | "pwsh"
            | "pwsh.exe"
            | "sh"
            | "wsl"
            | "wsl.exe"
            | "zsh"
    )
}

fn is_desktop_launcher(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "explorer" | "explorer.exe" | "init" | "launchd" | "systemd" | "userinit.exe"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeAuthorizationDecision {
    Approve,
    Reject,
    SystemAuthVerified,
    SystemAuthCancelled,
    SystemAuthUnavailable,
    SystemAuthFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeAuthorizationOutcome {
    NotRequired,
    Pending,
    Approved,
    Rejected,
    TimedOut,
    Unsupported,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeConnectionOutcome {
    Pending,
    Rejected,
    UpstreamFailed,
    Active,
    Disconnected,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeDecisionSource {
    App,
    SystemAuth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgePendingPhase {
    AwaitingApproval,
    AwaitingSystemAuth,
    AwaitingVaultUnlock,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgePendingAuthorization {
    pub request_id: String,
    pub profile_id: String,
    pub profile_name: String,
    pub level: BridgeSecurityLevel,
    pub phase: BridgePendingPhase,
    pub policy_generation: u64,
    pub peer: BridgePeerIdentity,
    pub created_at: i64,
    pub phase_started_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeAuditRecord {
    pub request_id: String,
    pub owner_instance_id: String,
    pub requested_at: i64,
    pub decision_at: Option<i64>,
    pub connected_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub profile_id: Option<String>,
    pub profile_name: Option<String>,
    pub security_level: BridgeSecurityLevel,
    pub peer: BridgePeerIdentity,
    pub authorization_outcome: BridgeAuthorizationOutcome,
    pub connection_outcome: BridgeConnectionOutcome,
    pub decision_source: Option<BridgeDecisionSource>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BridgeSecuritySnapshot {
    pub policy: BridgeSecurityPolicy,
    pub pending: Vec<BridgePendingAuthorization>,
    pub audit_health_error: Option<String>,
    pub policy_store_error: Option<String>,
    pub system_auth_available: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_timeout_is_validated() {
        assert!(
            BridgeSecurityLevel::RequireApproval { timeout_secs: 1 }
                .validate()
                .is_err()
        );
        assert!(
            BridgeSecurityLevel::RequireApproval { timeout_secs: 999 }
                .validate()
                .is_err()
        );
        for timeout_secs in [
            MIN_BRIDGE_APPROVAL_TIMEOUT_SECS,
            DEFAULT_BRIDGE_APPROVAL_TIMEOUT_SECS,
            MAX_BRIDGE_APPROVAL_TIMEOUT_SECS,
        ] {
            assert_eq!(
                BridgeSecurityLevel::RequireApproval { timeout_secs }.validate(),
                Ok(BridgeSecurityLevel::RequireApproval { timeout_secs })
            );
        }
    }

    #[test]
    fn standard_is_the_default_level() {
        assert_eq!(
            BridgeSecurityLevel::default(),
            BridgeSecurityLevel::Standard
        );
    }

    #[test]
    fn security_levels_have_stable_tagged_serialization() {
        assert_eq!(
            serde_json::to_string(&BridgeSecurityLevel::Standard).unwrap(),
            r#"{"kind":"standard"}"#
        );
        assert_eq!(
            serde_json::to_string(&BridgeSecurityLevel::RequireApproval { timeout_secs: 30 })
                .unwrap(),
            r#"{"kind":"require_approval","timeout_secs":30}"#
        );
        assert_eq!(
            serde_json::from_str::<BridgeSecurityLevel>(r#"{"kind":"require_system_auth"}"#)
                .unwrap(),
            BridgeSecurityLevel::RequireSystemAuth
        );
    }

    #[test]
    fn source_path_prefers_outermost_resolved_ancestor() {
        let peer = BridgePeerIdentity {
            process_chain: vec![
                BridgeProcessIdentity {
                    pid: 10,
                    parent_pid: Some(20),
                    started_at: None,
                    executable_path: Some("helper".into()),
                    capture_status: BridgeProcessCaptureStatus::Captured,
                },
                BridgeProcessIdentity {
                    pid: 20,
                    parent_pid: Some(30),
                    started_at: None,
                    executable_path: None,
                    capture_status: BridgeProcessCaptureStatus::AccessDenied,
                },
                BridgeProcessIdentity {
                    pid: 30,
                    parent_pid: None,
                    started_at: None,
                    executable_path: Some("outer-client".into()),
                    capture_status: BridgeProcessCaptureStatus::Captured,
                },
            ],
            ..BridgePeerIdentity::default()
        };
        assert_eq!(peer.source_path(), Some("outer-client"));
    }

    #[test]
    fn application_source_prefers_vscode_over_ssh_and_explorer() {
        let peer = peer_with_processes(&[
            r"C:\Program Files\Miaominal\miaominal.exe",
            r"C:\Windows\System32\OpenSSH\ssh.exe",
            r"C:\Program Files\Microsoft VS Code\Code.exe",
            r"C:\Windows\explorer.exe",
        ]);
        assert_eq!(
            peer.application_source_path(),
            Some(r"C:\Program Files\Microsoft VS Code\Code.exe")
        );
    }

    #[test]
    fn application_source_prefers_terminal_over_its_shell() {
        let peer = peer_with_processes(&[
            r"C:\Miaominal\miaominal.exe",
            r"C:\Windows\System32\OpenSSH\ssh.exe",
            r"C:\Program Files\PowerShell\7\pwsh.exe",
            r"C:\Program Files\WindowsApps\WindowsTerminal.exe",
            r"C:\Windows\explorer.exe",
        ]);
        assert_eq!(
            peer.application_source_path(),
            Some(r"C:\Program Files\WindowsApps\WindowsTerminal.exe")
        );
    }

    #[test]
    fn application_source_falls_back_to_a_direct_shell_or_ssh() {
        let shell =
            peer_with_processes(&["miaominal", "/usr/bin/ssh", "/usr/bin/zsh", "/sbin/launchd"]);
        assert_eq!(shell.application_source_path(), Some("/usr/bin/zsh"));

        let ssh = peer_with_processes(&[
            r"C:\Miaominal\miaominal.exe",
            r"C:\Windows\System32\OpenSSH\ssh.exe",
            r"C:\Windows\explorer.exe",
        ]);
        assert_eq!(
            ssh.application_source_path(),
            Some(r"C:\Windows\System32\OpenSSH\ssh.exe")
        );
    }

    fn peer_with_processes(paths: &[&str]) -> BridgePeerIdentity {
        BridgePeerIdentity {
            process_chain: paths
                .iter()
                .enumerate()
                .map(|(index, path)| BridgeProcessIdentity {
                    pid: index as u32 + 1,
                    parent_pid: None,
                    started_at: None,
                    executable_path: Some((*path).to_string()),
                    capture_status: BridgeProcessCaptureStatus::Captured,
                })
                .collect(),
            ..BridgePeerIdentity::default()
        }
    }
}
