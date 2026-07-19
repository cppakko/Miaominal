use super::resolve_username;
use miaominal_core::profile::{
    AuthMethod, DEFAULT_SESSION_CHARSET, ImportIssueKind, ImportSourceKind, ImportedBatch,
    ImportedSessionDraft,
};
use std::collections::HashMap;

const PUTTY_SESSION_PREFIX: &str = "HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\";

pub(super) fn import(content: &str) -> ImportedBatch {
    let mut batch = ImportedBatch::default();
    let mut current_section: Option<String> = None;
    let mut current_values: HashMap<String, String> = HashMap::new();

    let finalize_section =
        |batch: &mut ImportedBatch, section: Option<&str>, values: &HashMap<String, String>| {
            let Some(section) = section else {
                return;
            };
            let Some(session_name) = section.strip_prefix(PUTTY_SESSION_PREFIX) else {
                return;
            };
            if session_name.is_empty() {
                return;
            }

            let display_name = decode_putty_session_name(session_name);
            let protocol = values
                .get("Protocol")
                .map(|value| value.trim().to_ascii_lowercase())
                .unwrap_or_default();
            if protocol != "ssh" {
                batch.push_issue(
                    ImportIssueKind::UnsupportedProtocol,
                    display_name,
                    if protocol.is_empty() {
                        "PuTTY session is missing the Protocol field".to_string()
                    } else {
                        format!("PuTTY protocol {protocol} is not supported")
                    },
                );
                return;
            }

            let host = values
                .get("HostName")
                .map(|value| value.trim().to_string())
                .unwrap_or_default();
            if host.is_empty() {
                batch.push_issue(
                    ImportIssueKind::MissingRequiredField,
                    session_name,
                    "missing HostName",
                );
                return;
            }

            let Some(username) = resolve_username(values.get("UserName")) else {
                batch.push_issue(
                    ImportIssueKind::MissingRequiredField,
                    session_name,
                    "missing UserName",
                );
                return;
            };

            let private_key_path = values
                .get("PublicKeyFile")
                .map(|value| value.trim().to_string())
                .unwrap_or_default();
            let certificate_path = values
                .get("DetachedCertificate")
                .map(|value| value.trim().to_string())
                .unwrap_or_default();
            let startup_command = values
                .get("RemoteCommand")
                .map(|value| value.trim().to_string())
                .unwrap_or_default();
            let port = values
                .get("PortNumber")
                .and_then(|value| parse_registry_dword(value))
                .and_then(|value| u16::try_from(value).ok())
                .unwrap_or(22);
            let agent_forwarding = values
                .get("AgentFwd")
                .and_then(|value| parse_registry_dword(value))
                .is_some_and(|value| value != 0);

            batch.sessions.push(ImportedSessionDraft {
                source: ImportSourceKind::PuttyRegistry,
                name: display_name,
                group: ImportSourceKind::PuttyRegistry.label().to_string(),
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
        };

    for raw_line in content.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            finalize_section(&mut batch, current_section.as_deref(), &current_values);
            current_section = Some(trimmed[1..trimmed.len() - 1].to_string());
            current_values.clear();
            continue;
        }

        let Some((key, value)) = parse_registry_value(trimmed) else {
            continue;
        };
        current_values.insert(key, value);
    }

    finalize_section(&mut batch, current_section.as_deref(), &current_values);
    batch
}

fn parse_registry_value(line: &str) -> Option<(String, String)> {
    let (key, value) = line.split_once('=')?;
    let key = key.trim().trim_matches('"').to_string();
    let value = value.trim();
    let value = if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        value[1..value.len() - 1]
            .replace(r#"\\"#, r#"\"#)
            .replace(r#"\n"#, "\n")
            .replace(r#"\r"#, "\r")
    } else {
        value.to_string()
    };
    Some((key, value))
}

fn parse_registry_dword(value: &str) -> Option<u32> {
    value
        .trim()
        .strip_prefix("dword:")
        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
}

fn decode_putty_session_name(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = from_hex(bytes[index + 1]);
            let low = from_hex(bytes[index + 2]);
            if let (Some(high), Some(low)) = (high, low) {
                decoded.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn putty_imports_only_ssh_sessions() {
        let batch = import(
            r#"
                    Windows Registry Editor Version 5.00

                    [HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\ssh-prod]
                    "HostName"="192.168.0.10"
                    "PortNumber"=dword:00000016
                    "UserName"="root"
                    "PublicKeyFile"="C:\\Keys\\id_ed25519"
                    "Protocol"="ssh"

                    [HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\rdp-box]
                    "HostName"="192.168.0.11"
                    "Protocol"="telnet"
                "#,
        );

        assert_eq!(batch.sessions.len(), 1);
        let profile = &batch.sessions[0];
        assert_eq!(profile.name, "ssh-prod");
        assert_eq!(profile.host, "192.168.0.10");
        assert_eq!(profile.port, 22);
        assert_eq!(profile.auth_method, AuthMethod::KeyFile);
        assert_eq!(batch.issue_count(ImportIssueKind::UnsupportedProtocol), 1);
    }
}
