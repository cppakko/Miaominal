use anyhow::{Context, Result};
use miaominal_core::profile::SessionProfile;
use miaominal_core::snippet::SnippetRecord;
use miaominal_paths as paths;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionsDocument {
    #[serde(default)]
    pub sessions: Vec<SessionProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SnippetsDocument {
    #[serde(default)]
    pub snippets: Vec<SnippetRecord>,
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    sessions_file: PathBuf,
}

impl SessionStore {
    pub fn new() -> Result<Self> {
        Ok(Self {
            sessions_file: paths::config_file("sessions.toml")?,
        })
    }

    pub fn read_sessions_content(&self) -> Result<Option<String>> {
        if !self.sessions_file.exists() {
            return Ok(None);
        }

        fs::read_to_string(&self.sessions_file)
            .map(Some)
            .with_context(|| format!("failed to read {}", self.sessions_file.display()))
    }

    pub fn parse_sessions(&self, content: &str) -> Result<Vec<SessionProfile>> {
        let document: SessionsDocument = toml::from_str(content)
            .with_context(|| format!("failed to parse {}", self.sessions_file.display()))?;

        Ok(document.sessions)
    }

    pub fn save(&self, sessions: &[SessionProfile]) -> Result<()> {
        if let Some(parent) = self.sessions_file.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let mut normalized_sessions = sessions.to_vec();
        for profile in &mut normalized_sessions {
            profile.ensure_auth_method();
        }

        let content = toml::to_string_pretty(&SessionsDocument {
            sessions: normalized_sessions,
        })
        .context("failed to serialize session profiles")?;

        fs::write(&self.sessions_file, content)
            .with_context(|| format!("failed to write {}", self.sessions_file.display()))?;

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct SnippetStore {
    snippets_file: PathBuf,
}

impl SnippetStore {
    pub fn new() -> Result<Self> {
        Ok(Self {
            snippets_file: paths::config_file("snippets.toml")?,
        })
    }

    pub fn load(&self) -> Result<Vec<SnippetRecord>> {
        if !self.snippets_file.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&self.snippets_file)
            .with_context(|| format!("failed to read {}", self.snippets_file.display()))?;

        if content.trim().is_empty() {
            return Ok(Vec::new());
        }

        let document: SnippetsDocument = toml::from_str(&content)
            .with_context(|| format!("failed to parse {}", self.snippets_file.display()))?;

        Ok(document.snippets)
    }

    pub fn save(&self, snippets: &[SnippetRecord]) -> Result<()> {
        if let Some(parent) = self.snippets_file.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let content = toml::to_string_pretty(&SnippetsDocument {
            snippets: snippets.to_vec(),
        })
        .context("failed to serialize snippets")?;

        fs::write(&self.snippets_file, content)
            .with_context(|| format!("failed to write {}", self.snippets_file.display()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::profile::{AuthMethod, PortForwardKind, PortForwardRule};

    #[test]
    fn legacy_private_key_profiles_default_to_key_file_auth() {
        let document: SessionsDocument = toml::from_str(
            r#"
                [[sessions]]
                id = "session-1"
                name = "Legacy"
                host = "example.com"
                port = 22
                username = "akko"
                private_key_path = "C:/Users/akko/.ssh/id_ed25519"
            "#,
        )
        .expect("legacy profile should parse");

        let mut profile = document
            .sessions
            .into_iter()
            .next()
            .expect("profile exists");

        assert_eq!(profile.auth_method, None);
        assert_eq!(profile.effective_auth_method(), AuthMethod::KeyFile);

        profile.ensure_auth_method();
        assert_eq!(profile.auth_method, Some(AuthMethod::KeyFile));
    }

    #[test]
    fn legacy_profiles_default_to_empty_port_forwarding_rules() {
        let document: SessionsDocument = toml::from_str(
            r#"
                [[sessions]]
                id = "session-3"
                name = "Legacy"
                host = "example.com"
                port = 22
                username = "akko"
            "#,
        )
        .expect("legacy profile should parse");

        let profile = document
            .sessions
            .into_iter()
            .next()
            .expect("profile exists");

        assert!(profile.group.is_empty());
        assert!(profile.port_forwarding_rules.is_empty());
    }

    #[test]
    fn session_profiles_round_trip_port_forwarding_rules() {
        let mut profile = SessionProfile::blank("session-4", 1);
        profile.name = "Forwarded".into();
        profile.group = "Production".into();
        profile.host = "example.com".into();
        profile.username = "akko".into();
        profile.port_forwarding_rules = vec![PortForwardRule {
            id: "pf-1".into(),
            label: "Local API".into(),
            kind: PortForwardKind::Local,
            listen_host: "127.0.0.1".into(),
            listen_port: 15432,
            target_host: "10.0.0.5".into(),
            target_port: 5432,
            enabled: true,
        }];

        let content = toml::to_string_pretty(&SessionsDocument {
            sessions: vec![profile.clone()],
        })
        .expect("profile should serialize");
        let parsed: SessionsDocument =
            toml::from_str(&content).expect("profile should deserialize");

        assert_eq!(parsed.sessions.len(), 1);
        let parsed_profile = &parsed.sessions[0];
        assert_eq!(parsed_profile.id, profile.id);
        assert_eq!(parsed_profile.name, profile.name);
        assert_eq!(parsed_profile.group, profile.group);
        assert_eq!(parsed_profile.summary(), profile.summary());
        assert_eq!(
            parsed_profile.port_forwarding_rules,
            profile.port_forwarding_rules
        );
    }

    #[test]
    fn legacy_snippets_default_to_bash_language() {
        let document: SnippetsDocument = toml::from_str(
            r#"
                [[snippets]]
                id = "snippet-1"
                description = "Init"
                package = "ops"
                script = "echo hello"
            "#,
        )
        .expect("snippet should parse");

        let snippet = document
            .snippets
            .into_iter()
            .next()
            .expect("snippet exists");

        assert_eq!(snippet.language, "bash");
    }

    #[test]
    fn snippets_round_trip_package_and_script() {
        let snippet = SnippetRecord {
            id: "snippet-2".into(),
            description: "Rotate logs".into(),
            package: "maintenance".into(),
            language: "bash".into(),
            script: "logrotate -f /etc/logrotate.conf\n".into(),
        };

        let content = toml::to_string_pretty(&SnippetsDocument {
            snippets: vec![snippet.clone()],
        })
        .expect("snippet should serialize");
        let parsed: SnippetsDocument = toml::from_str(&content).expect("snippet should parse");

        assert_eq!(parsed.snippets, vec![snippet]);
    }
}
