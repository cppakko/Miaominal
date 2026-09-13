//! Local terminal sessions.
//!
//! A local terminal profile spawns the user's shell in a platform PTY and speaks
//! the same session protocol as an SSH session, so the workspace reuses the
//! existing terminal rendering, split panes, search, clipboard and reopen paths.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use crate::ui::i18n;
use miaominal_core::profile::SessionProfile;
use miaominal_core::terminal::MIN_TERMINAL_COLUMNS;
use miaominal_ssh::{SessionChannels, SessionCommand, SessionConnection, SessionEvent};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tokio::sync::mpsc::{Sender, UnboundedReceiver};

/// A shell program resolved for a local terminal profile.
pub(in crate::ui::shell) struct LocalShell {
    pub(in crate::ui::shell) program: String,
    pub(in crate::ui::shell) args: Vec<String>,
    /// Short label used for tab titles and status messages.
    pub(in crate::ui::shell) display: String,
}

/// Resolves the shell for a local terminal profile.
///
/// Local terminals never fall back to a detected shell: the profile has to name
/// the executable, and `None` means the profile does not specify one.
pub(in crate::ui::shell) fn resolve_local_shell(profile: &SessionProfile) -> Option<LocalShell> {
    let program = profile.local_shell.trim();
    if program.is_empty() {
        return None;
    }
    let program = resolve_program(program);
    Some(LocalShell {
        display: shell_display(&program),
        args: split_arguments(&profile.local_shell_args),
        program,
    })
}

/// Whether the configured shell can be located on this machine.
pub(in crate::ui::shell) fn local_shell_is_available(program: &str) -> bool {
    let program = program.trim();
    !program.is_empty() && find_program(program).is_some()
}

/// Whether the configured working directory can be used. An empty value means
/// the shell decides its own starting directory.
pub(in crate::ui::shell) fn local_working_directory_is_available(directory: &str) -> bool {
    let directory = directory.trim();
    directory.is_empty() || Path::new(directory).is_dir()
}

/// Placeholder for the required shell executable path field.
pub(in crate::ui::shell) fn local_shell_placeholder() -> String {
    if cfg!(windows) {
        i18n::string("placeholders.host_editor.local_shell_windows")
    } else {
        i18n::string("placeholders.host_editor.local_shell_posix")
    }
}

/// Label used when a local terminal profile has no user supplied name.
pub(in crate::ui::shell) fn local_terminal_label(shell: &str) -> String {
    let shell = shell.trim();
    if shell.is_empty() {
        i18n::string("tabs.initial.local_terminal_title")
    } else {
        shell_display(shell)
    }
}

/// Startup commands are typed into the shell once it is running. CMD and
/// PowerShell PTYs treat `\r` as Enter, POSIX shells treat `\n` as Enter.
fn startup_command_input(program: &str, startup_command: &str) -> Vec<u8> {
    let command = startup_command.trim();
    if command.is_empty() {
        return Vec::new();
    }
    let line_ending = if shell_program_uses_carriage_return(program) {
        "\r"
    } else {
        "\n"
    };
    format!("{command}{line_ending}").into_bytes()
}

fn shell_program_uses_carriage_return(program: &str) -> bool {
    let name = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(name.as_str());
    matches!(name, "cmd" | "powershell" | "pwsh")
}

fn shell_display(program: &str) -> String {
    Path::new(program)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or(program)
        .to_string()
}

/// Splits a user supplied argument string with the POSIX shell rules used on
/// this platform, so quoted arguments and escaped spaces survive intact.
#[cfg(not(windows))]
fn split_arguments(arguments: &str) -> Vec<String> {
    #[derive(Clone, Copy)]
    enum Quote {
        None,
        Single,
        Double,
    }

    let mut parsed = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quote = Quote::None;
    let mut characters = arguments.chars().peekable();
    while let Some(character) = characters.next() {
        match quote {
            Quote::None => match character {
                '\'' => {
                    quote = Quote::Single;
                    started = true;
                }
                '"' => {
                    quote = Quote::Double;
                    started = true;
                }
                '\\' => {
                    if let Some(escaped) = characters.next() {
                        current.push(escaped);
                    } else {
                        current.push('\\');
                    }
                    started = true;
                }
                character if character.is_whitespace() => {
                    if started {
                        parsed.push(std::mem::take(&mut current));
                        started = false;
                    }
                }
                character => {
                    current.push(character);
                    started = true;
                }
            },
            Quote::Single => {
                if character == '\'' {
                    quote = Quote::None;
                } else {
                    current.push(character);
                }
            }
            Quote::Double => match character {
                '"' => quote = Quote::None,
                '\\' => match characters.peek().copied() {
                    Some(escaped @ ('"' | '\\' | '$' | '`')) => {
                        characters.next();
                        current.push(escaped);
                    }
                    _ => current.push('\\'),
                },
                character => current.push(character),
            },
        }
    }
    if started {
        parsed.push(current);
    }
    parsed
}

/// Splits a user supplied argument string with the Windows command line rules
/// used by `CommandLineToArgvW`, so quoted arguments and backslashes survive
/// intact.
#[cfg(windows)]
fn split_arguments(arguments: &str) -> Vec<String> {
    let mut parsed = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut in_quotes = false;
    let mut backslashes = 0usize;
    for character in arguments.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                for _ in 0..backslashes / 2 {
                    current.push('\\');
                }
                if backslashes.is_multiple_of(2) {
                    in_quotes = !in_quotes;
                } else {
                    current.push('"');
                }
                backslashes = 0;
                started = true;
            }
            character if character.is_ascii_whitespace() && !in_quotes => {
                for _ in 0..backslashes {
                    current.push('\\');
                }
                backslashes = 0;
                if started {
                    parsed.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            character => {
                for _ in 0..backslashes {
                    current.push('\\');
                }
                backslashes = 0;
                current.push(character);
                started = true;
            }
        }
    }
    for _ in 0..backslashes {
        current.push('\\');
    }
    if started {
        parsed.push(current);
    }
    parsed
}

fn resolve_program(program: &str) -> String {
    find_program(program)
        .map(|found| found.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.to_string())
}

/// Locates a shell executable. A path has to point at an executable file; a
/// bare name is resolved through `PATH`, including `PATHEXT` on Windows.
fn find_program(program: &str) -> Option<PathBuf> {
    let program = program.trim();
    let is_path = program.contains('/') || program.contains('\\');
    if is_path && is_executable_file(Path::new(program)) {
        return Some(PathBuf::from(program));
    }
    let mut candidates = if is_path {
        Vec::new()
    } else {
        vec![PathBuf::from(program)]
    };
    if Path::new(program).extension().is_none() {
        candidates.extend(path_candidates_with_extensions(program));
    }
    candidates
        .into_iter()
        .find_map(|candidate| find_in_path(&candidate))
}

fn find_in_path(program: &Path) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(program))
        .find(|candidate| is_executable_file(candidate))
}

/// Whether `path` points at a file the platform can execute directly.
#[cfg(not(windows))]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.is_file()
        && path
            .metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

/// Whether `path` points at a file the platform can execute directly. Windows
/// executable resolution is extension based, so the file needs an extension
/// listed in `PATHEXT`.
#[cfg(windows)]
fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };
    let extension = format!(".{}", extension.to_ascii_lowercase());
    path_extensions()
        .iter()
        .any(|known_extension| known_extension == &extension)
}

#[cfg(windows)]
fn path_candidates_with_extensions(program: &str) -> Vec<PathBuf> {
    path_extensions()
        .into_iter()
        .map(|extension| PathBuf::from(format!("{program}{extension}")))
        .collect()
}

#[cfg(not(windows))]
fn path_candidates_with_extensions(_program: &str) -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn path_extensions() -> Vec<String> {
    let raw = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    let extensions: Vec<String> = raw
        .split(';')
        .map(str::trim)
        .filter(|extension| !extension.is_empty())
        .map(|extension| extension.to_ascii_lowercase())
        .collect();
    if extensions.is_empty() {
        vec![".exe".to_string()]
    } else {
        extensions
    }
}

/// Starts a local shell session and returns the session connection handed to
/// the workspace, matching `TerminalService::start_session`.
pub(in crate::ui::shell) fn start_local_session(
    profile: SessionProfile,
    columns: usize,
    lines: usize,
) -> SessionConnection {
    let SessionChannels {
        connection,
        commands,
        events,
    } = miaominal_ssh::session_channels();
    let thread_name = format!("local-session-{}", profile.id);
    std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || run_local_session(profile, columns, lines, commands, events))
        .expect("failed to spawn local session thread");
    connection
}

fn run_local_session(
    profile: SessionProfile,
    columns: usize,
    lines: usize,
    mut command_receiver: UnboundedReceiver<SessionCommand>,
    event_sender: Sender<SessionEvent>,
) {
    let Some(shell) = resolve_local_shell(&profile) else {
        fail_session(
            &event_sender,
            i18n::string("errors.profile.validation.local_shell_required"),
        );
        return;
    };
    let size = PtySize {
        rows: lines.max(1).min(u16::MAX as usize) as u16,
        cols: columns.max(MIN_TERMINAL_COLUMNS).min(u16::MAX as usize) as u16,
        pixel_width: 0,
        pixel_height: 0,
    };
    let pty_system = native_pty_system();
    let pair = match pty_system.openpty(size) {
        Ok(pair) => pair,
        Err(error) => {
            fail_session(&event_sender, error);
            return;
        }
    };

    let mut command = CommandBuilder::new(&shell.program);
    for argument in &shell.args {
        command.arg(argument);
    }
    let working_directory = profile.local_working_directory.trim();
    if !working_directory.is_empty() {
        command.cwd(working_directory);
    }
    for variable in &profile.environment_variables {
        let name = variable.name.trim();
        if !name.is_empty() {
            command.env(name, variable.value.as_str());
        }
    }
    command.env("TERM", "xterm-256color");

    let mut child = match pair.slave.spawn_command(command) {
        Ok(child) => child,
        Err(error) => {
            fail_session(&event_sender, error);
            return;
        }
    };
    drop(pair.slave);

    let reader = match pair.master.try_clone_reader() {
        Ok(reader) => reader,
        Err(error) => {
            let _ = child.kill();
            fail_session(&event_sender, error);
            return;
        }
    };
    let mut writer = match pair.master.take_writer() {
        Ok(writer) => writer,
        Err(error) => {
            let _ = child.kill();
            fail_session(&event_sender, error);
            return;
        }
    };
    let master = pair.master;

    let startup_input = startup_command_input(&shell.program, &profile.startup_command);
    if !startup_input.is_empty() {
        let _ = writer.write_all(&startup_input);
        let _ = writer.flush();
    }

    let _ = event_sender.blocking_send(SessionEvent::Connected(shell.display.clone()));

    let (finished_sender, mut finished_receiver) =
        tokio::sync::oneshot::channel::<Option<std::io::Error>>();
    let reader_events = event_sender.clone();
    let reader_thread = match std::thread::Builder::new()
        .name(format!("local-session-reader-{}", profile.id))
        .spawn(move || {
            let mut reader = reader;
            let mut buffer = [0u8; 8192];
            let mut read_error = None;
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        if reader_events
                            .blocking_send(SessionEvent::Output(buffer[..read].to_vec()))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        read_error = Some(error);
                        break;
                    }
                }
            }
            let _ = finished_sender.send(read_error);
        }) {
        Ok(thread) => thread,
        Err(_) => {
            let _ = child.kill();
            fail_session(&event_sender, "failed to spawn local session reader thread");
            return;
        }
    };

    let mut killer = child.clone_killer();
    let (exited_sender, mut exited_receiver) =
        tokio::sync::oneshot::channel::<portable_pty::ExitStatus>();
    let child_waiter = match std::thread::Builder::new()
        .name(format!("local-session-waiter-{}", profile.id))
        .spawn(move || {
            if let Ok(status) = child.wait() {
                let _ = exited_sender.send(status);
            }
        }) {
        Ok(thread) => thread,
        Err(_) => {
            let _ = killer.kill();
            fail_session(&event_sender, "failed to spawn local session waiter thread");
            return;
        }
    };

    enum LocalSessionEnd {
        ReaderEof,
        ReaderError(std::io::Error),
        ChildExited(Option<portable_pty::ExitStatus>),
        Abandoned,
    }

    let end = futures::executor::block_on(async {
        loop {
            tokio::select! {
                result = &mut finished_receiver => {
                    break match result {
                        Ok(Some(error)) => LocalSessionEnd::ReaderError(error),
                        Ok(None) | Err(_) => LocalSessionEnd::ReaderEof,
                    };
                }
                status = &mut exited_receiver => {
                    break LocalSessionEnd::ChildExited(status.ok());
                }
                command = command_receiver.recv() => match command {
                    None | Some(SessionCommand::Close) => break LocalSessionEnd::Abandoned,
                    Some(SessionCommand::Send(bytes)) => {
                        if writer.write_all(&bytes).is_err() {
                            break LocalSessionEnd::Abandoned;
                        }
                        let _ = writer.flush();
                    }
                    Some(SessionCommand::Resize { columns, lines }) => {
                        let _ = master.resize(PtySize {
                            rows: lines.max(1).min(u16::MAX as usize) as u16,
                            cols: columns
                                .max(MIN_TERMINAL_COLUMNS)
                                .min(u16::MAX as usize) as u16,
                            pixel_width: 0,
                            pixel_height: 0,
                        });
                    }
                    Some(_) => {}
                }
            }
        }
    });

    if !matches!(end, LocalSessionEnd::ChildExited(_)) {
        let _ = killer.kill();
    }
    drop(writer);
    drop(master);
    let _ = reader_thread.join();
    let _ = child_waiter.join();
    let exit_status = match &end {
        LocalSessionEnd::ChildExited(status) => status.clone(),
        _ => exited_receiver.try_recv().ok(),
    };

    match end {
        LocalSessionEnd::Abandoned => {}
        LocalSessionEnd::ReaderError(error) => {
            let _ = event_sender.blocking_send(SessionEvent::Error(error.to_string()));
        }
        LocalSessionEnd::ReaderEof | LocalSessionEnd::ChildExited(_) => {
            if let Some(status) = exit_status {
                let _ = event_sender.blocking_send(SessionEvent::Exited {
                    exit_code: status.exit_code(),
                    signal: status.signal().map(str::to_owned),
                });
            }
        }
    }
    let _ = event_sender.blocking_send(SessionEvent::Closed);
}

fn fail_session(event_sender: &Sender<SessionEvent>, error: impl std::fmt::Display) {
    let _ = event_sender.blocking_send(SessionEvent::Error(error.to_string()));
    let _ = event_sender.blocking_send(SessionEvent::Closed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_shells_receive_carriage_return_startup_commands() {
        assert_eq!(
            startup_command_input(r"C:\Windows\System32\cmd.exe", "dir"),
            b"dir\r"
        );
        assert_eq!(
            startup_command_input("pwsh.exe", "Get-Location"),
            b"Get-Location\r"
        );
    }

    #[test]
    fn posix_shells_receive_line_feed_startup_commands() {
        assert_eq!(
            startup_command_input("/bin/zsh", "  echo ready  "),
            b"echo ready\n"
        );
    }

    #[test]
    fn blank_startup_commands_produce_no_input() {
        assert!(startup_command_input("cmd.exe", "   ").is_empty());
    }

    #[test]
    fn local_shells_are_never_detected_automatically() {
        let mut profile = SessionProfile::blank_local("local-a", 1);
        assert!(resolve_local_shell(&profile).is_none());

        profile.local_shell = "   ".into();
        assert!(resolve_local_shell(&profile).is_none());
    }

    #[test]
    fn configured_shells_resolve_to_the_given_executable() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let executable = directory.path().join(if cfg!(windows) {
            "miaominal-test-shell.exe"
        } else {
            "miaominal-test-shell"
        });
        std::fs::write(&executable, b"").expect("test shell should be created");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = std::fs::metadata(&executable)
                .expect("test shell metadata should be read")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&executable, permissions)
                .expect("test shell should be made executable");
        }

        let mut profile = SessionProfile::blank_local("local-a", 1);
        profile.local_shell = executable.display().to_string();
        profile.local_shell_args = "-NoLogo  --fast".into();

        let shell = resolve_local_shell(&profile).expect("configured shell should resolve");

        assert_eq!(shell.program, executable.display().to_string());
        assert_eq!(
            shell.args,
            vec!["-NoLogo".to_string(), "--fast".to_string()]
        );
        assert_eq!(shell.display, "miaominal-test-shell");
    }

    #[test]
    fn regular_files_are_not_available_shells() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let regular_file =
            directory
                .path()
                .join(if cfg!(windows) { "shell.toml" } else { "shell" });
        std::fs::write(&regular_file, b"").expect("regular file should be created");

        assert!(!local_shell_is_available(
            &regular_file.display().to_string()
        ));
    }

    #[test]
    fn working_directories_have_to_exist_and_be_directories() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let regular_file = directory.path().join("not-a-directory");
        std::fs::write(&regular_file, b"").expect("regular file should be created");

        assert!(local_working_directory_is_available(""));
        assert!(local_working_directory_is_available("   "));
        assert!(local_working_directory_is_available(
            &directory.path().display().to_string()
        ));
        assert!(!local_working_directory_is_available(
            &regular_file.display().to_string()
        ));
        assert!(!local_working_directory_is_available(
            "miaominal-missing-directory-8f2c1d"
        ));
    }

    #[test]
    fn missing_shell_locations_are_reported_as_unavailable() {
        let missing = std::env::temp_dir().join("miaominal-missing-shell-8f2c1d");
        let existing = std::env::current_exe().expect("test executable path should be available");

        assert!(!local_shell_is_available(""));
        assert!(!local_shell_is_available(&missing.display().to_string()));
        assert!(!local_shell_is_available("miaominal-missing-shell-8f2c1d"));
        assert!(local_shell_is_available(&existing.display().to_string()));
    }

    #[cfg(windows)]
    #[test]
    fn windows_arguments_follow_command_line_rules() {
        assert_eq!(
            split_arguments(r#"-NoLogo --title "My Shell" --fast"#),
            vec!["-NoLogo", "--title", "My Shell", "--fast"]
        );
        assert_eq!(
            split_arguments(r#"--path "C:\Program Files\Git\bin""#),
            vec!["--path", r"C:\Program Files\Git\bin"]
        );
        assert_eq!(
            split_arguments(r#"--path "C:\Program Files\\""#),
            vec!["--path", r"C:\Program Files\"]
        );
        assert_eq!(
            split_arguments(r#"--literal \"quoted\" tail"#),
            vec!["--literal", r#""quoted""#, "tail"]
        );
        assert_eq!(split_arguments(r#""""#), vec![String::new()]);
        assert!(split_arguments("   ").is_empty());
    }

    #[cfg(not(windows))]
    #[test]
    fn posix_arguments_follow_shell_rules() {
        assert_eq!(
            split_arguments(r#"-NoLogo --title "My Shell" --fast"#),
            vec!["-NoLogo", "--title", "My Shell", "--fast"]
        );
        assert_eq!(
            split_arguments(r#"--path '/home/me/My Files' --flag"#),
            vec!["--path", "/home/me/My Files", "--flag"]
        );
        assert_eq!(
            split_arguments(r#"--literal \"quoted\" --fast"#),
            vec!["--literal", r#""quoted""#, "--fast"]
        );
        assert_eq!(
            split_arguments(r#"--escaped\ space plain"#),
            vec!["--escaped space", "plain"]
        );
        assert_eq!(split_arguments(r#""""#), vec![String::new()]);
        assert!(split_arguments("   ").is_empty());
    }

    #[test]
    fn local_terminal_labels_use_the_configured_shell() {
        assert_eq!(local_terminal_label("/bin/zsh"), "zsh");
        assert_eq!(local_terminal_label("/usr/local/bin/fish"), "fish");
        #[cfg(windows)]
        assert_eq!(
            local_terminal_label(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            "pwsh"
        );
        assert_eq!(
            local_terminal_label("   "),
            i18n::string("tabs.initial.local_terminal_title")
        );
    }
}

#[cfg(all(test, windows))]
mod local_pty_integration {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn windows_console_output_reaches_the_terminal_once_write_back_is_answered() {
        let program =
            std::env::var("COMSPEC").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".to_string());
        let mut profile = SessionProfile::blank_local("local-pty-integration", 1);
        profile.local_shell = program;
        let mut connection = start_local_session(profile, 80, 24);
        let terminal = miaominal_terminal::TerminalState::new(80, 24);

        let marker = "miaominal-local-pty";
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut raw = Vec::new();
        let mut sent_command = false;
        while Instant::now() < deadline {
            while let Ok(event) = connection.events.try_recv() {
                match event {
                    SessionEvent::Output(chunk) => {
                        terminal.push_bytes(&chunk);
                        raw.extend_from_slice(&chunk);
                    }
                    SessionEvent::Error(error) => panic!("local session failed: {error}"),
                    _ => {}
                }
            }

            let mut answered_write_back = false;
            while let Some(event) = terminal.try_recv_event() {
                if let miaominal_terminal::TerminalEvent::PtyWrite(sequence) = event {
                    connection
                        .commands
                        .send_bytes(sequence.into_bytes())
                        .expect("write-back should reach the local session");
                    answered_write_back = true;
                }
            }

            if answered_write_back && !sent_command {
                sent_command = true;
                connection
                    .commands
                    .send_bytes(format!("echo {marker}\r").into_bytes())
                    .expect("input should reach the local session");
            }

            if String::from_utf8_lossy(&raw).contains(marker) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        panic!(
            "local terminal produced no console output: {:?}",
            String::from_utf8_lossy(&raw)
        );
    }
}

#[cfg(test)]
mod local_exit_integration {
    use super::*;
    use std::time::{Duration, Instant};

    fn immediate_exit_profile() -> SessionProfile {
        let mut profile = SessionProfile::blank_local("local-exit-integration", 1);
        if cfg!(windows) {
            profile.local_shell = std::env::var("COMSPEC")
                .unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".to_string());
            profile.local_shell_args = "/c exit 7".into();
        } else {
            profile.local_shell = "/bin/sh".into();
            profile.local_shell_args = "-c \"exit 7\"".into();
        }
        profile
    }

    #[test]
    fn local_process_exit_reports_its_status_before_closing() {
        let mut connection = start_local_session(immediate_exit_profile(), 80, 24);
        let terminal = miaominal_terminal::TerminalState::new(80, 24);
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut exited = None;
        let mut closed = false;
        while Instant::now() < deadline && !closed {
            while let Ok(event) = connection.events.try_recv() {
                match event {
                    SessionEvent::Output(chunk) => terminal.push_bytes(&chunk),
                    SessionEvent::Exited { exit_code, signal } => {
                        exited = Some((exit_code, signal));
                    }
                    SessionEvent::Error(error) => panic!("local session failed: {error}"),
                    SessionEvent::Closed => closed = true,
                    _ => {}
                }
            }
            while let Some(event) = terminal.try_recv_event() {
                if let miaominal_terminal::TerminalEvent::PtyWrite(sequence) = event {
                    connection
                        .commands
                        .send_bytes(sequence.into_bytes())
                        .expect("write-back should reach the local session");
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        assert!(closed, "local session did not close");
        let (exit_code, signal) = exited.expect("exit status should be reported");
        assert_eq!(exit_code, 7);
        assert!(signal.is_none());
    }
}
