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
        Miaominal is a desktop SSH workspace for remote development and server operations. It brings terminal sessions, SSH host management, SFTP file transfer, port forwarding, sensitive credentials, encrypted sync, and session-level agents into one workspace. Built with Rust, GPUI, and `alacritty_terminal`, it aims to keep frequent remote workflows stable, focused, recoverable, and designed for lower memory overhead.
    </p>
</div>

<p align="center"><a href="./README.md">English</a> · <a href="./README_zh.md">简体中文</a></p>

<p align="center"><a href="#features">Features</a> · <a href="#installation">Installation</a> · <a href="#core-feature-screenshots">Core Feature Screenshots</a> · <a href="#encrypted-sync-and-settings">Encrypted Sync and Settings</a> · <a href="#running-from-source">Running from Source</a></p>

## Features

- **SSH host management:** Save connection profiles, authentication methods, startup commands, environment variables, tags, and groups.
- **Configuration import:** Import SSH profiles from OpenSSH config, PuTTY `.reg`, SecureCRT `.xml`, and FinalShell `.json`.
- **Lightweight native stack:** Built around a native Rust desktop stack and terminal backend, designed to reduce memory overhead during daily multi-session SSH workflows.
- **Modern terminal experience:** Powered by `alacritty_terminal`, with tabs, pane splitting, scrollback search, copy/paste, and recently closed tab recovery.
- **SFTP workspace:** Browse local and remote files, upload and download, drag to select, confirm overwrites and deletes, create folders, and pause / resume / cancel transfers.
- **Port forwarding:** Manage local and remote forwarding rules associated with saved SSH hosts.
- **Remote monitoring:** Collect CPU, memory, Swap, disk, network, and load metrics after an SSH session is ready.
- **Snippets:** Save reusable commands and shell recipes for quick use during daily sessions.
- **Credentials and trust:** Manage known hosts, system keychain storage, local vault storage, managed private keys, and SSH agent identities.
- **Encrypted sync:** Sync configuration through GitHub Gist or WebDAV. Sensitive fields are uploaded only after encryption with an Argon2id-derived key and AES-256-GCM.
- **Session Agent:** Chat history, title generation, attachments, Markdown rendering, tool-call status, background jobs, approval modes, and interruption recovery.

<div align="center">
    <img src="assets/second.png" width="760" />
</div>


## Installation

### Windows

1. Download `Miaominal-windows-x64-setup.exe` from [Releases](https://github.com/cppakko/miaominal/releases/latest).
2. Run the installer and follow the prompts.
   - You can also download `Miaominal-windows-x64-standalone.exe` and launch it directly.

### macOS

1. Download `Miaominal-macos-arm64.dmg` from [Releases](https://github.com/cppakko/miaominal/releases/latest).
2. Open the `.dmg` and drag `Miaominal.app` into `Applications`.
3. If macOS blocks the unsigned app, try running:

~~~ bash
spctl --global-disable
xattr -dr com.apple.quarantine /Applications/Miaominal.app
~~~

### Linux

1. Download `Miaominal-linux-x86_64.AppImage` from [Releases](https://github.com/cppakko/miaominal/releases/latest).
2. Make it executable and run it:

```bash
chmod +x Miaominal-linux-x86_64.AppImage
./Miaominal-linux-x86_64.AppImage
```

## Core Feature Screenshots

### Hosts and Terminal Sessions

Manage SSH hosts, recent connections, tags, groups, and authentication methods in one place, then open terminal tabs or split panes in the workspace.

<p align="center">
    <img src="./assets/terminal.png" width="760" style="border-radius: 10px;" alt="Hosts and terminal session screenshot" />
    <br>
</p>

### SFTP File Transfer

Use the local / remote dual-pane file browser in the terminal side panel to handle uploads, downloads, directory creation, overwrite confirmation, delete confirmation, and transfer progress.

<p align="center">
    <img src="./assets/sftp.png" width="760" style="border-radius: 10px;" alt="SFTP side panel screenshot" />
    <br>
</p>

### Session Agent

Open an Agent panel next to the current session and use a configurable provider to ask questions, read files, run commands, apply patches, search the web, or fetch web content. Tool calls are controlled by approval modes.

| Capability | Description |
| --- | --- |
| Current session | Read workspace information, understand the active terminal context, and hand short commands or long-running tasks to the corresponding shell. |
| SSH hosts | Mention opened or saved hosts with `@` to target file reads, search, command execution, and patch application at a specific remote machine. |
| Workspace files | Use `read`, `list`, `glob`, and `grep` to inspect files, then create, modify, or delete files with `apply_patch`. |
| Background tasks | Use background jobs for long-running server, log, test, deployment, and similar tasks; continue checking status, stopping tasks, or collecting results in the session. |
| Web lookup | Use configured Web Search / Fetch to collect web information and analyze it alongside terminal, file, and attachment context. |

| Execution mode | Best for | Tool and approval behavior |
| --- | --- | --- |
| **ASK** | Understanding a project, searching files, or answering questions only. | Only read-only tools, `web_search` / `web_fetch`, and user questions are available. It does not run commands or modify files. |
| **Run** | The default mode for daily development and operations. | All tools are available. Web search / fetch can run directly, while file edits, non-read-only shell commands, and high-risk operations go through approval or risk checks. |
| **Non-blocking** | Letting the Agent plan and list operations first, then approving them one by one. | All tools are available, but tool calls pause for approval except when asking the user a question. Approved calls then continue. |
| **Full Auto** | Explicitly authorizing the Agent to complete a task continuously. | All tools are available and run automatically, while basic path normalization, execution timeouts, and output limits remain in place. |

<p align="center">
    <img src="./assets/agent.png" width="760" style="border-radius: 10px;" alt="Session Agent panel screenshot" />
    <br>
</p>

### Port Forwarding

Create local or remote forwarding rules for saved hosts, then quickly connect, disconnect, copy, edit, or open the forwarded target in a browser.

<p align="center">
    <img src="./assets/forward.png" width="760" style="border-radius: 10px;" alt="Port forwarding rules screenshot" />
    <br>
</p>

### Encrypted Sync and Settings

Miaominal separates local storage from cloud backup: ordinary configuration can be synced, while sensitive credentials stay in the local secure store by default. When you need to sync sensitive data across devices, it is encrypted with your sync passphrase before being uploaded to the remote backend.

- **Local secure storage:** API keys, SSH passwords, sync credentials, and managed private keys are written to the system keyring by default. After enabling the local vault, these secrets are migrated into a local encrypted vault that is unlocked with the vault passphrase.
- **Local vault:** The vault is protected by a local password that is independent from the sync passphrase. It can be manually unlocked and locked, supports an auto-lock timeout, and is designed for sensitive data you want to keep device-local.
- **Cloud sync backends:** GitHub Gist and WebDAV are supported. Miaominal can sync non-appearance configuration such as SSH profiles, snippets, managed keys, shortcuts, connection preferences, and Agent provider / Web Search metadata.
- **End-to-end encryption:** Passwords, SSH private keys, AI provider API keys, and Web Search API keys are encrypted with a key derived from the sync passphrase when they participate in sync. The sync passphrase is processed with Argon2id, and ciphertext is protected with AES-256-GCM.
- **Multi-device recovery:** After binding a new device to the same Gist ID or WebDAV file, you can pull the remote configuration. The synced sensitive fields can be decrypted and used only after entering the same sync passphrase.
- **No plaintext upload:** GitHub tokens and WebDAV passwords are stored only in the local secret store. If AI provider / Web Search API keys participate in sync, they are placed in the encrypted secrets payload instead of being written to the sync file as plaintext configuration.

## Running from Source

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
