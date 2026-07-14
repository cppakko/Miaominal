#[derive(Debug, Clone)]
pub struct HostKeyPrompt {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
    pub previous_fingerprint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct KbiPrompt {
    pub prompt: String,
    pub echo: bool,
}

#[derive(Debug, Clone)]
pub struct KbiChallenge {
    pub name: String,
    pub instructions: String,
    pub prompts: Vec<KbiPrompt>,
}

#[derive(Debug, Clone, Copy)]
pub enum HostKeyDecision {
    AcceptOnce,
    AcceptAndSave,
    Reject,
}

#[derive(Debug, Clone)]
pub struct AgentIdentitySummary {
    pub serialized: String,
    pub label: String,
    pub comment: String,
    pub kind: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMonitorPlatform {
    Linux,
    Macos,
    Windows,
}

#[derive(Debug, Clone)]
pub struct SessionMonitorSnapshot {
    pub platform: SessionMonitorPlatform,
    pub hostname: Option<String>,
    pub logical_cpu_count: Option<u32>,
    pub uptime_seconds: Option<u64>,
    pub cpu_percent: f64,
    pub memory_percent: f64,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    pub swap_percent: f64,
    pub swap_used_bytes: u64,
    pub swap_total_bytes: u64,
    pub disk_percent: f64,
    pub disk_used_bytes: Option<u64>,
    pub disk_total_bytes: Option<u64>,
    pub network_rx_kbps: f64,
    pub network_tx_kbps: f64,
    pub load: f64,
}
