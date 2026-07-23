use super::resolve_username;
use miaominal_core::profile::{
    AuthMethod, DEFAULT_SESSION_CHARSET, ImportField, ImportIssueKind, ImportIssueReason,
    ImportSourceKind, ImportedBatch, ImportedSessionDraft,
};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Default)]
struct SshConfigBlock {
    patterns: Option<Vec<String>>,
    directives: Vec<(String, String)>,
}

pub(super) fn import(content: &str) -> ImportedBatch {
    let (blocks, mut batch) = parse_blocks(content);
    let aliases = collect_literal_aliases(&blocks);

    for alias in aliases {
        import_alias(&alias, &blocks, &mut batch);
    }

    batch
}

fn parse_blocks(content: &str) -> (Vec<SshConfigBlock>, ImportedBatch) {
    let mut batch = ImportedBatch::default();
    let mut blocks = vec![SshConfigBlock::default()];
    let mut current_block = Some(0usize);

    for raw_line in content.lines() {
        let line = strip_unquoted_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }

        let Some((key, raw_value)) = split_ssh_option(line) else {
            continue;
        };
        let normalized_key = key.to_ascii_lowercase();
        let value = parse_ssh_value(raw_value);

        match normalized_key.as_str() {
            "host" => {
                let patterns = tokenize_ssh_arguments(raw_value);
                blocks.push(SshConfigBlock {
                    patterns: Some(patterns),
                    directives: Vec::new(),
                });
                current_block = Some(blocks.len() - 1);
            }
            "match" => {
                batch.push_issue(
                    ImportIssueKind::UnsupportedFeature,
                    "Match",
                    ImportIssueReason::MatchNotEvaluated { expression: value },
                );
                current_block = None;
            }
            "include" => {
                batch.push_issue(
                    ImportIssueKind::UnsupportedFeature,
                    if value.is_empty() { "Include" } else { &value },
                    ImportIssueReason::IncludeNotExpanded,
                );
            }
            _ => {
                if let Some(index) = current_block {
                    blocks[index].directives.push((normalized_key, value));
                }
            }
        }
    }

    (blocks, batch)
}

fn collect_literal_aliases(blocks: &[SshConfigBlock]) -> Vec<String> {
    let mut aliases = Vec::new();
    let mut seen = HashSet::new();

    for block in blocks {
        let Some(patterns) = &block.patterns else {
            continue;
        };
        for pattern in patterns {
            if !is_literal_positive_pattern(pattern) {
                continue;
            }
            let normalized = pattern.to_ascii_lowercase();
            if seen.insert(normalized) {
                aliases.push(pattern.clone());
            }
        }
    }

    aliases
}

fn import_alias(alias: &str, blocks: &[SshConfigBlock], batch: &mut ImportedBatch) {
    let mut options = HashMap::<String, String>::new();
    let mut identity_files = Vec::new();
    let mut proxy_jump_seen = false;

    for block in blocks {
        if let Some(patterns) = &block.patterns
            && !host_block_matches(patterns, alias)
        {
            continue;
        }

        for (key, value) in &block.directives {
            match key.as_str() {
                "identityfile" => identity_files.push(value.clone()),
                "proxyjump" => proxy_jump_seen = true,
                _ => {
                    options.entry(key.clone()).or_insert_with(|| value.clone());
                }
            }
        }
    }

    if proxy_jump_seen {
        batch.push_issue(
            ImportIssueKind::UnsupportedFeature,
            alias,
            ImportIssueReason::ProxyJumpNotImported,
        );
    }
    if identity_files.len() > 1 {
        batch.push_issue(
            ImportIssueKind::UnsupportedFeature,
            alias,
            ImportIssueReason::MultipleIdentityFiles,
        );
    }

    let host = options
        .get("hostname")
        .cloned()
        .unwrap_or_else(|| alias.to_string())
        .trim()
        .to_string();
    if host.is_empty() {
        batch.push_issue(
            ImportIssueKind::MissingRequiredField,
            alias,
            ImportIssueReason::MissingField {
                field: ImportField::Host,
            },
        );
        return;
    }

    let Some(username) = resolve_username(options.get("user")) else {
        batch.push_issue(
            ImportIssueKind::MissingRequiredField,
            alias,
            ImportIssueReason::MissingField {
                field: ImportField::Username,
            },
        );
        return;
    };

    let Some(port) = parse_port(options.get("port"), alias, batch) else {
        return;
    };
    let private_key_path = identity_files.first().cloned().unwrap_or_default();
    let certificate_path = options
        .get("certificatefile")
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    let startup_command = options
        .get("remotecommand")
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    let agent_forwarding = options
        .get("forwardagent")
        .is_some_and(|value| parse_truthy_value(value));

    batch.sessions.push(ImportedSessionDraft {
        source: ImportSourceKind::OpenSshConfig,
        name: alias.trim().to_string(),
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

fn parse_port(value: Option<&String>, entry_name: &str, batch: &mut ImportedBatch) -> Option<u16> {
    let Some(value) = value else {
        return Some(22);
    };
    match value.trim().parse::<u16>() {
        Ok(port) if port != 0 => Some(port),
        _ => {
            batch.push_issue(
                ImportIssueKind::InvalidEntry,
                entry_name,
                ImportIssueReason::InvalidPort {
                    value: value.clone(),
                },
            );
            None
        }
    }
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

fn parse_ssh_value(value: &str) -> String {
    tokenize_ssh_arguments(value).join(" ")
}

fn tokenize_ssh_arguments(value: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\'' | '"' => {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(ch);
                } else {
                    current.push(ch);
                }
            }
            '\\' => {
                if let Some(next) = chars.peek().copied()
                    && (next.is_whitespace() || matches!(next, '\\' | '\'' | '"' | '#'))
                {
                    current.push(chars.next().expect("peeked character should exist"));
                } else {
                    current.push(ch);
                }
            }
            ch if ch.is_whitespace() && quote.is_none() => {
                if !current.is_empty() {
                    values.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        values.push(current);
    }
    values
}

fn strip_unquoted_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if matches!(ch, '\'' | '"') {
            if quote == Some(ch) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(ch);
            }
            continue;
        }
        if ch == '#' && quote.is_none() {
            return &line[..index];
        }
    }
    line
}

fn is_literal_positive_pattern(pattern: &str) -> bool {
    let pattern = pattern.trim();
    !pattern.is_empty()
        && !pattern.starts_with('!')
        && !pattern.chars().any(|ch| matches!(ch, '*' | '?'))
}

fn host_block_matches(patterns: &[String], alias: &str) -> bool {
    let mut positive_match = false;
    for pattern in patterns {
        let (negated, pattern) = pattern
            .strip_prefix('!')
            .map_or((false, pattern.as_str()), |pattern| (true, pattern));
        if wildcard_matches(pattern, alias) {
            if negated {
                return false;
            }
            positive_match = true;
        }
    }
    positive_match
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase().into_bytes();
    let value = value.to_ascii_lowercase().into_bytes();
    let (mut pattern_index, mut value_index) = (0usize, 0usize);
    let mut star_index = None;
    let mut star_value_index = 0usize;

    while value_index < value.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?' || pattern[pattern_index] == value[value_index])
        {
            pattern_index += 1;
            value_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star_index = Some(pattern_index);
            pattern_index += 1;
            star_value_index = value_index;
        } else if let Some(star) = star_index {
            pattern_index = star + 1;
            star_value_index += 1;
            value_index = star_value_index;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
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
    fn openssh_applies_wildcard_defaults_after_specific_hosts() {
        let batch = import(
            r#"
                Host app-prod
                  HostName 10.0.0.4
                  User specific
                  User ignored

                Host *
                  User deploy
                  Port 2222
            "#,
        );

        assert_eq!(batch.sessions.len(), 1);
        let profile = &batch.sessions[0];
        assert_eq!(profile.name, "app-prod");
        assert_eq!(profile.host, "10.0.0.4");
        assert_eq!(profile.port, 2222);
        assert_eq!(profile.username, "specific");
    }

    #[test]
    fn openssh_applies_wildcard_defaults_before_specific_hosts_with_first_value_wins() {
        let batch = import(
            r#"
                Host *
                  User deploy
                  Port 2200

                Host app-prod
                  HostName 10.0.0.4
                  Port 2222
            "#,
        );

        assert_eq!(batch.sessions.len(), 1);
        assert_eq!(batch.sessions[0].port, 2200);
        assert_eq!(batch.sessions[0].username, "deploy");
    }

    #[test]
    fn openssh_merges_duplicate_alias_blocks_and_respects_negation() {
        let batch = import(
            r#"
                Host app-prod
                  HostName 10.0.0.4

                Host app-* !app-prod
                  Port 2200

                Host app-prod
                  User deploy
                  Port 2222
            "#,
        );

        assert_eq!(batch.sessions.len(), 1);
        assert_eq!(batch.sessions[0].username, "deploy");
        assert_eq!(batch.sessions[0].port, 2222);
    }

    #[test]
    fn openssh_skips_match_blocks_and_handles_quoted_paths_and_comments() {
        let batch = import(
            r#"
                Host app-prod
                  HostName 10.0.0.4 # production
                  User deploy
                  IdentityFile "C:\Keys\prod key"
                  IdentityFile ~/.ssh/fallback
                Match host other
                  Port 2200
                Host *
                  Port 2222
            "#,
        );

        assert_eq!(batch.sessions.len(), 1);
        assert_eq!(batch.sessions[0].port, 2222);
        assert_eq!(batch.sessions[0].private_key_path, "C:\\Keys\\prod key");
        assert_eq!(batch.issue_count(ImportIssueKind::UnsupportedFeature), 2);
    }

    #[test]
    fn openssh_skips_profiles_with_invalid_explicit_ports() {
        let batch = import(
            r#"
                Host bad
                  User deploy
                  Port 0
                Host good
                  User deploy
            "#,
        );

        assert_eq!(batch.sessions.len(), 1);
        assert_eq!(batch.sessions[0].name, "good");
        assert_eq!(batch.sessions[0].port, 22);
        assert_eq!(batch.issue_count(ImportIssueKind::InvalidEntry), 1);
    }
}
