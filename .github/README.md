# Miaominal

<p align="center">
	<img src="assets/app-icon.png" alt="Miaominal logo" width="128" />
</p>

<p align="center">
	<a href="https://github.com/cppakko/miaominal/releases/tag/v0.1.0"><img alt="Version 0.1.0" src="https://img.shields.io/badge/version-0.1.0-2ea44f" /></a>
	<a href="https://github.com/cppakko/miaominal/releases"><img alt="Platforms: Linux, Windows, macOS" src="https://img.shields.io/badge/platform-Linux%20%7C%20Windows%20%7C%20macOS-1f6feb" /></a>
	<a href="https://github.com/cppakko/miaominal/actions/workflows/release.yml"><img alt="Release workflow status" src="https://img.shields.io/github/actions/workflow/status/cppakko/miaominal/release.yml?label=release" /></a>
	<a href="https://www.rust-lang.org/"><img alt="Rust 2024" src="https://img.shields.io/badge/Rust-2024-dea584?logo=rust&logoColor=white" /></a>
	<a href="https://opensource.org/licenses/MIT"><img alt="License MIT" src="https://img.shields.io/badge/license-MIT-a31f34" /></a>
</p>

Miaominal is a desktop terminal application built with Rust, GPUI, and alacritty_terminal. It focuses on SSH sessions, SFTP file transfers, configuration management, and secure credential storage.

![Miaominal screenshot](./assets/hero.png)

## Features

- SSH session management with support for authentication, port forwarding, and environment variables
- Import existing SSH profiles from OpenSSH config files, PuTTY registry exports, SecureCRT XML exports, and FinalShell JSON exports
- Built-in dual-pane SFTP browser for local and remote file operations
- Known hosts, system keychain, and local vault support for credential management
- Configuration sync through GitHub Gist or WebDAV

## Development

### Ubuntu

```bash
sudo apt-get install -y libwayland-dev libxkbcommon-dev libegl1-mesa-dev libfontconfig1-dev
sudo apt-get install -y libdbus-1-dev pkg-config
sudo apt-get install -y libxkbcommon-x11-dev
sudo apt-get install -y libxkbcommon-dev

cargo run
```

The first build will fetch git-based dependencies such as GPUI and gpui-component.

## Installation Notes for macOS

1. Download `Miaominal-macos-arm64.dmg` from the release page.
2. Open the DMG and drag `Miaominal.app` into the `Applications` folder.
3. Open `System Settings > Privacy & Security`, find the blocked-app notice near the bottom, and choose `Open Anyway`.
