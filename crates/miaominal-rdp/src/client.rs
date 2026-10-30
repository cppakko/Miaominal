use crate::{
    RdpEndpoint,
    credentials::CredentialLease,
    file::{ConnectionFile, render_connection_file, render_remmina_file, validate_value},
};
use anyhow::{Context, Result, bail};
use miaominal_settings::{RdpClientKind, RdpClientSettings};
use std::{
    ffi::{OsStr, OsString},
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RdpClientInfo {
    kind: RdpClientKind,
    path: PathBuf,
}
impl RdpClientInfo {
    pub fn kind(&self) -> RdpClientKind {
        self.kind
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn supports_password(&self) -> bool {
        matches!(self.kind, RdpClientKind::Mstsc | RdpClientKind::SdlFreeRdp)
    }
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(any(target_os = "linux", test))]
fn find_program(name: &str, search_path: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(search_path)
        // Relative or empty entries ("", ".") would resolve against the current
        // directory and let an untrusted folder provide the client.
        .filter(|directory| directory.is_absolute())
        .map(|path| path.join(name))
        .find(|path| is_executable(path))
}

/// SDL FreeRDP binary names, most specific first. Builds with both SDL2 and SDL3
/// install versioned names only (`sdl3-freerdp3`); FreeRDP 3.0–3.9 ship the SDL2
/// client as `sdl-freerdp3`. All accept the same arguments.
#[cfg(any(target_os = "linux", test))]
const SDL_FREERDP_NAMES: [&str; 4] = [
    "sdl3-freerdp3",
    "sdl3-freerdp",
    "sdl-freerdp3",
    "sdl-freerdp",
];

pub fn detect_clients() -> Vec<RdpClientInfo> {
    let mut clients = Vec::new();
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        let path = PathBuf::from(root).join("System32").join("mstsc.exe");
        if is_executable(&path) {
            clients.push(RdpClientInfo {
                kind: RdpClientKind::Mstsc,
                path,
            });
        }
    }
    #[cfg(target_os = "macos")]
    for name in ["Windows App.app", "Microsoft Remote Desktop.app"] {
        let mut roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Applications"));
        }
        for root in roots {
            let path = root.join(name);
            if path.is_dir() {
                clients.push(RdpClientInfo {
                    kind: RdpClientKind::WindowsApp,
                    path,
                });
            }
        }
    }
    #[cfg(target_os = "linux")]
    if let Some(path) = std::env::var_os("PATH") {
        if let Some(path) = find_program("remmina", &path) {
            clients.push(RdpClientInfo {
                kind: RdpClientKind::Remmina,
                path,
            });
        }
        if let Some(path) = SDL_FREERDP_NAMES
            .iter()
            .find_map(|name| find_program(name, &path))
        {
            clients.push(RdpClientInfo {
                kind: RdpClientKind::SdlFreeRdp,
                path,
            });
        }
    }
    clients
}

fn supported(kind: RdpClientKind) -> bool {
    match kind {
        RdpClientKind::Auto => true,
        RdpClientKind::Mstsc => cfg!(windows),
        RdpClientKind::WindowsApp => cfg!(target_os = "macos"),
        RdpClientKind::Remmina | RdpClientKind::SdlFreeRdp => cfg!(target_os = "linux"),
    }
}

pub fn resolve_client(settings: &RdpClientSettings) -> Result<RdpClientInfo> {
    if settings.kind() == RdpClientKind::Auto && !settings.path().is_empty() {
        bail!("select an RDP client type before specifying a client path");
    }
    if !supported(settings.kind()) {
        bail!("the selected RDP client is not supported on this operating system");
    }
    if settings.kind() != RdpClientKind::Auto && !settings.path().is_empty() {
        let path = PathBuf::from(settings.path());
        if !path.is_absolute() {
            bail!("the RDP client path must be absolute");
        }
        let usable = if settings.kind() == RdpClientKind::WindowsApp {
            // Launched through `open -a`, which needs an application bundle.
            path.is_dir() && path.extension() == Some(OsStr::new("app"))
        } else {
            is_executable(&path)
        };
        if !usable {
            bail!("the configured RDP client path is unavailable or is not executable");
        }
        return Ok(RdpClientInfo {
            kind: settings.kind(),
            path,
        });
    }
    detect_clients().into_iter().find(|client| settings.kind() == RdpClientKind::Auto || client.kind == settings.kind())
        .context("no RDP client found; install mstsc on Windows, Windows App on macOS, or Remmina / SDL3 FreeRDP on Linux")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientProcessState {
    Running,
    Exited,
    Untracked,
}

/// How a reused password was handed to the client for this launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordDelivery {
    /// The client asks for the password itself.
    Prompt,
    /// Written to the client's stdin (SDL FreeRDP); the client uses it directly.
    Stdin,
    /// A short-lived Windows credential was stored for mstsc. Windows may still ask,
    /// e.g. when Credential Guard blocks saved credentials.
    WindowsCredential,
}

/// Owns the launched child and its temporary resources, never the external application's lifetime.
pub struct LaunchedClient {
    child: Option<Child>,
    trackable: bool,
    file: Option<ConnectionFile>,
    lease: Option<CredentialLease>,
    started: Instant,
    password_delivery: PasswordDelivery,
}
impl LaunchedClient {
    pub fn password_delivery(&self) -> PasswordDelivery {
        self.password_delivery
    }
    /// State right after launch, without polling the child. Use [`Self::poll`] afterwards.
    pub fn initial_process_state(&self) -> ClientProcessState {
        if self.trackable {
            ClientProcessState::Running
        } else {
            ClientProcessState::Untracked
        }
    }
    pub fn poll(&mut self) -> Result<ClientProcessState> {
        if self.started.elapsed() >= Duration::from_secs(30) {
            self.lease.take();
        }
        let exited = self
            .child
            .as_mut()
            .map(|child| child.try_wait())
            .transpose()?
            .flatten()
            .is_some();
        if exited && self.trackable && self.started.elapsed() >= Duration::from_secs(30)
            || self.started.elapsed() >= Duration::from_secs(600)
        {
            self.file.take();
        }
        Ok(if !self.trackable {
            ClientProcessState::Untracked
        } else if exited {
            ClientProcessState::Exited
        } else {
            ClientProcessState::Running
        })
    }
}
impl Drop for LaunchedClient {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // Do not kill an external desktop application when its tunnel closes.
            // Reap on a detached thread so a long-lived Linux client cannot become a zombie.
            let _ = std::thread::Builder::new()
                .name("rdp-client-reaper".into())
                .spawn(move || {
                    let _ = child.wait();
                });
        }
    }
}

fn arguments(kind: RdpClientKind, file: &Path, from_stdin: bool) -> Vec<OsString> {
    match kind {
        RdpClientKind::Mstsc => vec![file.as_os_str().to_owned()],
        RdpClientKind::Remmina => vec!["-c".into(), file.as_os_str().to_owned()],
        RdpClientKind::SdlFreeRdp => {
            let mut args = vec![file.as_os_str().to_owned()];
            if from_stdin {
                args.push("/from-stdin:force".into());
            }
            args
        }
        _ => Vec::new(),
    }
}

/// Passwords are borrowed for this launch only. No password is placed in argv or connection files.
pub fn launch(
    client: &RdpClientInfo,
    endpoint: &RdpEndpoint,
    username: &str,
    password: Option<&str>,
) -> Result<LaunchedClient> {
    validate_value(username)?;
    if let Some(password) = password {
        validate_value(password)?;
        if password.len() > 4096 {
            bail!("RDP password is too long");
        }
    }
    let lease = if client.kind == RdpClientKind::Mstsc {
        password
            .map(|password| CredentialLease::acquire(endpoint, username, password))
            .transpose()?
            .flatten()
    } else {
        None
    };
    // FreeRDP's forced stdin reader has a fixed 512-byte buffer. Let its GUI ask
    // for longer passwords rather than silently truncating one.
    let from_stdin =
        client.kind == RdpClientKind::SdlFreeRdp && password.is_some_and(|p| p.len() < 512);
    let file = if client.kind == RdpClientKind::Remmina {
        ConnectionFile::remmina(&render_remmina_file(endpoint, username)?)?
    } else {
        ConnectionFile::new(&render_connection_file(
            endpoint,
            username,
            lease.is_none() && !from_stdin,
        )?)?
    };
    let mut command = if client.kind == RdpClientKind::WindowsApp {
        let mut command = Command::new("/usr/bin/open");
        command.arg("-a").arg(&client.path).arg(file.path());
        command
    } else {
        let mut command = Command::new(&client.path);
        command.args(arguments(client.kind, file.path(), from_stdin));
        command
    };
    command
        .stdin(if from_stdin {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().context("could not start the RDP client")?;
    if from_stdin {
        let result = (|| -> std::io::Result<()> {
            let mut input = child
                .stdin
                .take()
                .ok_or_else(|| std::io::Error::other("client stdin unavailable"))?;
            input.write_all(password.unwrap_or_default().as_bytes())?;
            input.write_all(b"\n")
        })();
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
            bail!("could not deliver credentials to the RDP client");
        }
    }
    let password_delivery = if lease.is_some() {
        PasswordDelivery::WindowsCredential
    } else if from_stdin {
        PasswordDelivery::Stdin
    } else {
        PasswordDelivery::Prompt
    };
    Ok(LaunchedClient {
        child: Some(child),
        trackable: matches!(
            client.kind,
            RdpClientKind::Mstsc | RdpClientKind::SdlFreeRdp
        ),
        file: Some(file),
        lease,
        started: Instant::now(),
        password_delivery,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arguments_keep_paths_atomic_and_do_not_contain_passwords() {
        let path = Path::new("/tmp/远程 desktop & $host.rdp");
        assert_eq!(
            arguments(RdpClientKind::Remmina, path, false),
            vec![OsString::from("-c"), path.into()]
        );
        assert_eq!(
            arguments(RdpClientKind::SdlFreeRdp, path, true),
            vec![path.into(), OsString::from("/from-stdin:force")]
        );
        assert_eq!(
            arguments(RdpClientKind::Mstsc, path, false),
            vec![OsString::from(path)]
        );
    }
    #[test]
    fn explicit_missing_path_is_not_silently_replaced() {
        let kind = if cfg!(windows) {
            RdpClientKind::Mstsc
        } else if cfg!(target_os = "macos") {
            RdpClientKind::WindowsApp
        } else {
            RdpClientKind::Remmina
        };
        assert!(resolve_client(&RdpClientSettings::new(kind, "relative-client")).is_err());
        assert!(
            resolve_client(&RdpClientSettings::new(
                kind,
                std::env::temp_dir()
                    .join("missing-rdp-client")
                    .to_string_lossy()
            ))
            .is_err()
        );
    }
    #[test]
    fn path_detection_uses_injected_path_without_mutating_environment() {
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("test-client");
        std::fs::write(&executable, "test").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let path = std::env::join_paths([temp.path()]).unwrap();
        assert_eq!(find_program("test-client", &path), Some(executable));
        assert!(find_program("missing", &path).is_none());
        // Relative PATH entries never resolve against the current directory.
        let relative = std::env::join_paths([Path::new("."), Path::new("relative")]).unwrap();
        assert!(find_program("test-client", &relative).is_none());
    }
    #[test]
    fn sdl_freerdp_names_prefer_sdl3_builds() {
        assert_eq!(SDL_FREERDP_NAMES[0], "sdl3-freerdp3");
        assert!(SDL_FREERDP_NAMES.contains(&"sdl-freerdp"));
    }
}
