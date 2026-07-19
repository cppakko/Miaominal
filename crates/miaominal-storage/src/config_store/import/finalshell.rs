use super::resolve_username;
use anyhow::{Context, Result};
use base64::Engine as _;
use miaominal_core::profile::{
    AuthMethod, DEFAULT_SESSION_CHARSET, ImportIssueKind, ImportSourceKind, ImportedBatch,
    ImportedSessionDraft,
};
use serde::Deserialize;
use std::path::Path;

pub(super) fn import(content: &str, path: &Path) -> Result<ImportedBatch> {
    let record: FinalShellConnection =
        serde_json::from_str(content).context("failed to parse FinalShell JSON")?;
    let mut batch = ImportedBatch::default();
    let name = if record.name.trim().is_empty() {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("FinalShell")
            .to_string()
    } else {
        record.name.trim().to_string()
    };

    if record.conection_type != 100 {
        batch.push_issue(
            ImportIssueKind::UnsupportedProtocol,
            name,
            format!(
                "FinalShell connection type {} is not SSH ({})",
                record.conection_type,
                finalshell_protocol_label(record.conection_type)
            ),
        );
        return Ok(batch);
    }

    let host = record.host.trim().to_string();
    if host.is_empty() {
        batch.push_issue(ImportIssueKind::MissingRequiredField, name, "missing host");
        return Ok(batch);
    }

    let Some(username) = resolve_username(Some(&record.user_name)) else {
        batch.push_issue(
            ImportIssueKind::MissingRequiredField,
            name,
            "missing username",
        );
        return Ok(batch);
    };

    let entry_name = if record.name.trim().is_empty() {
        host.clone()
    } else {
        record.name.trim().to_string()
    };
    let password = decode_probable_base64_secret(&record.password);
    if !record.password.trim().is_empty() && password.is_none() {
        batch.push_issue(
            ImportIssueKind::UnsupportedCredential,
            entry_name.clone(),
            "FinalShell stored password could not be decoded and was not imported",
        );
    }
    if !record.secret_key_id.trim().is_empty() {
        batch.push_issue(
            ImportIssueKind::UnsupportedFeature,
            entry_name.clone(),
            "FinalShell key references are not exported as local key paths",
        );
    }

    batch.sessions.push(ImportedSessionDraft {
        source: ImportSourceKind::FinalShellJson,
        name: entry_name,
        group: ImportSourceKind::FinalShellJson.label().to_string(),
        host,
        port: if record.port == 0 { 22 } else { record.port },
        username,
        password,
        auth_method: AuthMethod::Password,
        private_key_path: String::new(),
        certificate_path: String::new(),
        passphrase: None,
        agent_forwarding: false,
        startup_command: String::new(),
        charset: if record.terminal_encoding.trim().is_empty() {
            DEFAULT_SESSION_CHARSET.to_string()
        } else {
            record.terminal_encoding.trim().to_string()
        },
    });

    Ok(batch)
}

fn decode_probable_base64_secret(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(trimmed)
        .ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let decoded = decoded.trim_matches(char::from(0)).trim().to_string();
    if decoded.is_empty() {
        return None;
    }
    if decoded
        .chars()
        .all(|ch| !ch.is_control() || matches!(ch, '\n' | '\r' | '\t'))
    {
        Some(decoded)
    } else {
        None
    }
}

fn finalshell_protocol_label(connection_type: i64) -> &'static str {
    match connection_type {
        100 => "SSH",
        101 => "RDP",
        _ => "unknown",
    }
}

#[derive(Debug, Deserialize)]
struct FinalShellConnection {
    #[serde(default)]
    name: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    user_name: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    secret_key_id: String,
    #[serde(default)]
    conection_type: i64,
    #[serde(default)]
    terminal_encoding: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalshell_skips_non_ssh_and_warns_about_opaque_passwords() {
        let ssh_batch = import(
            r#"{
                    "name":"fs_test_01",
                    "host":"192.111.222.222",
                    "port":49771,
                    "user_name":"root",
                    "password":"bFIZKiUYYw/ZJPVRu6TPPqzVylyvFVVX",
                    "secret_key_id":"",
                    "conection_type":100,
                    "terminal_encoding":"UTF-8"
                }"#,
            Path::new("fs_test_01_connect_config.json"),
        )
        .expect("FinalShell JSON should parse");

        assert_eq!(ssh_batch.sessions.len(), 1);
        assert_eq!(
            ssh_batch.issue_count(ImportIssueKind::UnsupportedCredential),
            1
        );
        assert!(ssh_batch.sessions[0].password.is_none());

        let rdp_batch = import(
            r#"{
                    "name":"fs_test_03_rdp",
                    "host":"192.1.1.1",
                    "port":3389,
                    "user_name":"admin",
                    "password":"opaque",
                    "conection_type":101,
                    "terminal_encoding":"UTF-8"
                }"#,
            Path::new("fs_test_03_rdp_connect_config.json"),
        )
        .expect("FinalShell JSON should parse");

        assert!(rdp_batch.sessions.is_empty());
        assert_eq!(
            rdp_batch.issue_count(ImportIssueKind::UnsupportedProtocol),
            1
        );
    }
}
