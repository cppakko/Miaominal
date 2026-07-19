use super::resolve_username;
use miaominal_core::profile::{
    AuthMethod, DEFAULT_SESSION_CHARSET, ImportIssueKind, ImportSourceKind, ImportedBatch,
    ImportedSessionDraft,
};
use std::collections::HashMap;

pub(super) fn import(content: &str) -> ImportedBatch {
    let mut batch = ImportedBatch::default();
    let mut global_options = HashMap::new();
    let mut current_patterns: Vec<String> = Vec::new();
    let mut current_options: HashMap<String, String> = HashMap::new();

    let finalize_entry = |batch: &mut ImportedBatch,
                          patterns: &[String],
                          options: &HashMap<String, String>,
                          globals: &HashMap<String, String>| {
        if patterns.is_empty() {
            return;
        }

        let mut merged = globals.clone();
        merged.extend(
            options
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        let mut block_has_literal_pattern = false;

        for pattern in patterns {
            if !is_literal_ssh_host_pattern(pattern) {
                batch.push_issue(
                    ImportIssueKind::UnsupportedFeature,
                    pattern,
                    "OpenSSH wildcard or negated Host patterns are not supported",
                );
                continue;
            }

            block_has_literal_pattern = true;
            let host = merged
                .get("hostname")
                .cloned()
                .unwrap_or_else(|| pattern.clone())
                .trim()
                .to_string();
            if host.is_empty() {
                batch.push_issue(
                    ImportIssueKind::MissingRequiredField,
                    pattern,
                    "missing host name",
                );
                continue;
            }

            let Some(username) = resolve_username(merged.get("user")) else {
                batch.push_issue(
                    ImportIssueKind::MissingRequiredField,
                    pattern,
                    "missing username",
                );
                continue;
            };

            if merged.contains_key("proxyjump") {
                batch.push_issue(
                    ImportIssueKind::UnsupportedFeature,
                    pattern,
                    "ProxyJump is not imported yet",
                );
            }
            if merged.contains_key("include") {
                batch.push_issue(
                    ImportIssueKind::UnsupportedFeature,
                    pattern,
                    "Include directives are not expanded during import",
                );
            }

            let private_key_path = merged
                .get("identityfile")
                .map(|value| value.trim().to_string())
                .unwrap_or_default();
            let certificate_path = merged
                .get("certificatefile")
                .map(|value| value.trim().to_string())
                .unwrap_or_default();
            let startup_command = merged
                .get("remotecommand")
                .map(|value| value.trim().to_string())
                .unwrap_or_default();
            let port = merged
                .get("port")
                .and_then(|value| value.trim().parse::<u16>().ok())
                .unwrap_or(22);
            let agent_forwarding = merged
                .get("forwardagent")
                .is_some_and(|value| parse_truthy_value(value));

            batch.sessions.push(ImportedSessionDraft {
                source: ImportSourceKind::OpenSshConfig,
                name: pattern.trim().to_string(),
                group: ImportSourceKind::OpenSshConfig.label().to_string(),
                host,
                port,
                username,
                password: None,
                auth_method: if private_key_path.is_empty() {
                    AuthMethod::Password
                } else {
                    AuthMethod::KeyFile
                },
                private_key_path,
                certificate_path,
                passphrase: None,
                agent_forwarding,
                startup_command,
                charset: DEFAULT_SESSION_CHARSET.to_string(),
            });
        }

        if !block_has_literal_pattern {
            batch.push_issue(
                ImportIssueKind::InvalidEntry,
                patterns.join(", "),
                "no importable OpenSSH host aliases were found in this block",
            );
        }
    };

    for raw_line in content.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let Some((key, value)) = split_ssh_option(trimmed) else {
            continue;
        };
        if key.eq_ignore_ascii_case("host") {
            finalize_entry(
                &mut batch,
                &current_patterns,
                &current_options,
                &global_options,
            );
            current_patterns = value
                .split_whitespace()
                .map(str::trim)
                .filter(|pattern| !pattern.is_empty())
                .map(str::to_string)
                .collect();
            current_options.clear();
            continue;
        }

        let normalized_key = key.to_ascii_lowercase();
        if current_patterns.is_empty() {
            global_options.insert(normalized_key, value.trim().to_string());
        } else {
            current_options.insert(normalized_key, value.trim().to_string());
        }
    }

    finalize_entry(
        &mut batch,
        &current_patterns,
        &current_options,
        &global_options,
    );
    batch
}

fn split_ssh_option(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim();
    let split_at = trimmed.find(|ch: char| ch.is_whitespace() || ch == '=')?;
    let key = trimmed[..split_at].trim();
    let value = trimmed[split_at..]
        .trim_start_matches(|ch: char| ch.is_whitespace() || ch == '=')
        .trim();
    if key.is_empty() || value.is_empty() {
        None
    } else {
        Some((key, value))
    }
}

fn is_literal_ssh_host_pattern(pattern: &str) -> bool {
    let trimmed = pattern.trim();
    !trimmed.is_empty()
        && !trimmed.starts_with('!')
        && !trimmed.chars().any(|ch| ch == '*' || ch == '?')
}

fn parse_truthy_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "yes" | "true" | "1" | "on"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openssh_imports_literal_hosts_and_skips_wildcards() {
        let batch = import(
            r#"
                    User deploy
                    Port 2222

                    Host app-prod
                      HostName 10.0.0.4
                      IdentityFile ~/.ssh/id_ed25519
                      CertificateFile ~/.ssh/id_ed25519-cert.pub
                      ForwardAgent yes

                    Host dev-*
                      HostName 10.0.0.9
                "#,
        );

        assert_eq!(batch.sessions.len(), 1);
        let profile = &batch.sessions[0];
        assert_eq!(profile.name, "app-prod");
        assert_eq!(profile.host, "10.0.0.4");
        assert_eq!(profile.port, 2222);
        assert_eq!(profile.username, "deploy");
        assert_eq!(profile.auth_method, AuthMethod::KeyFile);
        assert!(profile.agent_forwarding);
        assert_eq!(profile.certificate_path, "~/.ssh/id_ed25519-cert.pub");
        assert_eq!(batch.issue_count(ImportIssueKind::UnsupportedFeature), 1);
    }
}
