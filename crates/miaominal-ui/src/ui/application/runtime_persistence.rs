//! Durable reconciliation of forwarding execution and configuration.
//!
//! Every attempt starts from the latest committed catalog. Runtime observations
//! remain visible even when the storage write fails; only a successful write
//! may be adopted as committed configuration.

use miaominal_core::profile::SessionProfile;
use miaominal_services::PortForwardManagerSnapshot;

pub(super) fn retry_delay(attempt: usize) -> std::time::Duration {
    const DELAYS_MS: [u64; 5] = [250, 1_000, 4_000, 10_000, 30_000];
    std::time::Duration::from_millis(DELAYS_MS[attempt.min(DELAYS_MS.len() - 1)])
}

pub(super) fn persisted_runtime_candidate(
    committed: &[SessionProfile],
    runtime: &PortForwardManagerSnapshot,
    persist: impl FnOnce(&[SessionProfile]) -> anyhow::Result<()>,
) -> anyhow::Result<Option<Vec<SessionProfile>>> {
    let mut candidate = committed.to_vec();
    if !super::reconcile_port_forward_enabled(&mut candidate, runtime) {
        return Ok(None);
    }
    persist(&candidate)?;
    Ok(Some(candidate))
}

#[derive(Default)]
pub(super) struct PendingRuntimePersistence {
    pub(super) pending: bool,
    pub(super) error_reported: bool,
}

impl PendingRuntimePersistence {
    pub(super) fn failed(&mut self) -> bool {
        self.pending = true;
        let report = !self.error_reported;
        self.error_reported = true;
        report
    }

    pub(super) fn resolved(&mut self) {
        self.pending = false;
        self.error_reported = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::profile::{PortForwardKind, PortForwardRule};

    fn profile() -> SessionProfile {
        let mut profile = SessionProfile::blank("live", 1);
        profile.port_forwarding_rules.push(PortForwardRule {
            id: "rule".into(),
            label: "rule".into(),
            kind: PortForwardKind::Local,
            listen_host: "127.0.0.1".into(),
            listen_port: 8080,
            target_host: "127.0.0.1".into(),
            target_port: 80,
            enabled: true,
        });
        profile
    }

    #[test]
    fn failed_save_preserves_committed_flag_and_can_recover_without_runtime_change() {
        let runtime = PortForwardManagerSnapshot::default();
        let committed = vec![profile()];
        let mut pending = PendingRuntimePersistence::default();
        let failure = persisted_runtime_candidate(&committed, &runtime, |_| {
            anyhow::bail!("storage unavailable")
        });
        assert!(failure.is_err());
        assert!(pending.failed());
        assert!(!pending.failed(), "one visible error per pending episode");
        assert!(committed[0].port_forwarding_rules[0].enabled);

        let recovered = persisted_runtime_candidate(&committed, &runtime, |_| Ok(()))
            .unwrap()
            .unwrap();
        pending.resolved();
        assert!(!recovered[0].port_forwarding_rules[0].enabled);
        assert!(!pending.pending);
    }

    #[test]
    fn retry_rebases_on_latest_edits_and_sync_loaded_data() {
        let runtime = PortForwardManagerSnapshot::default();
        let mut latest = profile();
        latest.name = "another window's edit".into();
        latest.remote_path_favorites.push("/latest".into());
        latest.port_forwarding_rules[0].target_port = 443;
        let adopted = persisted_runtime_candidate(&[latest.clone()], &runtime, |saved| {
            assert_eq!(saved[0].name, latest.name);
            assert_eq!(saved[0].remote_path_favorites, latest.remote_path_favorites);
            assert_eq!(saved[0].port_forwarding_rules[0].target_port, 443);
            Ok(())
        })
        .unwrap()
        .unwrap();
        assert!(!adopted[0].port_forwarding_rules[0].enabled);
    }

    #[test]
    fn retry_backoff_stays_bounded() {
        assert_eq!(retry_delay(0), std::time::Duration::from_millis(250));
        assert_eq!(retry_delay(1), std::time::Duration::from_secs(1));
        assert_eq!(retry_delay(usize::MAX), std::time::Duration::from_secs(30));
    }

    #[test]
    fn unchanged_runtime_does_not_write_or_publish() {
        let runtime = PortForwardManagerSnapshot::default();
        let mut committed = profile();
        committed.port_forwarding_rules[0].enabled = false;
        let result = persisted_runtime_candidate(&[committed], &runtime, |_| {
            panic!("unchanged observations must not write")
        })
        .unwrap();
        assert!(result.is_none());
    }
}
