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

    let content = read_text_file(path)?;
    match source {
        ImportSourceKind::OpenSshConfig => Ok(openssh::import(&content)),
        ImportSourceKind::PuttyRegistry => Ok(putty::import(&content)),
        ImportSourceKind::SecureCrtXml => securecrt::import(&content),
        ImportSourceKind::FinalShellJson => finalshell::import(&content, path),
    }
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
    if !bytes.len().is_multiple_of(2) {
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
}
