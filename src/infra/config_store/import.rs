use crate::domain::profile::{AuthMethod, DEFAULT_SESSION_CHARSET};
use crate::domain::profile::{ImportIssueKind, ImportedSessionDraft};
use crate::domain::profile::{ImportSourceKind, ImportedBatch};
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use roxmltree::{Document, Node};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

const PUTTY_SESSION_PREFIX: &str = "HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\";

#[derive(Debug, Default)]
struct SecureCrtImportContext {
    credential_profiles: HashMap<String, HashMap<String, String>>,
    global_ssh2: HashMap<String, String>,
}

pub fn import_profiles_from_path(source: ImportSourceKind, path: &Path) -> Result<ImportedBatch> {
    if !source.accepts_path(path) {
        return Err(anyhow!(
            "file {} does not match {} import",
            path.display(),
            source.label()
        ));
    }

    let content = read_text_file(path)?;
    match source {
        ImportSourceKind::OpenSshConfig => Ok(import_openssh_config(&content)),
        ImportSourceKind::PuttyRegistry => Ok(import_putty_registry(&content)),
        ImportSourceKind::SecureCrtXml => import_securecrt_xml(&content),
        ImportSourceKind::FinalShellJson => import_finalshell_json(&content, path),
    }
}

fn import_openssh_config(content: &str) -> ImportedBatch {
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

fn import_putty_registry(content: &str) -> ImportedBatch {
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

fn import_securecrt_xml(content: &str) -> Result<ImportedBatch> {
    let document = Document::parse(content).context("failed to parse SecureCRT XML")?;
    let sessions_root = document
        .descendants()
        .find(|node| is_named_key(*node, "Sessions"))
        .ok_or_else(|| anyhow!("SecureCRT XML does not contain a Sessions root"))?;
    let context = build_securecrt_import_context(&document);

    let mut batch = ImportedBatch::default();
    let mut folders = Vec::new();
    collect_securecrt_entries(sessions_root, &context, &mut folders, &mut batch);
    Ok(batch)
}

fn import_finalshell_json(content: &str, path: &Path) -> Result<ImportedBatch> {
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

fn collect_securecrt_entries(
    node: Node<'_, '_>,
    context: &SecureCrtImportContext,
    folders: &mut Vec<String>,
    batch: &mut ImportedBatch,
) {
    for child in node.children().filter(|child| child.is_element()) {
        if child.tag_name().name() != "key" {
            continue;
        }

        let Some(name) = child.attribute("name") else {
            continue;
        };
        if is_securecrt_session_node(child) {
            parse_securecrt_session(child, context, folders, name, batch);
            continue;
        }

        folders.push(name.to_string());
        collect_securecrt_entries(child, context, folders, batch);
        folders.pop();
    }
}

fn parse_securecrt_session(
    node: Node<'_, '_>,
    context: &SecureCrtImportContext,
    folders: &[String],
    session_name: &str,
    batch: &mut ImportedBatch,
) {
    let fields = collect_securecrt_fields(node);
    let credential_title = securecrt_field(&fields, "Credential Title");
    let credential_fields =
        credential_title.and_then(|title| context.credential_profiles.get(title));
    if credential_title.is_some() && credential_fields.is_none() {
        batch.push_issue(
            ImportIssueKind::InvalidEntry,
            session_name,
            format!(
                "SecureCRT credential profile {} was not found",
                credential_title.unwrap_or_default()
            ),
        );
    }

    let protocol = fields
        .get("Protocol Name")
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if !protocol.starts_with("ssh") {
        batch.push_issue(
            ImportIssueKind::UnsupportedProtocol,
            session_name,
            if protocol.is_empty() {
                "SecureCRT session is missing Protocol Name".to_string()
            } else {
                format!("SecureCRT protocol {protocol} is not supported")
            },
        );
        return;
    }

    let host = fields
        .get("Hostname")
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    if host.is_empty() {
        batch.push_issue(
            ImportIssueKind::MissingRequiredField,
            session_name,
            "missing Hostname",
        );
        return;
    }

    let Some(username) = resolve_username_text(
        first_non_empty_securecrt_field("Username", &fields, credential_fields, None).as_deref(),
    ) else {
        batch.push_issue(
            ImportIssueKind::MissingRequiredField,
            session_name,
            "missing Username",
        );
        return;
    };

    let uses_global_public_key = securecrt_uses_global_public_key(&fields, credential_fields);
    let private_key_path = first_non_empty_securecrt_field(
        "Identity Filename V2",
        &fields,
        credential_fields,
        uses_global_public_key.then_some(&context.global_ssh2),
    )
    .unwrap_or_default();
    let mut certificate_path = String::new();
    if matches!(
        first_non_empty_securecrt_field(
            "Use Associated OpenSSH Certificate",
            &fields,
            credential_fields,
            uses_global_public_key.then_some(&context.global_ssh2),
        )
        .as_deref(),
        Some("1")
    ) {
        certificate_path = first_non_empty_securecrt_field(
            "Associated OpenSSH Certificate",
            &fields,
            credential_fields,
            uses_global_public_key.then_some(&context.global_ssh2),
        )
        .unwrap_or_default();
    }

    let port = first_non_empty_securecrt_field("[SSH2] Port", &fields, credential_fields, None)
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(22);
    let startup_command = if matches!(
        first_non_empty_securecrt_field("Use Shell Command", &fields, credential_fields, None,)
            .as_deref(),
        Some("1")
    ) {
        first_non_empty_securecrt_field("Shell Command", &fields, credential_fields, None)
            .unwrap_or_default()
    } else {
        String::new()
    };

    let agent_forwarding = resolve_securecrt_agent_forwarding(
        &fields,
        credential_fields,
        &context.global_ssh2,
        session_name,
        batch,
    );

    if first_non_empty_securecrt_field("Password V2", &fields, credential_fields, None)
        .is_some_and(|value| !value.is_empty())
        || first_non_empty_securecrt_field(
            "Session Password Saved",
            &fields,
            credential_fields,
            None,
        )
        .is_some_and(|value| value == "1")
    {
        batch.push_issue(
            ImportIssueKind::UnsupportedCredential,
            session_name,
            "SecureCRT stored passwords are encrypted and were not imported",
        );
    }

    if uses_global_public_key && private_key_path.is_empty() {
        batch.push_issue(
            ImportIssueKind::UnsupportedFeature,
            session_name,
            "SecureCRT is configured to use a global public key; no session key path was exported",
        );
    }

    batch.sessions.push(ImportedSessionDraft {
        source: ImportSourceKind::SecureCrtXml,
        name: session_name.to_string(),
        group: if folders.is_empty() {
            ImportSourceKind::SecureCrtXml.label().to_string()
        } else {
            folders.join("/")
        },
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
        charset: fields
            .get("Output Transformer Name")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_SESSION_CHARSET.to_string()),
    });
}

fn build_securecrt_import_context(document: &Document<'_>) -> SecureCrtImportContext {
    let mut context = SecureCrtImportContext::default();
    if let Some(credentials_root) = document
        .descendants()
        .find(|node| is_named_key(*node, "Credentials"))
    {
        collect_securecrt_credential_profiles(credentials_root, &mut context.credential_profiles);
    }
    if let Some(global_ssh2_root) = document
        .descendants()
        .find(|node| is_named_key(*node, "SSH2"))
    {
        context.global_ssh2 = collect_securecrt_fields(global_ssh2_root);
    }
    context
}

fn collect_securecrt_credential_profiles(
    node: Node<'_, '_>,
    profiles: &mut HashMap<String, HashMap<String, String>>,
) {
    for child in node.children().filter(|child| child.is_element()) {
        if child.tag_name().name() != "key" {
            continue;
        }

        let Some(name) = child.attribute("name") else {
            continue;
        };
        let fields = collect_securecrt_fields(child);
        if !fields.is_empty() {
            profiles.insert(name.to_string(), fields);
        }
        collect_securecrt_credential_profiles(child, profiles);
    }
}

fn collect_securecrt_fields(node: Node<'_, '_>) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    for child in node.children().filter(|child| child.is_element()) {
        let tag_name = child.tag_name().name();
        if tag_name != "string" && tag_name != "dword" {
            continue;
        }
        let Some(name) = child.attribute("name") else {
            continue;
        };
        fields.insert(
            name.to_string(),
            child.text().unwrap_or("").trim().to_string(),
        );
    }
    fields
}

fn read_text_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(rest.to_vec())
            .with_context(|| format!("failed to decode {} as UTF-8", path.display()));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16(rest, true)
            .with_context(|| format!("failed to decode {} as UTF-16LE", path.display()));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16(rest, false)
            .with_context(|| format!("failed to decode {} as UTF-16BE", path.display()));
    }

    String::from_utf8(bytes)
        .with_context(|| format!("failed to decode {} as UTF-8", path.display()))
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Result<String> {
    if bytes.len() % 2 != 0 {
        bail!("UTF-16 input has an odd number of bytes");
    }

    let words = bytes
        .chunks_exact(2)
        .map(|chunk| {
            if little_endian {
                u16::from_le_bytes([chunk[0], chunk[1]])
            } else {
                u16::from_be_bytes([chunk[0], chunk[1]])
            }
        })
        .collect::<Vec<_>>();
    String::from_utf16(&words).context("failed to decode UTF-16 data")
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

fn securecrt_field<'a>(fields: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    fields
        .get(key)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn first_non_empty_securecrt_field(
    key: &str,
    session_fields: &HashMap<String, String>,
    credential_fields: Option<&HashMap<String, String>>,
    global_fields: Option<&HashMap<String, String>>,
) -> Option<String> {
    [Some(session_fields), credential_fields, global_fields]
        .into_iter()
        .flatten()
        .find_map(|fields| securecrt_field(fields, key).map(str::to_string))
}

fn securecrt_uses_global_public_key(
    session_fields: &HashMap<String, String>,
    credential_fields: Option<&HashMap<String, String>>,
) -> bool {
    matches!(
        first_non_empty_securecrt_field(
            "Use Global Public Key",
            session_fields,
            credential_fields,
            None,
        )
        .as_deref(),
        Some("1")
    )
}

fn resolve_securecrt_agent_forwarding(
    session_fields: &HashMap<String, String>,
    credential_fields: Option<&HashMap<String, String>>,
    global_fields: &HashMap<String, String>,
    session_name: &str,
    batch: &mut ImportedBatch,
) -> bool {
    for fields in [Some(session_fields), credential_fields, Some(global_fields)]
        .into_iter()
        .flatten()
    {
        let Some(value) = securecrt_field(fields, "Enable Agent Forwarding") else {
            continue;
        };
        match value {
            "1" => return true,
            "0" => return false,
            "2" => continue,
            _ => {
                batch.push_issue(
                    ImportIssueKind::UnsupportedFeature,
                    session_name,
                    format!("SecureCRT agent forwarding setting {value} could not be resolved"),
                );
                return false;
            }
        }
    }

    false
}

fn resolve_username_text(value: Option<&str>) -> Option<String> {
    let username = value.map(str::trim).unwrap_or_default();
    if !username.is_empty() {
        return Some(username.to_string());
    }

    let fallback = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let fallback = fallback.trim();
    (!fallback.is_empty()).then(|| fallback.to_string())
}

fn resolve_username(value: Option<&String>) -> Option<String> {
    resolve_username_text(value.map(String::as_str))
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

fn is_named_key(node: Node<'_, '_>, expected_name: &str) -> bool {
    node.is_element()
        && node.tag_name().name() == "key"
        && node.attribute("name") == Some(expected_name)
}

fn is_securecrt_session_node(node: Node<'_, '_>) -> bool {
    node.children().any(|child| {
        child.is_element()
            && child.tag_name().name() == "dword"
            && child.attribute("name") == Some("Is Session")
            && child.text().is_some_and(|value| value.trim() == "1")
    })
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
    fn openssh_imports_literal_hosts_and_skips_wildcards() {
        let batch = import_openssh_config(
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

    #[test]
    fn putty_imports_only_ssh_sessions() {
        let batch = import_putty_registry(
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

    #[test]
    fn securecrt_imports_only_ssh_sessions() {
        let batch = import_securecrt_xml(
            r#"<?xml version="1.0" encoding="UTF-8"?>
                <VanDyke version="3.0">
                  <key name="Sessions">
                    <key name="Servers">
                      <key name="ssh-prod">
                        <dword name="Is Session">1</dword>
                        <string name="Protocol Name">SSH2</string>
                        <string name="Hostname">10.10.10.10</string>
                        <string name="Username">ubuntu</string>
                        <dword name="[SSH2] Port">22</dword>
                        <string name="Identity Filename V2">C:\\Keys\\id_rsa</string>
                        <string name="Output Transformer Name">UTF-8</string>
                      </key>
                      <key name="rdp-box">
                        <dword name="Is Session">1</dword>
                        <string name="Protocol Name">RDP</string>
                        <string name="Hostname">10.10.10.11</string>
                      </key>
                    </key>
                  </key>
                </VanDyke>"#,
        )
        .expect("SecureCRT XML should parse");

        assert_eq!(batch.sessions.len(), 1);
        let profile = &batch.sessions[0];
        assert_eq!(profile.group, "Servers");
        assert_eq!(profile.name, "ssh-prod");
        assert_eq!(profile.host, "10.10.10.10");
        assert_eq!(profile.username, "ubuntu");
        assert_eq!(profile.auth_method, AuthMethod::KeyFile);
        assert_eq!(batch.issue_count(ImportIssueKind::UnsupportedProtocol), 1);
    }

    #[test]
    fn securecrt_inherits_credential_profiles_and_global_defaults() {
        let batch = import_securecrt_xml(
                        r#"<?xml version="1.0" encoding="UTF-8"?>
                                <VanDyke version="3.0">
                                    <key name="Sessions">
                                        <key name="Servers">
                                            <key name="ssh-prod">
                                                <dword name="Is Session">1</dword>
                                                <string name="Protocol Name">SSH2</string>
                                                <string name="Hostname">10.10.10.10</string>
                                                <string name="Username"></string>
                                                <string name="Credential Title">SharedCreds</string>
                                                <dword name="Use Global Public Key">1</dword>
                                                <dword name="Enable Agent Forwarding">2</dword>
                                            </key>
                                        </key>
                                    </key>
                                    <key name="Credentials">
                                        <key name="SharedCreds">
                                            <string name="Username">deploy</string>
                                        </key>
                                    </key>
                                    <key name="SSH2">
                                        <string name="Identity Filename V2">C:\Keys\global_id_ed25519</string>
                                        <dword name="Enable Agent Forwarding">1</dword>
                                    </key>
                                </VanDyke>"#,
                )
                .expect("SecureCRT XML should parse");

        assert_eq!(batch.sessions.len(), 1);
        let profile = &batch.sessions[0];
        assert_eq!(profile.username, "deploy");
        assert_eq!(profile.private_key_path, "C:\\Keys\\global_id_ed25519");
        assert!(profile.agent_forwarding);
        assert_eq!(batch.issue_count(ImportIssueKind::UnsupportedFeature), 0);
    }

    #[test]
    fn finalshell_skips_non_ssh_and_warns_about_opaque_passwords() {
        let ssh_batch = import_finalshell_json(
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

        let rdp_batch = import_finalshell_json(
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

    #[test]
    fn imported_draft_marks_stored_secrets() {
        let profile = ImportedSessionDraft {
            source: ImportSourceKind::FinalShellJson,
            name: "Imported".into(),
            group: String::new(),
            host: "example.com".into(),
            port: 22,
            username: "root".into(),
            password: Some("secret".into()),
            auth_method: AuthMethod::Password,
            private_key_path: String::new(),
            certificate_path: String::new(),
            passphrase: Some("phrase".into()),
            agent_forwarding: false,
            startup_command: String::new(),
            charset: DEFAULT_SESSION_CHARSET.to_string(),
        }
        .into_session_profile("session-99".into());

        assert_eq!(profile.id, "session-99");
        assert_eq!(profile.group, "FinalShell");
        assert!(profile.has_stored_password);
        assert!(profile.has_stored_passphrase);
    }
}
