<div align="center" style="border-bottom: none">
    <h1>
        Miaominal
        <br><br>
        <img src="assets/hero.png" width="760" alt="Miaominal logo" />
    </h1>
    <a href="https://github.com/cppakko/miaominal/releases"><img alt="Latest release" src="https://img.shields.io/github/v/release/cppakko/miaominal?include_prereleases&color=2ea44f"></a>
    <a href="https://github.com/cppakko/miaominal/actions/workflows/release.yml"><img alt="Release workflow" src="https://img.shields.io/github/actions/workflow/status/cppakko/miaominal/release.yml?label=release"></a>
    <a href="https://www.rust-lang.org/"><img alt="Rust 2024" src="https://img.shields.io/badge/Rust-2024-dea584?logo=rust&logoColor=white"></a>
    <a href="../LICENSE"><img alt="License MIT" src="https://img.shields.io/badge/license-MIT-blue"></a>
    <br>
    <p align="center">
        Miaominal is a desktop SSH workspace for remote development and server operations. Manage hosts, terminal sessions, SFTP transfers, port forwarding, and session agents in one workspace, then expose saved hosts to external clients through OpenSSH integration. Built with Rust, GPUI Kit, and <code>alacritty_terminal</code>.
    </p>
</div>

<p align="center"><a href="./README.md">English</a> · <a href="./README_zh.md">简体中文</a></p>

<p align="center"><a href="#features">Features</a> · <a href="#installation">Installation</a> · <a href="#core-workflows">Core Workflows</a> · <a href="#credentials-and-sync">Credentials and Sync</a> · <a href="#data-directories-and-portable-mode">Portable Mode</a> · <a href="#building-from-source">Building from Source</a></p>

## Features

- **SSH host management:** Save connection profiles, authentication methods, startup commands, environment variables, tags, groups, and reusable SOCKS5 / HTTP CONNECT entry proxies.
- **Configuration import:** Import SSH profiles from OpenSSH config, PuTTY `.reg`, SecureCRT `.xml`, and FinalShell `.json`.
- **Terminal workspace:** Tabs, split panes, scrollback search, copy/paste, recently closed tab recovery, detached windows, and Free Type mode.
- **SFTP workspace:** Browse local and remote files, upload and download, drag to select, save remote path favorites, confirm overwrites and deletes, create folders, and pause / resume / cancel transfers.
- **Port forwarding:** Manage local and remote forwarding rules associated with saved SSH hosts.
- **Remote monitoring:** Collect CPU, memory, Swap, disk, network, and load metrics after an SSH session is ready.
- **Snippets:** Save reusable commands and shell scripts for quick use during daily sessions.
- **Credentials and trust:** Manage known hosts, system credential storage, a local vault, managed private keys, and SSH agent identities.
- **OpenSSH integration:** Generate OpenSSH configuration for saved hosts; Direct exports ordinary host entries, while Bridge connects through Miaominal with security approval and audit logging.
- **Encrypted sync:** Sync configuration through GitHub Gist or WebDAV manually or automatically. Sensitive fields are encrypted with an Argon2id-derived key and AES-256-GCM before upload, with notifications for conflicts.
- **Data directories and portable mode:** Choose a custom data directory, migrate data safely, or use a Portable package with a local vault for credentials.
- **Desktop experience:** System tray support, configurable close behavior, single-instance activation, notification history, and independent interface / terminal font settings.
- **Session Agent:** Chat history, title generation, attachments, Markdown rendering, tool-call status, background jobs, approval modes, and interruption recovery.

<div align="center">
    <img src="assets/second.png" width="760" />
</div>


## Installation

Choose the package for your operating system and processor architecture from [Releases](https://github.com/cppakko/miaominal/releases/latest). `<version>` below means the release version, such as `0.2.3`; use the complete filename shown on the release page.

| Platform | Package | Notes |
| --- | --- | --- |
| macOS arm64 | `Miaominal-macos-arm64-<version>.dmg` | Drag the app into Applications |
| Windows x64 / arm64 | `Miaominal-windows-<arch>-<version>-setup.exe` | Installer; a standalone executable is also available |
| Linux x86_64 / arm64 | `.AppImage`, `.deb`, `.rpm` | Choose the format for your distribution |
| All three platforms | `*-<version>-portable.zip` | Extract and run, with data in the included `data` directory |

### macOS

1. Download `Miaominal-macos-arm64-<version>.dmg`.
2. Open the `.dmg` and drag `Miaominal.app` into `Applications`.

> [!WARNING]
> Miaominal is not notarized. If macOS blocks the app, remove the quarantine attribute:
> ~~~ bash
> xattr -dr com.apple.quarantine /Applications/Miaominal.app
> ~~~

### Windows

1. Download the package for your architecture ending in `-setup.exe`.
2. Run the installer and follow the prompts.

A file ending in `-standalone.exe` runs without installation and still uses the standard data directory by default. Choose `-portable.zip` to carry your data with the program.

### Linux

1. Download the `.AppImage` for your architecture, or install a `.deb` / `.rpm` package with your distribution's package manager.
2. For AppImage, make it executable and run it. This example uses x86_64 version `0.2.3`; replace the filename with the one you downloaded:

```bash
chmod +x Miaominal-linux-x86_64-0.2.3.AppImage
./Miaominal-linux-x86_64-0.2.3.AppImage
```

## Core Workflows

### Hosts and Terminal Sessions

Manage SSH hosts, recent connections, tags, groups, and authentication methods in one place, then open terminal tabs, split panes, or move tabs into detached windows. Multiple hosts can reuse the same SOCKS5 / HTTP CONNECT entry proxy.

Enable Free Type mode in settings to click to move the terminal cursor, select editable text, and drag text within or between terminal panes. Hold Ctrl/Cmd while dragging to copy. When a terminal application enables mouse reporting, hold Alt to use Free Type temporarily; Shift uses traditional selection and Shift+Alt uses block selection.

Interface and terminal fonts and sizes can be configured independently. Closing the main window can quit the app or minimize it to the system tray; launching with the same data directory activates the existing instance.

The notification center keeps a history of operation notifications and provides settings shortcuts for items that need attention, such as sync conflicts.

<p align="center">
    <img src="./assets/terminal.png" width="760" style="border-radius: 10px;" alt="Hosts and terminal session screenshot" />
    <br>
</p>

### SFTP File Transfer

Use the local / remote dual-pane file browser in the terminal side panel to handle uploads, downloads, directory creation, remote path favorites, overwrite confirmation, delete confirmation, and transfer progress. Favorite remote paths can be synced with your configuration.

<p align="center">
    <img src="./assets/sftp.png" width="760" style="border-radius: 10px;" alt="SFTP side panel screenshot" />
    <br>
</p>

### Session Agent

Open an Agent panel next to the current session and configure a model provider to ask questions, read files, run commands, apply patches, search the web, or fetch web content. The selected mode determines which tools are available and how approval works.

| Capability | Description |
| --- | --- |
| Current session | Read workspace information, understand the active terminal context, and hand short commands or long-running tasks to the corresponding shell. |
| SSH hosts | Mention opened or saved hosts with `@` to target file reads, search, command execution, and patch application at a specific remote machine. |
| Workspace files | Use `read`, `list`, `glob`, and `grep` to inspect files, then create, modify, or delete files with `apply_patch`. |
| Background tasks | Use background jobs for long-running server, log, test, deployment, and similar tasks; continue checking status, stopping tasks, or collecting results in the session. |
| Web lookup | Use configured Web Search / Fetch to collect web information and analyze it alongside terminal, file, and attachment context. |

| Execution mode | Best for | Tool and approval behavior |
| --- | --- | --- |
| **Ask** | Understanding a project, searching files, or answering questions only. | Only read-only tools, `web_search` / `web_fetch`, and user questions are available. It does not run commands or modify files. |
| **Execute** | The default mode for daily development and operations. | All tools are available. Web search / fetch can run directly, while file edits, non-read-only shell commands, and high-risk operations go through approval or risk checks. |
| **Non-blocking** | Approving the Agent's tool calls individually. | All tools are available, policy checks are bypassed, and tool calls wait for approval before executing. Path normalization is always enforced. |
| **Full Auto** | Explicitly authorizing the Agent to complete a task continuously. | All tools are available, policy enforcement is bypassed entirely, and tool calls execute automatically. Only `..` and `~` path normalization is always enforced. |

<p align="center">
    <img src="./assets/agent.png" width="760" style="border-radius: 10px;" alt="Session Agent panel screenshot" />
    <br>
</p>

### Port Forwarding and Monitoring

Create local or remote forwarding rules for saved hosts, then quickly connect, disconnect, copy, edit, or open the forwarded target in a browser. Forwarding failures are reported through the notification center.

Once an SSH session is ready, the monitoring panel can display remote CPU, memory, Swap, disk, network, and load metrics.

<p align="center">
    <img src="./assets/forward.png" width="760" style="border-radius: 10px;" alt="Port forwarding rules screenshot" />
    <br>
</p>

### OpenSSH Integration

Choose a mode under **Settings → OpenSSH Integration** to use hosts saved in Miaominal from external OpenSSH clients:

| Mode | Connection behavior | Requirements |
| --- | --- | --- |
| Direct | Exports ordinary OpenSSH host entries; the external client connects to the remote host | Cannot directly use passwords saved in Miaominal or Miaominal-specific routes |
| Bridge | Miaominal establishes the remote connection using saved credentials and entry proxies | Miaominal must stay running, and the required credentials must be available |

The settings page shows the generated configuration file path. Find a `Host` alias in that file and use it from an external terminal. For example, if the configuration contains `Host miaominal-production`:

```bash
ssh miaominal-production
```

Bridge does not export passwords or private keys into the generated configuration. Choose standard connections, approval for every connection, or system authentication on supported platforms. The app prompts when approval or vault unlocking is needed. The OpenSSH settings page provides security policies and pending requests, while connection activity is recorded in an audit log.

## Credentials and Sync

Miaominal manages local credential storage separately from cloud sync. Ordinary configuration can be synced, while sensitive credentials included in sync are encrypted with your sync passphrase before upload.

- **Local secure storage:** In standard mode, API keys, SSH passwords, sync credentials, and managed private keys are stored in the system keyring by default. Enabling the local vault migrates those credentials into an encrypted local vault.
- **Local vault:** Protected by a password independent from the sync passphrase, with manual unlocking, locking, and a configurable auto-lock timeout.
- **Sync scope:** GitHub Gist and WebDAV are supported. Sync includes SSH host profiles, entry proxies, snippets, managed keys, remote path favorites, shortcuts, connection preferences, and model provider / web lookup configuration. Device-local options such as interface appearance and whether port forwarding rules are enabled are not synced.
- **Sensitive data encryption:** Passwords, SSH private keys, and model provider / web lookup API keys included in sync are encrypted with an Argon2id-derived key and AES-256-GCM, rather than uploaded as plaintext configuration. GitHub tokens and WebDAV passwords stay in the local credential store.
- **Multi-device recovery:** Bind a new device to the same Gist ID or WebDAV file to pull the remote configuration. Enter the same sync passphrase to decrypt and use the synced sensitive fields.
- **Automatic sync:** Disabled by default; configure a sync provider and encryption passphrase first. Once enabled, local configuration changes are pushed after a short delay, and remote updates are checked periodically. Remote changes can be pulled automatically when there are no unsynced local changes. Conflicts or a missing sync baseline stop automatic operations and prompt manual intervention through the notification center. Automatic sync pauses while the local vault is locked.
- **Managed keys:** Manage private keys, including RSA-4096 keys, in local secure storage for host connections or OpenSSH Bridge.

## Data Directories and Portable Mode

In standard mode, select a custom data directory in settings and migrate your existing data.

For Portable packages, extract the archive and keep the program, `portable.flag`, and `data` directory together. On macOS, the flag and data directory sit beside the `.app` bundle. You can also enable portable mode with the `--portable` startup argument. This mode always uses the portable directory's `data` folder and requires a local vault for sensitive credentials instead of the system keyring.

The local vault password and cloud sync passphrase are independent. After moving the portable directory to another device, use the original vault password to unlock your credentials.

## Building from Source

Install the Rust toolchain and obtain the repository source first, then run the following commands from the repository root.

### macOS Example

```bash
brew install cmake
xcodebuild -downloadComponent MetalToolchain
cargo build --release
```

### Linux Example

Ubuntu 24.04 example:

```bash
sudo apt-get update
sudo apt-get install -y \
  build-essential clang cmake curl git pkg-config \
  libasound2-dev libdbus-1-dev libfontconfig-dev libglib2.0-dev \
  libgtk-3-dev libayatana-appindicator3-dev \
  libgit2-dev libsecret-1-dev libsqlite3-dev libssl-dev libva-dev \
  libvulkan1 libwayland-dev libx11-xcb-dev libxkbcommon-x11-dev \
  libzstd-dev
cargo build --release
```

### Windows Example

```powershell
winget install -e --id Kitware.CMake
choco install nasm --yes --no-progress
cargo build --release
```
