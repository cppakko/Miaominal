use super::resolve_username_text;
use anyhow::{Context, Result, anyhow};
use miaominal_core::profile::{
    AuthMethod, DEFAULT_SESSION_CHARSET, ImportField, ImportIssueKind, ImportIssueReason,
    ImportSourceKind, ImportedBatch, ImportedSessionDraft,
};
use roxmltree::{Document, Node};
use std::collections::HashMap;

#[derive(Debug, Default)]
struct SecureCrtImportContext {
    credential_profiles: HashMap<String, HashMap<String, String>>,
    global_ssh2: HashMap<String, String>,
}

pub(super) fn import(content: &str) -> Result<ImportedBatch> {
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
            ImportIssueReason::CredentialProfileNotFound {
                profile: credential_title.unwrap_or_default().to_string(),
            },
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
                ImportIssueReason::MissingField {
                    field: ImportField::Protocol,
                }
            } else {
                ImportIssueReason::UnsupportedProtocol { protocol }
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
            ImportIssueReason::MissingField {
                field: ImportField::Host,
            },
        );
        return;
    }

    let Some(username) = resolve_username_text(
        first_non_empty_securecrt_field("Username", &fields, credential_fields, None).as_deref(),
    ) else {
        batch.push_issue(
            ImportIssueKind::MissingRequiredField,
            session_name,
            ImportIssueReason::MissingField {
                field: ImportField::Username,
            },
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

    let port_value =
        first_non_empty_securecrt_field("[SSH2] Port", &fields, credential_fields, None);
    let port = match port_value {
        None => 22,
        Some(value) => match value.parse::<u16>().ok().filter(|port| *port != 0) {
            Some(port) => port,
            None => {
                batch.push_issue(
                    ImportIssueKind::InvalidEntry,
                    session_name,
                    ImportIssueReason::InvalidPort { value },
                );
                return;
            }
        },
    };
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
            ImportIssueReason::EncryptedPasswordNotImported,
        );
    }

    if uses_global_public_key && private_key_path.is_empty() {
        batch.push_issue(
            ImportIssueKind::UnsupportedFeature,
            session_name,
            ImportIssueReason::GlobalPublicKeyPathMissing,
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
                    ImportIssueReason::AgentForwardingUnresolved {
                        value: value.to_string(),
                    },
                );
                return false;
            }
        }
    }

    false
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn securecrt_imports_only_ssh_sessions() {
        let batch = import(
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
        let batch = import(
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
    fn securecrt_skips_invalid_ports_and_defaults_missing_ports() {
        let batch = import(
            r#"<?xml version="1.0" encoding="UTF-8"?>
                <VanDyke version="3.0">
                  <key name="Sessions">
                    <key name="bad">
                      <dword name="Is Session">1</dword>
                      <string name="Protocol Name">SSH2</string>
                      <string name="Hostname">bad.example.com</string>
                      <string name="Username">root</string>
                      <dword name="[SSH2] Port">0</dword>
                    </key>
                    <key name="good">
                      <dword name="Is Session">1</dword>
                      <string name="Protocol Name">SSH2</string>
                      <string name="Hostname">good.example.com</string>
                      <string name="Username">root</string>
                    </key>
                  </key>
                </VanDyke>"#,
        )
        .expect("SecureCRT XML should parse");

        assert_eq!(batch.sessions.len(), 1);
        assert_eq!(batch.sessions[0].name, "good");
        assert_eq!(batch.sessions[0].port, 22);
        assert_eq!(batch.issue_count(ImportIssueKind::InvalidEntry), 1);
    }
}
