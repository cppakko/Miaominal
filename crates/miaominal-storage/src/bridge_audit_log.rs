use anyhow::{Context, Result, anyhow};
use miaominal_core::ssh_bridge_security::{
    BridgeAuditRecord, BridgeAuthorizationOutcome, BridgeConnectionOutcome, BridgeDecisionSource,
    BridgePeerIdentity, BridgeSecurityLevel,
};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const AUDIT_LOG_FILE_NAME: &str = "ssh_bridge_audit.log";
const AUDIT_LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;
const AUDIT_LOG_MAX_ROTATED_FILES: u32 = 3;

#[derive(Clone)]
pub struct BridgeAuditLog {
    state: std::sync::Arc<Mutex<AuditLogState>>,
}

struct AuditLogState {
    path: PathBuf,
    file: File,
}

impl BridgeAuditLog {
    pub fn open_default() -> Result<Self> {
        Self::open(&miaominal_paths::config_file(AUDIT_LOG_FILE_NAME)?)
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        Ok(Self {
            state: std::sync::Arc::new(Mutex::new(AuditLogState {
                path: path.to_path_buf(),
                file,
            })),
        })
    }

    pub fn path(&self) -> PathBuf {
        self.state
            .lock()
            .map(|state| state.path.clone())
            .unwrap_or_default()
    }

    pub fn write_requested(&self, record: &BridgeAuditRecord) -> Result<()> {
        let mut line = format!(
            "{} | requested | request={} | profile={} | name=\"{}\" | level={} | {}",
            utc_timestamp(record.requested_at),
            sanitize(&record.request_id),
            sanitize(record.profile_id.as_deref().unwrap_or("-")),
            sanitize(record.profile_name.as_deref().unwrap_or("-")),
            level_label(record.security_level),
            peer_label(&record.peer),
        );
        line.push('\n');
        self.append(&line)
    }

    pub fn write_finished(&self, record: &BridgeAuditRecord) -> Result<()> {
        let finished_at = record.finished_at.unwrap_or(record.requested_at);
        let duration = finished_at.saturating_sub(record.requested_at).max(0);
        let decision = match record.decision_source {
            Some(BridgeDecisionSource::App) => "app",
            Some(BridgeDecisionSource::SystemAuth) => "system_auth",
            None => "-",
        };
        let error = record.error_code.as_deref().unwrap_or("-");
        let mut line = format!(
            "{} | finished | request={} | profile={} | authorization={} | decision={} | connection={} | duration={}s | error={}",
            utc_timestamp(finished_at),
            sanitize(&record.request_id),
            sanitize(record.profile_id.as_deref().unwrap_or("-")),
            authorization_label(record.authorization_outcome),
            decision,
            connection_label(record.connection_outcome),
            duration,
            sanitize(error),
        );
        line.push('\n');
        self.append(&line)
    }

    fn append(&self, line: &str) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("SSH Bridge audit log lock is poisoned"))?;
        if should_rotate(&state.file, line.len() as u64)? {
            rotate_logs(&state.path)?;
            state.file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&state.path)
                .with_context(|| format!("failed to reopen {}", state.path.display()))?;
        }
        state
            .file
            .write_all(line.as_bytes())
            .context("failed to write SSH Bridge audit log")?;
        state
            .file
            .flush()
            .context("failed to flush SSH Bridge audit log")?;
        Ok(())
    }
}

fn should_rotate(file: &File, incoming: u64) -> Result<bool> {
    let size = file
        .metadata()
        .context("failed to inspect SSH Bridge audit log size")?
        .len();
    Ok(size + incoming > AUDIT_LOG_MAX_BYTES)
}

fn rotate_logs(path: &Path) -> Result<()> {
    for index in (1..=AUDIT_LOG_MAX_ROTATED_FILES).rev() {
        let current = rotated_path(path, index);
        if current.exists() {
            if index == AUDIT_LOG_MAX_ROTATED_FILES {
                std::fs::remove_file(&current)
                    .with_context(|| format!("failed to remove {}", current.display()))?;
            } else {
                let next = rotated_path(path, index + 1);
                std::fs::rename(&current, &next)
                    .with_context(|| format!("failed to rotate {}", current.display()))?;
            }
        }
    }
    std::fs::rename(path, rotated_path(path, 1))
        .with_context(|| format!("failed to rotate {}", path.display()))?;
    Ok(())
}

fn rotated_path(path: &Path, index: u32) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(format!(".{index}"));
    value.into()
}

fn level_label(level: BridgeSecurityLevel) -> String {
    match level {
        BridgeSecurityLevel::Standard => "standard".into(),
        BridgeSecurityLevel::RequireApproval { timeout_secs } => {
            format!("approval({timeout_secs}s)")
        }
        BridgeSecurityLevel::RequireSystemAuth => "system_auth".into(),
    }
}

fn authorization_label(outcome: BridgeAuthorizationOutcome) -> &'static str {
    match outcome {
        BridgeAuthorizationOutcome::NotRequired => "not_required",
        BridgeAuthorizationOutcome::Pending => "pending",
        BridgeAuthorizationOutcome::Approved => "approved",
        BridgeAuthorizationOutcome::Rejected => "rejected",
        BridgeAuthorizationOutcome::TimedOut => "timed_out",
        BridgeAuthorizationOutcome::Unsupported => "unsupported",
        BridgeAuthorizationOutcome::Failed => "failed",
        BridgeAuthorizationOutcome::Cancelled => "cancelled",
    }
}

fn connection_label(outcome: BridgeConnectionOutcome) -> &'static str {
    match outcome {
        BridgeConnectionOutcome::Pending => "pending",
        BridgeConnectionOutcome::Rejected => "rejected",
        BridgeConnectionOutcome::UpstreamFailed => "upstream_failed",
        BridgeConnectionOutcome::Active => "active",
        BridgeConnectionOutcome::Disconnected => "disconnected",
        BridgeConnectionOutcome::Interrupted => "interrupted",
    }
}

fn peer_label(peer: &BridgePeerIdentity) -> String {
    let pid = peer
        .pid
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "-".into());
    let uid = peer
        .uid
        .map(|uid| uid.to_string())
        .unwrap_or_else(|| "-".into());
    let gid = peer
        .gid
        .map(|gid| gid.to_string())
        .unwrap_or_else(|| "-".into());
    let sid = peer.user_sid.as_deref().unwrap_or("-");
    let path = peer.source_path().unwrap_or("-");
    format!(
        "pid={} | uid={} | gid={} | sid={} | path=\"{}\"",
        sanitize(&pid),
        sanitize(&uid),
        sanitize(&gid),
        sanitize(sid),
        sanitize(path),
    )
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() || character == '"' || character == '|' {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn utc_timestamp(unix_secs: i64) -> String {
    let Ok(value) = time::OffsetDateTime::from_unix_timestamp(unix_secs) else {
        return unix_secs.to_string();
    };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        value.year(),
        u8::from(value.month()),
        value.day(),
        value.hour(),
        value.minute(),
        value.second()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::ssh_bridge_security::{
        BridgeAuditRecord, BridgeAuthorizationOutcome, BridgeConnectionOutcome, BridgePeerIdentity,
        BridgeProcessCaptureStatus, BridgeProcessIdentity,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now_secs() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .min(i64::MAX as u64) as i64
    }

    fn record(request_id: &str) -> BridgeAuditRecord {
        BridgeAuditRecord {
            request_id: request_id.into(),
            owner_instance_id: "test-owner".into(),
            requested_at: 1_700_000_000,
            decision_at: Some(1_700_000_005),
            connected_at: Some(1_700_000_006),
            finished_at: Some(1_700_000_010),
            profile_id: Some("profile-a".into()),
            profile_name: Some("Profile A".into()),
            security_level: BridgeSecurityLevel::RequireApproval { timeout_secs: 30 },
            peer: BridgePeerIdentity {
                pid: Some(1234),
                process_chain: vec![BridgeProcessIdentity {
                    pid: 1234,
                    parent_pid: None,
                    started_at: None,
                    executable_path: Some("C:\\path\\ssh.exe".into()),
                    capture_status: BridgeProcessCaptureStatus::Captured,
                }],
                capture_status: Some(BridgeProcessCaptureStatus::Captured),
                ..BridgePeerIdentity::default()
            },
            authorization_outcome: BridgeAuthorizationOutcome::Approved,
            connection_outcome: BridgeConnectionOutcome::Disconnected,
            decision_source: Some(BridgeDecisionSource::App),
            error_code: None,
        }
    }

    #[test]
    fn audit_lines_are_human_readable_and_greppable() {
        let directory = tempfile::tempdir().unwrap();
        let log = BridgeAuditLog::open(&directory.path().join("audit.log")).unwrap();
        let record = record("request-a");
        log.write_requested(&record).unwrap();
        log.write_finished(&record).unwrap();

        let content = std::fs::read_to_string(directory.path().join("audit.log")).unwrap();
        let lines = content.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("| requested | request=request-a |"));
        assert!(lines[0].contains("level=approval(30s)"));
        assert!(lines[0].contains("pid=1234"));
        assert!(lines[0].contains("path=\"C:\\path\\ssh.exe\""));
        assert!(lines[1].contains("| finished | request=request-a |"));
        assert!(lines[1].contains("authorization=approved"));
        assert!(lines[1].contains("decision=app"));
        assert!(lines[1].contains("connection=disconnected"));
        assert!(lines[1].contains("duration=10s"));
        assert!(lines[1].contains("error=-"));
    }

    #[test]
    fn control_characters_and_delimiters_are_sanitized() {
        let directory = tempfile::tempdir().unwrap();
        let log = BridgeAuditLog::open(&directory.path().join("audit.log")).unwrap();
        let mut record = record("request-a");
        record.profile_name = Some("a|b\"c\nd".into());
        log.write_requested(&record).unwrap();

        let content = std::fs::read_to_string(directory.path().join("audit.log")).unwrap();
        assert!(content.contains("name=\"a b c d\""));
        assert_eq!(content.lines().count(), 1);
    }

    #[test]
    fn oversized_log_rotates_while_preserving_recent_lines() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("audit.log");
        std::fs::write(&path, "x".repeat(AUDIT_LOG_MAX_BYTES as usize)).unwrap();
        let log = BridgeAuditLog::open(&path).unwrap();
        log.write_finished(&record("after-rotation")).unwrap();

        assert!(directory.path().join("audit.log.1").exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("request=after-rotation"));
        assert_eq!(content.lines().count(), 1);
    }

    #[test]
    fn timestamp_fallback_never_panics() {
        assert_eq!(
            utc_timestamp(now_secs()).len(),
            "2026-08-03T12:34:56Z".len()
        );
    }
}
