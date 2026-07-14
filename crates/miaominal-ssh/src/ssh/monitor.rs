use super::session::{ClientHandler, SessionEvent, SessionEventSender};
use anyhow::{Context, Result, bail};
use base64::Engine as _;
use miaominal_core::forwarding::{SessionMonitorPlatform, SessionMonitorSnapshot};
use russh::ChannelMsg;
use russh::client;
use std::borrow::Cow;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;

#[derive(Debug, Clone, Copy)]
struct CpuTotals {
    total: u64,
    idle: u64,
}

#[derive(Debug, Clone, Copy)]
struct NetworkTotals {
    rx_bytes: u64,
    tx_bytes: u64,
}

#[derive(Debug, Default)]
struct MonitorMetadata {
    hostname: Option<String>,
    logical_cpu_count: Option<u32>,
    uptime_seconds: Option<u64>,
}

#[derive(Debug, serde::Deserialize)]
struct WindowsMonitorPayload {
    #[serde(default)]
    hostname: Option<String>,
    #[serde(default)]
    cores: Option<u32>,
    #[serde(default)]
    uptime: Option<u64>,
    cpu: f64,
    mem: f64,
    mem_used: f64,
    mem_total: f64,
    swap: f64,
    swap_used: f64,
    swap_total: f64,
    #[serde(default)]
    disk: f64,
    #[serde(default)]
    disk_used: Option<f64>,
    #[serde(default)]
    disk_total: Option<f64>,
    rx: f64,
    tx: f64,
    load: f64,
}

const LINUX_MONITOR_SCRIPT: &str = include_str!("monitor_scripts/linux.sh");
const MACOS_MONITOR_SCRIPT: &str = include_str!("monitor_scripts/macos.sh");
const WINDOWS_MONITOR_SCRIPT: &str = include_str!("monitor_scripts/windows.ps1");

#[derive(Default)]
struct RemoteMonitorCollector {
    platform: Option<SessionMonitorPlatform>,
    previous_cpu_totals: Option<CpuTotals>,
    previous_network_totals: Option<NetworkTotals>,
    previous_sample_at: Option<Instant>,
}

impl RemoteMonitorCollector {
    fn reset(&mut self) {
        self.platform = None;
        self.previous_cpu_totals = None;
        self.previous_network_totals = None;
        self.previous_sample_at = None;
    }

    async fn poll(
        &mut self,
        session: &Arc<client::Handle<ClientHandler>>,
    ) -> Result<SessionMonitorSnapshot> {
        let platform = if let Some(platform) = self.platform {
            platform
        } else {
            let detected = detect_monitor_platform(session).await?;
            self.platform = Some(detected);
            detected
        };

        match platform {
            SessionMonitorPlatform::Linux => self.poll_linux(session).await,
            SessionMonitorPlatform::Macos => self.poll_macos(session).await,
            SessionMonitorPlatform::Windows => self.poll_windows(session).await,
        }
    }

    async fn poll_linux(
        &mut self,
        session: &Arc<client::Handle<ClientHandler>>,
    ) -> Result<SessionMonitorSnapshot> {
        let output = run_posix_monitor_script(session, LINUX_MONITOR_SCRIPT).await?;
        self.parse_linux_snapshot(&output)
    }

    fn parse_linux_snapshot(&mut self, output: &str) -> Result<SessionMonitorSnapshot> {
        let mut metadata = MonitorMetadata::default();
        let mut cpu_totals = None;
        let mut memory_total_kib = None;
        let mut memory_available_kib = None;
        let mut swap_total_kib = None;
        let mut swap_free_kib = None;
        let mut network_totals = None;
        let mut load = None;
        let mut disk_percent = None;
        let mut disk_total_kib = None;
        let mut disk_used_kib = None;

        for line in output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let mut parts = line.split_whitespace();
            match parts.next() {
                Some("host") => {
                    metadata.hostname = non_empty_string(parts.collect::<Vec<_>>().join(" "));
                }
                Some("meta") => {
                    metadata.logical_cpu_count = parts
                        .next()
                        .and_then(|value| value.parse::<u32>().ok())
                        .filter(|value| *value > 0);
                    metadata.uptime_seconds = parts
                        .next()
                        .and_then(|value| value.parse::<u64>().ok())
                        .filter(|value| *value > 0);
                }
                Some("cpu") => {
                    let values: Vec<u64> = parts.filter_map(|value| value.parse().ok()).collect();
                    if values.len() >= 4 {
                        let total = values.iter().sum();
                        let idle = values.get(3).copied().unwrap_or_default()
                            + values.get(4).copied().unwrap_or_default();
                        cpu_totals = Some(CpuTotals { total, idle });
                    }
                }
                Some("mem") => {
                    let values: Vec<u64> = parts.filter_map(|value| value.parse().ok()).collect();
                    if values.len() >= 4 {
                        memory_total_kib = values.first().copied();
                        memory_available_kib = values.get(1).copied();
                        swap_total_kib = values.get(2).copied();
                        swap_free_kib = values.get(3).copied();
                    }
                }
                Some("net") => {
                    let values: Vec<u64> = parts.filter_map(|value| value.parse().ok()).collect();
                    if values.len() >= 2 {
                        network_totals = Some(NetworkTotals {
                            rx_bytes: values[0],
                            tx_bytes: values[1],
                        });
                    }
                }
                Some("load") => {
                    load = parts.next().and_then(|value| value.parse::<f64>().ok());
                }
                Some("disk") => {
                    disk_total_kib = parts.next().and_then(|value| value.parse::<u64>().ok());
                    disk_used_kib = parts.next().and_then(|value| value.parse::<u64>().ok());
                    disk_percent = parts.next().and_then(parse_disk_percent);
                }
                _ => {}
            }
        }

        let cpu_totals = cpu_totals.context("missing Linux CPU totals")?;
        let network_totals = network_totals.context("missing Linux network totals")?;
        let memory_total_kib = memory_total_kib.context("missing Linux memory total")? as f64;
        let memory_available_kib =
            memory_available_kib.context("missing Linux memory available")? as f64;
        let swap_total_kib = swap_total_kib.context("missing Linux swap total")? as f64;
        let swap_free_kib = swap_free_kib.context("missing Linux swap free")? as f64;
        let memory_used_kib = (memory_total_kib - memory_available_kib).max(0.0);
        let swap_used_kib = (swap_total_kib - swap_free_kib).max(0.0);

        let cpu_percent = self.compute_cpu_percent(cpu_totals);
        let (network_rx_kbps, network_tx_kbps) = self.compute_network_rates(network_totals);
        let memory_percent = if memory_total_kib > 0.0 {
            ((memory_total_kib - memory_available_kib).max(0.0) / memory_total_kib) * 100.0
        } else {
            0.0
        };
        let swap_percent = if swap_total_kib > 0.0 {
            ((swap_total_kib - swap_free_kib).max(0.0) / swap_total_kib) * 100.0
        } else {
            0.0
        };

        Ok(SessionMonitorSnapshot {
            platform: SessionMonitorPlatform::Linux,
            hostname: metadata.hostname,
            logical_cpu_count: metadata.logical_cpu_count,
            uptime_seconds: metadata.uptime_seconds,
            cpu_percent,
            memory_percent,
            memory_used_bytes: kib_to_bytes(memory_used_kib),
            memory_total_bytes: kib_to_bytes(memory_total_kib),
            swap_percent,
            swap_used_bytes: kib_to_bytes(swap_used_kib),
            swap_total_bytes: kib_to_bytes(swap_total_kib),
            disk_percent: disk_percent.unwrap_or_default(),
            disk_used_bytes: disk_used_kib.map(kib_u64_to_bytes),
            disk_total_bytes: disk_total_kib.map(kib_u64_to_bytes),
            network_rx_kbps,
            network_tx_kbps,
            load: load.unwrap_or_default(),
        })
    }

    async fn poll_macos(
        &mut self,
        session: &Arc<client::Handle<ClientHandler>>,
    ) -> Result<SessionMonitorSnapshot> {
        let output = run_posix_monitor_script(session, MACOS_MONITOR_SCRIPT).await?;
        self.parse_macos_snapshot(&output)
    }

    fn parse_macos_snapshot(&mut self, output: &str) -> Result<SessionMonitorSnapshot> {
        let mut metadata = MonitorMetadata::default();
        let mut cpu_percent = None;
        let mut memory_used = None;
        let mut memory_total = None;
        let mut swap_used = None;
        let mut swap_total = None;
        let mut network_totals = None;
        let mut load = None;
        let mut disk_percent = None;
        let mut disk_total_kib = None;
        let mut disk_used_kib = None;

        for line in output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let mut parts = line.split_whitespace();
            match parts.next() {
                Some("host") => {
                    metadata.hostname = non_empty_string(parts.collect::<Vec<_>>().join(" "));
                }
                Some("meta") => {
                    metadata.logical_cpu_count = parts
                        .next()
                        .and_then(|value| value.parse::<u32>().ok())
                        .filter(|value| *value > 0);
                    metadata.uptime_seconds = parts
                        .next()
                        .and_then(|value| value.parse::<u64>().ok())
                        .filter(|value| *value > 0);
                }
                Some("cpu") => {
                    cpu_percent = parts.next().and_then(|value| value.parse::<f64>().ok());
                }
                Some("memtotal") => {
                    memory_total = parts.next().and_then(|value| value.parse::<f64>().ok());
                }
                Some("physmem") => {
                    let raw = parts.collect::<Vec<_>>().join(" ");
                    memory_used = parse_value_before_label(&raw, "used");
                }
                Some("swapraw") => {
                    let raw = parts.collect::<Vec<_>>().join(" ");
                    swap_total = parse_value_after_label(&raw, "total");
                    swap_used = parse_value_after_label(&raw, "used");
                }
                Some("net") => {
                    let values: Vec<u64> = parts.filter_map(|value| value.parse().ok()).collect();
                    if values.len() >= 2 {
                        network_totals = Some(NetworkTotals {
                            rx_bytes: values[0],
                            tx_bytes: values[1],
                        });
                    }
                }
                Some("load") => {
                    load = parts.next().and_then(|value| value.parse::<f64>().ok());
                }
                Some("disk") => {
                    disk_total_kib = parts.next().and_then(|value| value.parse::<u64>().ok());
                    disk_used_kib = parts.next().and_then(|value| value.parse::<u64>().ok());
                    disk_percent = parts.next().and_then(parse_disk_percent);
                }
                _ => {}
            }
        }

        let memory_used = memory_used.context("missing macOS memory usage")?;
        let memory_total = memory_total.context("missing macOS memory total")?;
        let swap_used = swap_used.context("missing macOS swap usage")?;
        let swap_total = swap_total.context("missing macOS swap total")?;
        let network_totals = network_totals.context("missing macOS network totals")?;
        let (network_rx_kbps, network_tx_kbps) = self.compute_network_rates(network_totals);

        Ok(SessionMonitorSnapshot {
            platform: SessionMonitorPlatform::Macos,
            hostname: metadata.hostname,
            logical_cpu_count: metadata.logical_cpu_count,
            uptime_seconds: metadata.uptime_seconds,
            cpu_percent: cpu_percent.unwrap_or_default(),
            memory_percent: if memory_total > 0.0 {
                (memory_used / memory_total) * 100.0
            } else {
                0.0
            },
            memory_used_bytes: f64_to_u64(memory_used),
            memory_total_bytes: f64_to_u64(memory_total),
            swap_percent: if swap_total > 0.0 {
                (swap_used / swap_total) * 100.0
            } else {
                0.0
            },
            swap_used_bytes: f64_to_u64(swap_used),
            swap_total_bytes: f64_to_u64(swap_total),
            disk_percent: disk_percent.unwrap_or_default(),
            disk_used_bytes: disk_used_kib.map(kib_u64_to_bytes),
            disk_total_bytes: disk_total_kib.map(kib_u64_to_bytes),
            network_rx_kbps,
            network_tx_kbps,
            load: load.unwrap_or_default(),
        })
    }

    async fn poll_windows(
        &mut self,
        session: &Arc<client::Handle<ClientHandler>>,
    ) -> Result<SessionMonitorSnapshot> {
        let command = powershell_encoded_command(WINDOWS_MONITOR_SCRIPT);
        let output = run_exec_command(session, &command).await?;
        self.parse_windows_snapshot(&output)
    }

    fn parse_windows_snapshot(&mut self, output: &str) -> Result<SessionMonitorSnapshot> {
        let payload: WindowsMonitorPayload = serde_json::from_str(output.trim())
            .context("failed to parse Windows monitoring payload")?;
        let network_rx_kbps = payload.rx.max(0.0) / 1024.0;
        let network_tx_kbps = payload.tx.max(0.0) / 1024.0;

        Ok(SessionMonitorSnapshot {
            platform: SessionMonitorPlatform::Windows,
            hostname: payload.hostname.and_then(non_empty_string),
            logical_cpu_count: payload.cores.filter(|value| *value > 0),
            uptime_seconds: payload.uptime.filter(|value| *value > 0),
            cpu_percent: payload.cpu,
            memory_percent: payload.mem,
            memory_used_bytes: f64_to_u64(payload.mem_used),
            memory_total_bytes: f64_to_u64(payload.mem_total),
            swap_percent: payload.swap,
            swap_used_bytes: f64_to_u64(payload.swap_used),
            swap_total_bytes: f64_to_u64(payload.swap_total),
            disk_percent: payload.disk.clamp(0.0, 100.0),
            disk_used_bytes: payload.disk_used.map(f64_to_u64),
            disk_total_bytes: payload.disk_total.map(f64_to_u64),
            network_rx_kbps,
            network_tx_kbps,
            load: payload.load,
        })
    }

    fn compute_cpu_percent(&mut self, next: CpuTotals) -> f64 {
        let cpu_percent = if let Some(previous) = self.previous_cpu_totals {
            let total_delta = next.total.saturating_sub(previous.total);
            let idle_delta = next.idle.saturating_sub(previous.idle);
            if total_delta > 0 {
                ((total_delta.saturating_sub(idle_delta)) as f64 / total_delta as f64) * 100.0
            } else {
                0.0
            }
        } else {
            0.0
        };
        self.previous_cpu_totals = Some(next);
        cpu_percent
    }

    fn compute_network_rates(&mut self, next: NetworkTotals) -> (f64, f64) {
        let now = Instant::now();
        let rates = if let (Some(previous), Some(previous_at)) =
            (self.previous_network_totals, self.previous_sample_at)
        {
            let elapsed = now.saturating_duration_since(previous_at).as_secs_f64();
            if elapsed > 0.0 {
                (
                    (next.rx_bytes.saturating_sub(previous.rx_bytes) as f64 / 1024.0) / elapsed,
                    (next.tx_bytes.saturating_sub(previous.tx_bytes) as f64 / 1024.0) / elapsed,
                )
            } else {
                (0.0, 0.0)
            }
        } else {
            (0.0, 0.0)
        };

        self.previous_network_totals = Some(next);
        self.previous_sample_at = Some(now);
        rates
    }
}

pub(super) async fn run_monitor_loop(
    session: Arc<client::Handle<ClientHandler>>,
    event_sender: SessionEventSender,
    mut enabled_receiver: watch::Receiver<bool>,
) {
    let mut collector = RemoteMonitorCollector::default();

    loop {
        if !*enabled_receiver.borrow() {
            collector.reset();
            if enabled_receiver.changed().await.is_err() {
                break;
            }
            continue;
        }

        match collector.poll(&session).await {
            Ok(snapshot) => {
                if event_sender
                    .send(SessionEvent::MonitorUpdated(snapshot))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(error) => {
                if event_sender
                    .send(SessionEvent::MonitorFailed(error.to_string()))
                    .await
                    .is_err()
                {
                    break;
                }
                collector.reset();
                if enabled_receiver.changed().await.is_err() {
                    break;
                }
                continue;
            }
        }

        tokio::select! {
            changed = enabled_receiver.changed() => {
                if changed.is_err() {
                    break;
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
        }
    }
}

async fn detect_monitor_platform(
    session: &Arc<client::Handle<ClientHandler>>,
) -> Result<SessionMonitorPlatform> {
    match run_exec_command(session, "uname -s").await {
        Ok(output) => match output.trim() {
            "Linux" => Ok(SessionMonitorPlatform::Linux),
            "Darwin" => Ok(SessionMonitorPlatform::Macos),
            other => bail!("unsupported remote OS for monitoring: {other}"),
        },
        Err(_) => {
            let output = run_exec_command(
                session,
                "powershell -NoProfile -Command \"Write-Output Windows\"",
            )
            .await?;
            if output.trim().eq_ignore_ascii_case("windows") {
                Ok(SessionMonitorPlatform::Windows)
            } else {
                bail!("unsupported remote OS for monitoring")
            }
        }
    }
}

async fn run_posix_monitor_script(
    session: &Arc<client::Handle<ClientHandler>>,
    script: &str,
) -> Result<String> {
    let script = normalize_posix_script_line_endings(script);
    let command = format!("/bin/sh -c {}", quote_shell_argument(&script));
    run_exec_command(session, &command).await
}

fn normalize_posix_script_line_endings(script: &str) -> Cow<'_, str> {
    if script.contains('\r') {
        Cow::Owned(script.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(script)
    }
}

fn quote_shell_argument(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn powershell_encoded_payload(script: &str) -> String {
    let mut bytes = Vec::with_capacity(script.len() * 2);
    for unit in script.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn powershell_encoded_command(script: &str) -> String {
    format!(
        "powershell.exe -NoProfile -EncodedCommand {}",
        powershell_encoded_payload(script)
    )
}

pub(super) async fn run_exec_command(
    session: &Arc<client::Handle<ClientHandler>>,
    command: &str,
) -> Result<String> {
    let mut channel = session
        .channel_open_session()
        .await
        .context("failed to open SSH exec channel")?;
    channel
        .exec(true, command.as_bytes().to_vec())
        .await
        .with_context(|| format!("failed to execute remote command: {command}"))?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_status = None;

    while let Some(message) = channel.wait().await {
        match message {
            ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
            ChannelMsg::ExtendedData { data, .. } => stderr.extend_from_slice(&data),
            ChannelMsg::ExitStatus {
                exit_status: status,
            } => exit_status = Some(status),
            ChannelMsg::Eof | ChannelMsg::Close => {}
            _ => {}
        }
    }

    if let Err(error) = channel.close().await {
        log::debug!("failed to close SSH exec channel cleanly: {error:?}");
    }

    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    let stderr = String::from_utf8_lossy(&stderr).into_owned();
    if exit_status.unwrap_or(0) != 0 {
        let stderr_preview = stderr.trim();
        let stdout_preview = stdout.trim();
        if stderr_preview.is_empty() {
            bail!("remote monitoring command failed: {stdout_preview}");
        } else {
            bail!("remote monitoring command failed: {stderr_preview}");
        }
    }

    Ok(stdout)
}

pub(super) async fn run_exec_pty_command(
    session: &Arc<client::Handle<ClientHandler>>,
    command: &str,
    columns: u32,
    lines: u32,
) -> Result<String> {
    let mut channel = session
        .channel_open_session()
        .await
        .context("failed to open SSH session channel for PTY exec")?;

    channel
        .request_pty(true, "xterm-256color", columns, lines, 0, 0, &[])
        .await
        .context("failed to request PTY for exec")?;

    channel
        .exec(true, command.as_bytes().to_vec())
        .await
        .with_context(|| format!("failed to execute remote command with PTY: {command}"))?;

    // With a PTY allocated, stdout and stderr are merged at the transport layer.
    // Only ChannelMsg::Data arrives; ExtendedData will not fire.
    let mut output = Vec::new();
    let mut exit_status = None;

    while let Some(message) = channel.wait().await {
        match message {
            ChannelMsg::Data { data } => output.extend_from_slice(&data),
            ChannelMsg::ExitStatus {
                exit_status: status,
            } => exit_status = Some(status),
            ChannelMsg::Eof | ChannelMsg::Close => {}
            _ => {}
        }
    }

    if let Err(error) = channel.close().await {
        log::debug!("failed to close SSH PTY exec channel cleanly: {error:?}");
    }

    let raw = String::from_utf8_lossy(&output).into_owned();
    let cleaned = strip_ansi_text(&raw);

    if exit_status.unwrap_or(0) != 0 {
        if cleaned.trim().is_empty() {
            bail!(
                "remote PTY command failed with exit status {}",
                exit_status.unwrap_or(0)
            );
        } else {
            let preview: String = cleaned.lines().take(20).collect::<Vec<_>>().join("\n");
            bail!("remote PTY command failed: {preview}");
        }
    }

    Ok(cleaned)
}

/// Strip ANSI escape sequences from PTY output.
/// Handles CSI sequences (ESC [ ... final_byte), OSC sequences (ESC ] ... BEL/ST),
/// and carriage returns.
fn strip_ansi_text(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(next) = chars.next() {
                        if next == '\x07' {
                            break;
                        }
                        if next == '\x1b' && chars.peek().copied() == Some('\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if ch != '\r' {
            output.push(ch);
        }
    }
    output
}

fn non_empty_string(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn f64_to_u64(value: f64) -> u64 {
    if value.is_finite() && value > 0.0 {
        value.min(u64::MAX as f64).round() as u64
    } else {
        0
    }
}

fn kib_to_bytes(value: f64) -> u64 {
    f64_to_u64(value * 1024.0)
}

fn kib_u64_to_bytes(value: u64) -> u64 {
    value.saturating_mul(1024)
}

fn normalized_label(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphabetic())
        .flat_map(char::to_lowercase)
        .collect()
}

fn parse_value_before_label(input: &str, label: &str) -> Option<f64> {
    let parts = input.split_whitespace().collect::<Vec<_>>();
    parts.windows(2).find_map(|pair| {
        (normalized_label(pair[1]) == label)
            .then(|| {
                parse_scaled_number(
                    pair[0].trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '.'),
                )
            })
            .flatten()
    })
}

fn parse_value_after_label(input: &str, label: &str) -> Option<f64> {
    let parts = input.split_whitespace().collect::<Vec<_>>();
    let label_index = parts
        .iter()
        .position(|part| normalized_label(part) == label)?;
    parts[label_index + 1..].iter().find_map(|part| {
        let value = part.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '.');
        (!value.is_empty())
            .then(|| parse_scaled_number(value))
            .flatten()
    })
}

fn parse_scaled_number(value: &str) -> Option<f64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut chars = trimmed.chars();
    let suffix = chars.next_back()?;
    let (number_part, scale) = match suffix {
        'K' | 'k' => (chars.as_str(), 1024.0),
        'M' | 'm' => (chars.as_str(), 1024.0 * 1024.0),
        'G' | 'g' => (chars.as_str(), 1024.0 * 1024.0 * 1024.0),
        'T' | 't' => (chars.as_str(), 1024.0 * 1024.0 * 1024.0 * 1024.0),
        '0'..='9' | '.' => return trimmed.parse::<f64>().ok(),
        _ => return None,
    };

    number_part
        .trim()
        .parse::<f64>()
        .ok()
        .map(|value| value * scale)
}

fn parse_disk_percent(value: &str) -> Option<f64> {
    value
        .trim()
        .trim_end_matches('%')
        .parse::<f64>()
        .ok()
        .map(|value| {
            if value.is_finite() {
                value.clamp(0.0, 100.0)
            } else {
                0.0
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linux_snapshot_with_resource_capacities() {
        let mut collector = RemoteMonitorCollector::default();
        let snapshot = collector
            .parse_linux_snapshot(
                "host demo-linux\nmeta 8 86461\ncpu 10 2 3 85 0 0 0 0\nmem 8192 2048 4096 3072\nnet 102400 204800\nload 2.4\ndisk 1024000 512000 50\n",
            )
            .expect("Linux fixture should parse");

        assert_eq!(snapshot.platform, SessionMonitorPlatform::Linux);
        assert_eq!(snapshot.hostname.as_deref(), Some("demo-linux"));
        assert_eq!(snapshot.logical_cpu_count, Some(8));
        assert_eq!(snapshot.uptime_seconds, Some(86461));
        assert_eq!(snapshot.memory_used_bytes, 6 * 1024 * 1024);
        assert_eq!(snapshot.memory_total_bytes, 8 * 1024 * 1024);
        assert_eq!(snapshot.swap_used_bytes, 1024 * 1024);
        assert_eq!(snapshot.swap_total_bytes, 4 * 1024 * 1024);
        assert_eq!(snapshot.disk_used_bytes, Some(512000 * 1024));
        assert_eq!(snapshot.disk_total_bytes, Some(1024000 * 1024));
        assert_eq!(snapshot.disk_percent, 50.0);
    }

    #[test]
    fn parses_macos_snapshot_and_tolerates_missing_metadata() {
        let mut collector = RemoteMonitorCollector::default();
        let snapshot = collector
            .parse_macos_snapshot(
                "cpu 24.5\nmemtotal 8589934592\nphysmem PhysMem: 6G used (1536M wired, 512M compressor), 2G unused.\nswapraw total = 2048.00M used = 512.00M free = 1536.00M (encrypted)\nnet 1000 2000\nload 1.25\ndisk 2048000 1024000 50\n",
            )
            .expect("macOS fixture should parse");

        assert_eq!(snapshot.platform, SessionMonitorPlatform::Macos);
        assert_eq!(snapshot.hostname, None);
        assert_eq!(snapshot.logical_cpu_count, None);
        assert_eq!(snapshot.uptime_seconds, None);
        assert_eq!(snapshot.memory_used_bytes, 6 * 1024 * 1024 * 1024);
        assert_eq!(snapshot.memory_total_bytes, 8 * 1024 * 1024 * 1024);
        assert_eq!(snapshot.swap_used_bytes, 512 * 1024 * 1024);
        assert_eq!(snapshot.swap_total_bytes, 2 * 1024 * 1024 * 1024);
        assert_eq!(snapshot.disk_percent, 50.0);
    }

    #[test]
    fn quotes_posix_monitor_scripts_for_the_login_shell() {
        assert_eq!(
            quote_shell_argument("printf '%s' \"$HOME\""),
            "'printf '\"'\"'%s'\"'\"' \"$HOME\"'"
        );
    }

    #[test]
    fn normalizes_embedded_posix_scripts_to_lf() {
        let lf = "first\nsecond\n";
        assert!(matches!(
            normalize_posix_script_line_endings(lf),
            Cow::Borrowed(_)
        ));

        let normalized = normalize_posix_script_line_endings("first\r\nsecond\rthird\r\n");
        assert_eq!(normalized, "first\nsecond\nthird\n");
        assert!(!normalized.contains('\r'));

        for script in [LINUX_MONITOR_SCRIPT, MACOS_MONITOR_SCRIPT] {
            assert!(!normalize_posix_script_line_endings(script).contains('\r'));
        }
    }

    #[test]
    fn windows_monitor_script_uses_utf16_encoded_command() {
        let command = powershell_encoded_command(WINDOWS_MONITOR_SCRIPT);
        let payload = command
            .strip_prefix("powershell.exe -NoProfile -EncodedCommand ")
            .expect("Windows monitor command should use EncodedCommand");
        assert!(!command.contains("$cpu"));
        assert!(
            command.len() < 8191,
            "encoded command exceeds cmd.exe limit"
        );

        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload)
            .expect("payload should be valid base64");
        let mut chunks = bytes.chunks_exact(2);
        let units = chunks
            .by_ref()
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        assert!(chunks.remainder().is_empty());
        assert_eq!(String::from_utf16(&units).unwrap(), WINDOWS_MONITOR_SCRIPT);
    }

    #[test]
    fn windows_monitor_script_rejects_missing_required_counters() {
        assert!(WINDOWS_MONITOR_SCRIPT.contains("throw \"Missing performance counter: $pattern\""));
        for required_counter in [
            "processor time$' $false $true",
            "processor queue length$' $false $true",
            "system up time$' $false $true",
            "bytes received/sec$' $true $true",
            "bytes sent/sec$' $true $true",
        ] {
            assert!(
                WINDOWS_MONITOR_SCRIPT.contains(required_counter),
                "counter should be required: {required_counter}"
            );
        }
        assert!(
            WINDOWS_MONITOR_SCRIPT.contains("paging file\\(_total\\)\\\\% usage$' $false $false")
        );
        assert!(!WINDOWS_MONITOR_SCRIPT.contains("if ($null -eq $rx)"));
        assert!(!WINDOWS_MONITOR_SCRIPT.contains("if ($null -eq $tx)"));
    }

    #[test]
    fn parses_windows_snapshot_with_optional_disk_details() {
        let mut collector = RemoteMonitorCollector::default();
        let snapshot = collector
            .parse_windows_snapshot(
                r#"{"hostname":"WIN-HOST","cores":16,"uptime":3600,"cpu":30.5,"mem":62.5,"mem_used":5368709120.0,"mem_total":8589934592.0,"swap":25.0,"swap_used":536870912.0,"swap_total":2147483648.0,"disk":75.0,"disk_used":80530636800.0,"disk_total":107374182400.0,"rx":1000.0,"tx":2000.0,"load":3.0}"#,
            )
            .expect("Windows fixture should parse");

        assert_eq!(snapshot.platform, SessionMonitorPlatform::Windows);
        assert_eq!(snapshot.hostname.as_deref(), Some("WIN-HOST"));
        assert_eq!(snapshot.logical_cpu_count, Some(16));
        assert_eq!(snapshot.uptime_seconds, Some(3600));
        assert_eq!(snapshot.memory_used_bytes, 5 * 1024 * 1024 * 1024);
        assert_eq!(snapshot.disk_percent, 75.0);
        assert_eq!(snapshot.disk_total_bytes, Some(100 * 1024 * 1024 * 1024));
        assert!((snapshot.network_rx_kbps - (1000.0 / 1024.0)).abs() < f64::EPSILON);
        assert!((snapshot.network_tx_kbps - (2000.0 / 1024.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn parses_disk_percent_variants() {
        assert_eq!(parse_disk_percent("42"), Some(42.0));
        assert_eq!(parse_disk_percent("87%"), Some(87.0));
        assert_eq!(parse_disk_percent("120%"), Some(100.0));
        assert_eq!(parse_disk_percent("-1"), Some(0.0));
        assert_eq!(parse_disk_percent(""), None);
        assert_eq!(parse_disk_percent("n/a"), None);
    }
}
