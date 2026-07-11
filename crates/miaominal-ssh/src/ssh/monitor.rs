use super::session::{ClientHandler, SessionEvent, SessionEventSender};
use anyhow::{Context, Result, bail};
use miaominal_core::forwarding::{SessionMonitorPlatform, SessionMonitorSnapshot};
use russh::ChannelMsg;
use russh::client;
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
        const LINUX_MONITOR_COMMAND: &str = "awk 'NR==1 { printf(\"cpu %s %s %s %s %s %s %s %s\\n\", $2, $3, $4, $5, $6, $7, $8, $9) }' /proc/stat; awk '/MemTotal:/ { total=$2 } /MemAvailable:/ { available=$2 } /SwapTotal:/ { swap_total=$2 } /SwapFree:/ { swap_free=$2 } END { printf(\"mem %s %s %s %s\\n\", total, available, swap_total, swap_free) }' /proc/meminfo; awk 'NR>2 && $1 !~ /^lo:/ { rx+=$2; tx+=$10 } END { printf(\"net %s %s\\n\", rx, tx) }' /proc/net/dev; awk '{ printf(\"load %s\\n\", $1) }' /proc/loadavg; df -P / 2>/dev/null | awk 'NR==2 { gsub(/%/, \"\", $5); printf(\"disk %s\\n\", $5) }'";

        let output = run_exec_command(session, LINUX_MONITOR_COMMAND).await?;
        let mut cpu_totals = None;
        let mut memory_total_kib = None;
        let mut memory_available_kib = None;
        let mut swap_total_kib = None;
        let mut swap_free_kib = None;
        let mut network_totals = None;
        let mut load = None;
        let mut disk_percent = None;

        for line in output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let mut parts = line.split_whitespace();
            match parts.next() {
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
            cpu_percent,
            memory_percent,
            swap_percent,
            disk_percent: disk_percent.unwrap_or_default(),
            network_rx_kbps,
            network_tx_kbps,
            load: load.unwrap_or_default(),
        })
    }

    async fn poll_macos(
        &mut self,
        session: &Arc<client::Handle<ClientHandler>>,
    ) -> Result<SessionMonitorSnapshot> {
        const MACOS_MONITOR_COMMAND: &str = "top -l 1 -n 0 | awk -F'[:,% ]+' '/CPU usage/ { printf(\"cpu %.2f\\n\", 100 - $(NF-1)) } /PhysMem:/ { used=0; free=0; for (i=1; i<=NF; i++) { if ($(i+1) == \"used\") used=$i; if ($(i+1) == \"unused\") free=$i } if (used + free > 0) { printf(\"mem %s %s\\n\", used, used + free) } }'; sysctl -n vm.swapusage | awk -F'[ =]+' '{ printf(\"swap %s %s\\n\", $4, $7) }'; netstat -ibn | awk 'NR>1 && $1 !~ /^lo/ && $7 ~ /^[0-9]+$/ && $10 ~ /^[0-9]+$/ { rx+=$7; tx+=$10 } END { printf(\"net %s %s\\n\", rx, tx) }'; sysctl -n vm.loadavg | awk '{ gsub(/[{}]/, \"\"); printf(\"load %s\\n\", $1) }'; df -P / 2>/dev/null | awk 'NR==2 { gsub(/%/, \"\", $5); printf(\"disk %s\\n\", $5) }'";

        let output = run_exec_command(session, MACOS_MONITOR_COMMAND).await?;
        let mut cpu_percent = None;
        let mut memory_used = None;
        let mut memory_total = None;
        let mut swap_used = None;
        let mut swap_total = None;
        let mut network_totals = None;
        let mut load = None;
        let mut disk_percent = None;

        for line in output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let mut parts = line.split_whitespace();
            match parts.next() {
                Some("cpu") => {
                    cpu_percent = parts.next().and_then(|value| value.parse::<f64>().ok());
                }
                Some("mem") => {
                    memory_used = parts.next().and_then(parse_scaled_number);
                    memory_total = parts.next().and_then(parse_scaled_number);
                }
                Some("swap") => {
                    swap_total = parts.next().and_then(parse_scaled_number);
                    swap_used = parts.next().and_then(parse_scaled_number);
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
            cpu_percent: cpu_percent.unwrap_or_default(),
            memory_percent: if memory_total > 0.0 {
                (memory_used / memory_total) * 100.0
            } else {
                0.0
            },
            swap_percent: if swap_total > 0.0 {
                (swap_used / swap_total) * 100.0
            } else {
                0.0
            },
            disk_percent: disk_percent.unwrap_or_default(),
            network_rx_kbps,
            network_tx_kbps,
            load: load.unwrap_or_default(),
        })
    }

    async fn poll_windows(
        &mut self,
        session: &Arc<client::Handle<ClientHandler>>,
    ) -> Result<SessionMonitorSnapshot> {
        const WINDOWS_MONITOR_COMMAND: &str = r#"powershell -NoProfile -Command "$cpu=(Get-Counter '\Processor(_Total)\% Processor Time').CounterSamples[0].CookedValue; $os=Get-CimInstance Win32_OperatingSystem; $mem=if($os.TotalVisibleMemorySize -gt 0){100 - (($os.FreePhysicalMemory / $os.TotalVisibleMemorySize) * 100)} else {0}; $swapBase=[double]($os.TotalVirtualMemorySize - $os.TotalVisibleMemorySize); $swapUsed=[double](($os.TotalVirtualMemorySize - $os.FreeVirtualMemory) - ($os.TotalVisibleMemorySize - $os.FreePhysicalMemory)); $swap=if($swapBase -gt 0){[Math]::Max(0,[Math]::Min(100,($swapUsed / $swapBase) * 100))} else {0}; $stats=Get-NetAdapterStatistics | Where-Object { $_.Name -notmatch 'Loopback' -and $_.ReceivedBytes -ne $null -and $_.SentBytes -ne $null }; $rx=($stats | Measure-Object -Property ReceivedBytes -Sum).Sum; $tx=($stats | Measure-Object -Property SentBytes -Sum).Sum; if($null -eq $rx){$rx=0}; if($null -eq $tx){$tx=0}; $load=(Get-Counter '\System\Processor Queue Length').CounterSamples[0].CookedValue; $systemDrive=$env:SystemDrive; $diskInfo=Get-CimInstance Win32_LogicalDisk | Where-Object { $_.DeviceID -eq $systemDrive }; $disk=if($diskInfo -and $diskInfo.Size -gt 0){100 - (($diskInfo.FreeSpace / $diskInfo.Size) * 100)} else {0}; [pscustomobject]@{cpu=$cpu;mem=$mem;swap=$swap;disk=$disk;rx=$rx;tx=$tx;load=$load} | ConvertTo-Json -Compress""#;

        #[derive(serde::Deserialize)]
        struct WindowsMonitorPayload {
            cpu: f64,
            mem: f64,
            swap: f64,
            #[serde(default)]
            disk: f64,
            rx: f64,
            tx: f64,
            load: f64,
        }

        let output = run_exec_command(session, WINDOWS_MONITOR_COMMAND).await?;
        let payload: WindowsMonitorPayload = serde_json::from_str(output.trim())
            .context("failed to parse Windows monitoring payload")?;
        let (network_rx_kbps, network_tx_kbps) = self.compute_network_rates(NetworkTotals {
            rx_bytes: payload.rx.max(0.0) as u64,
            tx_bytes: payload.tx.max(0.0) as u64,
        });

        Ok(SessionMonitorSnapshot {
            cpu_percent: payload.cpu,
            memory_percent: payload.mem,
            swap_percent: payload.swap,
            disk_percent: payload.disk.clamp(0.0, 100.0),
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
    fn parses_disk_percent_variants() {
        assert_eq!(parse_disk_percent("42"), Some(42.0));
        assert_eq!(parse_disk_percent("87%"), Some(87.0));
        assert_eq!(parse_disk_percent("120%"), Some(100.0));
        assert_eq!(parse_disk_percent("-1"), Some(0.0));
        assert_eq!(parse_disk_percent(""), None);
        assert_eq!(parse_disk_percent("n/a"), None);
    }
}
