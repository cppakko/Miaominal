use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

const FILE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RdpEndpoint {
    host: String,
    port: u16,
}

impl RdpEndpoint {
    pub fn new(host: impl Into<String>, port: u16) -> Result<Self> {
        let host = host.into();
        validate_value(&host)?;
        let host = host.trim().to_string();
        if host.is_empty() || port == 0 {
            bail!("RDP host and nonzero port are required");
        }
        let bracketed = host.starts_with('[') || host.ends_with(']');
        let host = if bracketed {
            host.strip_prefix('[')
                .and_then(|host| host.strip_suffix(']'))
                .context("invalid bracketed RDP address")?
        } else {
            host.as_str()
        };
        if bracketed || host.contains(':') {
            host.parse::<std::net::Ipv6Addr>()
                .context("invalid RDP IPv6 address")?;
        }
        Ok(Self {
            host: host.to_string(),
            port,
        })
    }
    pub fn host(&self) -> &str {
        &self.host
    }
    pub fn port(&self) -> u16 {
        self.port
    }
    pub fn address(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

pub(crate) fn validate_value(value: &str) -> Result<()> {
    if value
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}'))
    {
        bail!("RDP values cannot contain control characters");
    }
    Ok(())
}

pub fn render_connection_file(
    endpoint: &RdpEndpoint,
    username: &str,
    prompt: bool,
) -> Result<String> {
    validate_value(username)?;
    Ok(format!(
        "full address:s:{}\r\nusername:s:{}\r\nscreen mode id:i:2\r\nuse multimon:i:0\r\nredirectclipboard:i:1\r\ndrivestoredirect:s:\r\nredirectdrives:i:0\r\naudiomode:i:0\r\naudiocapturemode:i:0\r\nauthentication level:i:2\r\nprompt for credentials:i:{}\r\nautoreconnection enabled:i:1\r\n",
        endpoint.address(),
        username,
        u8::from(prompt)
    ))
}

pub(crate) fn render_remmina_file(endpoint: &RdpEndpoint, username: &str) -> Result<String> {
    validate_value(username)?;
    // GKeyFile uses backslash escapes, including in Windows domain usernames.
    let escape = |value: &str| value.replace('\\', "\\\\").replace(' ', "\\s");
    // Remmina passes `username` to FreeRDP verbatim; NTLM needs the domain separately.
    // UPN names (`user@domain`) stay intact.
    let (domain, username) = match username.split_once('\\') {
        Some((domain, user)) if !domain.is_empty() && !user.is_empty() => (domain, user),
        _ => ("", username),
    };
    Ok(format!(
        "[remmina]\nname=Remote Desktop\nprotocol=RDP\nserver={}\nusername={}\ndomain={}\nviewmode=4\nmultimon=0\nspan=0\ndisableclipboard=0\nsharefolder=\nsound=local\nmicrophone=0\ncert_ignore=0\ndisablepasswordstoring=1\n",
        escape(&endpoint.address()),
        escape(username),
        escape(domain)
    ))
}

fn directory() -> Result<PathBuf> {
    Ok(miaominal_paths::config_dir()?.join("cache").join("rdp"))
}

pub(crate) struct ConnectionFile {
    path: PathBuf,
}
impl ConnectionFile {
    pub(crate) fn new(contents: &str) -> Result<Self> {
        Self::write(&directory()?, contents, false)
    }
    pub(crate) fn remmina(contents: &str) -> Result<Self> {
        Self::write(&directory()?, contents, true)
    }
    fn write(directory: &Path, contents: &str, remmina: bool) -> Result<Self> {
        fs::create_dir_all(&directory).context("could not create RDP cache directory")?;
        let path = directory.join(format!(
            "{}.{}",
            uuid::Uuid::new_v4(),
            if remmina { "remmina" } else { "rdp" }
        ));
        let bytes = if remmina {
            contents.as_bytes().to_vec()
        } else {
            let mut bytes = vec![0xff, 0xfe];
            bytes.extend(contents.encode_utf16().flat_map(u16::to_le_bytes));
            bytes
        };
        miaominal_paths::atomic_write_user_only(&path, bytes)?;
        Ok(Self { path })
    }
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}
impl Drop for ConnectionFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn cleanup_stale_files() -> Result<()> {
    cleanup_directory(&directory()?)
}

fn cleanup_directory(directory: &Path) -> Result<()> {
    if !directory.exists() {
        return Ok(());
    }
    // One unreadable or locked entry must not keep the remaining leftovers on disk.
    let mut first_error = None;
    for entry in fs::read_dir(directory)? {
        if let Err(error) = cleanup_entry(entry) {
            first_error.get_or_insert(error);
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn cleanup_entry(entry: std::io::Result<fs::DirEntry>) -> Result<()> {
    let entry = entry?;
    let path = entry.path();
    if !matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("rdp" | "remmina")
    ) || path
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .is_none()
        || !entry.file_type()?.is_file()
    {
        return Ok(());
    }
    if entry
        .metadata()?
        .modified()?
        .elapsed()
        .unwrap_or(Duration::ZERO)
        >= FILE_TTL
    {
        fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_preserves_username_and_secure_defaults() {
        let endpoint = RdpEndpoint::new("[2001:db8::1]", 3389).unwrap();
        let file = render_connection_file(&endpoint, "域\\用户", true).unwrap();
        assert!(file.starts_with("full address:s:[2001:db8::1]:3389\r\nusername:s:域\\用户\r\n"));
        assert!(file.contains("authentication level:i:2\r\n"));
        assert!(file.contains("redirectdrives:i:0\r\n"));
        assert!(!file.contains("password"));
        assert!(render_connection_file(&endpoint, "user\r\nredirectdrives:i:1", true).is_err());
        assert!(RdpEndpoint::new("host\nother", 3389).is_err());
        assert!(RdpEndpoint::new("host", 0).is_err());
        assert!(RdpEndpoint::new("[]", 3389).is_err());
        assert!(RdpEndpoint::new("[::1", 3389).is_err());
    }
    #[test]
    fn native_files_use_correct_encoding_and_are_deleted_on_drop() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = RdpEndpoint::new("::1", 3389).unwrap();
        let contents = render_remmina_file(&endpoint, "域\\用 户").unwrap();
        assert!(contents.contains("username=用\\s户\ndomain=域\n"));
        let upn = render_remmina_file(&endpoint, "user@corp.example").unwrap();
        assert!(upn.contains("username=user@corp.example\ndomain=\n"));
        let odd = render_remmina_file(&endpoint, "\\user").unwrap();
        assert!(odd.contains("username=\\\\user\ndomain=\n"));
        assert!(contents.contains("cert_ignore=0\n"));
        assert!(render_remmina_file(&endpoint, "user\npassword=x").is_err());
        let native = ConnectionFile::write(directory.path(), &contents, true).unwrap();
        assert_eq!(fs::read_to_string(native.path()).unwrap(), contents);
        let path = native.path().to_owned();
        drop(native);
        assert!(!path.exists());
        let file = ConnectionFile::write(directory.path(), "username:s:用户\r\n", false).unwrap();
        assert!(fs::read(file.path()).unwrap().starts_with(&[0xff, 0xfe]));
    }

    #[test]
    fn expiry_cleanup_preserves_recent_and_unrecognized_files() {
        let directory = tempfile::tempdir().unwrap();
        let expired = ConnectionFile::write(directory.path(), "expired", false).unwrap();
        let recent = ConnectionFile::write(directory.path(), "recent", true).unwrap();
        let unrelated = directory.path().join("user-connection.rdp");
        fs::write(&unrelated, "user file").unwrap();
        let timestamp = std::time::SystemTime::now() - FILE_TTL - Duration::from_secs(1);
        for path in [expired.path(), unrelated.as_path()] {
            fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(timestamp))
                .unwrap();
        }
        cleanup_directory(directory.path()).unwrap();
        assert!(!expired.path().exists());
        assert!(recent.path().exists());
        assert!(unrelated.exists());
    }
}
