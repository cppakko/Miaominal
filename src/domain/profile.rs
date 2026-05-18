use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMethod {
    #[default]
    Password,
    KeyFile,
    ManagedKey,
    Agent,
    KeyboardInteractive,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PortForwardKind {
    #[default]
    Local,
    Remote,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortForwardRule {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub kind: PortForwardKind,
    #[serde(default = "default_port_forward_host")]
    pub listen_host: String,
    pub listen_port: u16,
    #[serde(default = "default_port_forward_host")]
    pub target_host: String,
    pub target_port: u16,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SessionEnvironmentVariable {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ShellType {
    #[default]
    Posix,
    Fish,
    PowerShell,
    Cmd,
}

pub const DEFAULT_SESSION_CHARSET: &str = "UTF-8";

fn default_port_forward_host() -> String {
    "127.0.0.1".into()
}

fn default_session_charset() -> String {
    DEFAULT_SESSION_CHARSET.into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionProfile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(default, skip_serializing, skip_deserializing)]
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<AuthMethod>,
    #[serde(default)]
    pub private_key_path: String,
    #[serde(default)]
    pub managed_key_id: String,
    #[serde(default)]
    pub agent_identity: String,
    #[serde(default)]
    pub agent_identity_label: String,
    #[serde(default)]
    pub certificate_path: String,
    #[serde(default, skip_serializing, skip_deserializing)]
    pub passphrase: String,
    #[serde(default)]
    pub agent_forwarding: bool,
    #[serde(default)]
    pub startup_command: String,
    #[serde(default = "default_session_charset")]
    pub charset: String,
    #[serde(default)]
    pub environment_variables: Vec<SessionEnvironmentVariable>,
    #[serde(default)]
    pub shell_type: ShellType,
    #[serde(default)]
    pub proxy_jump_profile_ids: Vec<String>,
    #[serde(default)]
    pub has_stored_password: bool,
    #[serde(default)]
    pub has_stored_passphrase: bool,
    #[serde(default)]
    pub port_forwarding_rules: Vec<PortForwardRule>,
    #[serde(default)]
    pub is_favorite: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_connected_at: Option<u64>,
}

impl SessionProfile {
    pub fn blank(id: impl Into<String>, ordinal: usize) -> Self {
        Self {
            id: id.into(),
            name: format!("Session {}", ordinal),
            group: String::new(),
            tags: Vec::new(),
            host: String::new(),
            port: 22,
            username: String::new(),
            password: String::new(),
            auth_method: Some(AuthMethod::Password),
            private_key_path: String::new(),
            managed_key_id: String::new(),
            agent_identity: String::new(),
            agent_identity_label: String::new(),
            certificate_path: String::new(),
            passphrase: String::new(),
            agent_forwarding: false,
            startup_command: String::new(),
            charset: default_session_charset(),
            environment_variables: Vec::new(),
            shell_type: ShellType::Posix,
            proxy_jump_profile_ids: Vec::new(),
            has_stored_password: false,
            has_stored_passphrase: false,
            port_forwarding_rules: Vec::new(),
            is_favorite: false,
            last_connected_at: None,
        }
    }

    pub fn summary(&self) -> String {
        format!("{}@{}:{}", self.username, self.host, self.port)
    }

    pub fn connection_label(&self) -> String {
        if self.name.trim().is_empty() {
            self.summary()
        } else {
            self.name.clone()
        }
    }

    pub fn effective_auth_method(&self) -> AuthMethod {
        self.auth_method.unwrap_or_else(|| {
            if !self.managed_key_id.trim().is_empty() {
                AuthMethod::ManagedKey
            } else if !self.agent_identity.trim().is_empty() {
                AuthMethod::Agent
            } else if !self.private_key_path.trim().is_empty() {
                AuthMethod::KeyFile
            } else {
                AuthMethod::Password
            }
        })
    }

    pub fn ensure_auth_method(&mut self) {
        self.auth_method = Some(self.effective_auth_method());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportSourceKind {
    OpenSshConfig,
    PuttyRegistry,
    SecureCrtXml,
    FinalShellJson,
}

impl ImportSourceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenSshConfig => "OpenSSH",
            Self::PuttyRegistry => "PuTTY",
            Self::SecureCrtXml => "SecureCRT",
            Self::FinalShellJson => "FinalShell",
        }
    }

    pub fn expected_extensions(self) -> &'static [&'static str] {
        match self {
            Self::OpenSshConfig => &["config"],
            Self::PuttyRegistry => &["reg"],
            Self::SecureCrtXml => &["xml"],
            Self::FinalShellJson => &["json"],
        }
    }

    pub fn accepts_path(self, path: &Path) -> bool {
        match self {
            Self::OpenSshConfig => true,
            _ => path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    self.expected_extensions()
                        .iter()
                        .any(|candidate| candidate.eq_ignore_ascii_case(extension))
                }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportIssueKind {
    UnsupportedProtocol,
    MissingRequiredField,
    UnsupportedCredential,
    UnsupportedFeature,
    InvalidEntry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportIssue {
    pub kind: ImportIssueKind,
    pub entry_name: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedSessionDraft {
    pub source: ImportSourceKind,
    pub name: String,
    pub group: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub auth_method: AuthMethod,
    pub private_key_path: String,
    pub certificate_path: String,
    pub passphrase: Option<String>,
    pub agent_forwarding: bool,
    pub startup_command: String,
    pub charset: String,
}

impl ImportedSessionDraft {
    pub fn into_session_profile(self, id: String) -> SessionProfile {
        let mut profile = SessionProfile::blank(id, 1);
        let password = self.password.unwrap_or_default();
        let passphrase = self.passphrase.unwrap_or_default();

        profile.name = self.name;
        profile.group = if self.group.trim().is_empty() {
            self.source.label().to_string()
        } else {
            self.group
        };
        profile.host = self.host;
        profile.port = self.port;
        profile.username = self.username;
        profile.password = password.clone();
        profile.auth_method = Some(self.auth_method);
        profile.private_key_path = self.private_key_path;
        profile.certificate_path = self.certificate_path;
        profile.passphrase = passphrase.clone();
        profile.agent_forwarding = self.agent_forwarding;
        profile.startup_command = self.startup_command;
        profile.charset = if self.charset.trim().is_empty() {
            DEFAULT_SESSION_CHARSET.to_string()
        } else {
            self.charset
        };
        profile.shell_type = ShellType::Posix;
        profile.has_stored_password = !password.is_empty();
        profile.has_stored_passphrase = !passphrase.is_empty();
        profile.ensure_auth_method();
        profile
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportedBatch {
    pub sessions: Vec<ImportedSessionDraft>,
    pub issues: Vec<ImportIssue>,
}

impl ImportedBatch {
    pub(crate) fn push_issue(
        &mut self,
        kind: ImportIssueKind,
        entry_name: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.issues.push(ImportIssue {
            kind,
            entry_name: entry_name.into(),
            detail: detail.into(),
        });
    }

    #[cfg(test)]
    pub fn issue_count(&self, kind: ImportIssueKind) -> usize {
        self.issues
            .iter()
            .filter(|issue| issue.kind == kind)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_label_prefers_profile_name() {
        let mut profile = SessionProfile::blank("session-1", 1);
        profile.name = "Production".into();
        profile.host = "example.com".into();
        profile.username = "akko".into();

        assert_eq!(profile.connection_label(), "Production");
    }

    #[test]
    fn connection_label_falls_back_to_summary_when_name_is_blank() {
        let mut profile = SessionProfile::blank("session-1", 1);
        profile.name = "   ".into();
        profile.host = "example.com".into();
        profile.username = "akko".into();

        assert_eq!(profile.connection_label(), "akko@example.com:22");
    }

    #[test]
    fn explicit_password_auth_survives_with_key_path_present() {
        let mut profile = SessionProfile::blank("session-2", 1);
        profile.name = "Pinned".into();
        profile.host = "example.com".into();
        profile.username = "akko".into();
        profile.auth_method = Some(AuthMethod::Password);
        profile.private_key_path = "C:/Users/akko/.ssh/id_ed25519".into();
        profile.has_stored_password = true;

        assert_eq!(profile.effective_auth_method(), AuthMethod::Password);
    }
}
