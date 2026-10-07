#[path = "import/finalshell.rs"]
mod finalshell;
#[path = "import/openssh.rs"]
mod openssh;
#[path = "import/putty.rs"]
mod putty;
#[path = "import/securecrt.rs"]
mod securecrt;

use anyhow::{Context, Result, anyhow, bail};
use miaominal_core::profile::{ImportSourceKind, ImportedBatch};
use std::fs;
use std::path::Path;

pub fn import_profiles_from_path(source: ImportSourceKind, path: &Path) -> Result<ImportedBatch> {
    if !source.accepts_path(path) {
        return Err(anyhow!(
            "file {} does not match {} import",
            path.display(),
            source.label()
        ));
    }

    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    import_profiles_from_bytes(source, &path.to_string_lossy(), &bytes)
}

/// Parse profiles from file content that never touched a filesystem path.
///
/// Android documents arrive through SAF as `content://` URIs, so the host reads
/// the bytes and passes them here; the same BOM/UTF-16 decoding and per-source
/// parsers run as for [`import_profiles_from_path`]. `file_name` is only used
/// for the FinalShell display-name fallback and must not be treated as a path.
pub fn import_profiles_from_bytes(
    source: ImportSourceKind,
    file_name: &str,
    bytes: &[u8],
) -> Result<ImportedBatch> {
    let content = decode_text(bytes, file_name)?;
    import_profiles_from_content(source, file_name, &content)
}

/// Parse profiles from already-decoded text (the caller guarantees UTF-8).
pub fn import_profiles_from_content(
    source: ImportSourceKind,
    file_name: &str,
    content: &str,
) -> Result<ImportedBatch> {
    match source {
        ImportSourceKind::OpenSshConfig => Ok(openssh::import(content)),
        ImportSourceKind::PuttyRegistry => Ok(putty::import(content)),
        ImportSourceKind::SecureCrtXml => securecrt::import(content),
        ImportSourceKind::FinalShellJson => finalshell::import(content, Path::new(file_name)),
    }
}

fn decode_text(bytes: &[u8], label: &str) -> Result<String> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(rest.to_vec())
            .with_context(|| format!("failed to decode {label} as UTF-8"));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16(rest, true)
            .with_context(|| format!("failed to decode {label} as UTF-16LE"));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16(rest, false)
            .with_context(|| format!("failed to decode {label} as UTF-16BE"));
    }

    String::from_utf8(bytes.to_vec()).with_context(|| format!("failed to decode {label} as UTF-8"))
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Result<String> {
    if !bytes.len().is_multiple_of(2) {
        bail!("UTF-16 input has an odd number of bytes");
    }

    let words = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&chunk| {
            if little_endian {
                u16::from_le_bytes(chunk)
            } else {
                u16::from_be_bytes(chunk)
            }
        })
        .collect::<Vec<_>>();
    String::from_utf16(&words).context("failed to decode UTF-16 data")
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

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::profile::{
        AuthMethod, DEFAULT_SESSION_CHARSET, ImportSourceKind, ImportedSessionDraft,
    };

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

    #[test]
    fn byte_entry_parses_utf8_and_utf16_without_a_path() {
        let utf8 = b"Host demo\n  HostName demo.example.com\n  User akko\n";
        let batch = import_profiles_from_bytes(ImportSourceKind::OpenSshConfig, "config", utf8)
            .expect("UTF-8 content should parse");
        assert_eq!(batch.sessions.len(), 1);
        assert_eq!(batch.sessions[0].host, "demo.example.com");

        let mut utf16 = vec![0xFF, 0xFE];
        for word in "Host demo16\n  HostName demo16.example.com\n  User akko\n".encode_utf16() {
            utf16.extend_from_slice(&word.to_le_bytes());
        }
        let batch = import_profiles_from_bytes(ImportSourceKind::OpenSshConfig, "config", &utf16)
            .expect("UTF-16LE content should parse");
        assert_eq!(batch.sessions.len(), 1);
        assert_eq!(batch.sessions[0].host, "demo16.example.com");

        let invalid = import_profiles_from_bytes(ImportSourceKind::SecureCrtXml, "x.xml", &[0xFF]);
        assert!(invalid.is_err(), "invalid UTF-8 must be reported");
    }

    #[test]
    fn finalshell_byte_entry_uses_the_file_name_for_the_display_name() {
        let content = br#"{
            "host":"fs.example.com",
            "user_name":"root",
            "conection_type":100
        }"#;
        let batch = import_profiles_from_bytes(
            ImportSourceKind::FinalShellJson,
            "fs_named_connection.json",
            content,
        )
        .expect("FinalShell JSON should parse");
        assert_eq!(batch.sessions.len(), 1);
        assert_eq!(batch.sessions[0].name, "fs.example.com");
    }
}
