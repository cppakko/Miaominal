//! Approval modes, the approval matrix (design §10.2) and in-memory session
//! grants (design §10.3).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::AgentRiskLevel;

/// How much the agent may do without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    /// Read-only tools only; side-effecting tools are not offered.
    ReadOnly,
    /// Every non-read-only action needs confirmation.
    Cautious,
    /// Low-risk modifications run automatically; service/system and
    /// destructive actions need confirmation.
    #[default]
    Standard,
    /// Everything up to L2 runs automatically; L3 only with
    /// [`ApprovalSettings::auto_approve_dangerous`].
    FullAuto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ApprovalSettings {
    pub mode: ApprovalMode,
    /// Separate, explicitly warned switch: in FullAuto, also run L3
    /// without asking. Ignored in every other mode.
    #[serde(default)]
    pub auto_approve_dangerous: bool,
}

impl From<miaominal_settings::AiAgentMode> for ApprovalMode {
    /// Migration of the pre-v2 agent modes (design §10.2).
    fn from(mode: miaominal_settings::AiAgentMode) -> Self {
        use miaominal_settings::AiAgentMode;
        match mode {
            AiAgentMode::Ask => Self::ReadOnly,
            AiAgentMode::Execute => Self::Standard,
            AiAgentMode::NonBlocking => Self::Cautious,
            AiAgentMode::FullAuto => Self::FullAuto,
        }
    }
}

impl From<ApprovalMode> for miaominal_settings::AiAgentMode {
    fn from(mode: ApprovalMode) -> Self {
        match mode {
            ApprovalMode::ReadOnly => Self::Ask,
            ApprovalMode::Standard => Self::Execute,
            ApprovalMode::Cautious => Self::NonBlocking,
            ApprovalMode::FullAuto => Self::FullAuto,
        }
    }
}

impl ApprovalSettings {
    pub fn from_settings(settings: &miaominal_settings::AppSettings) -> Self {
        Self {
            mode: settings.agent_mode.into(),
            auto_approve_dangerous: settings.agent_auto_approve_dangerous,
        }
    }
}

/// The policy outcome for one tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Ask { reason: String },
    Deny { reason: String },
}

impl ApprovalSettings {
    /// Apply the approval matrix to an assessed risk.
    pub fn verdict(&self, risk: AgentRiskLevel, reason: &str) -> Verdict {
        use AgentRiskLevel::*;
        let ask = || Verdict::Ask {
            reason: reason.to_string(),
        };
        match (risk, self.mode) {
            (L4Forbidden, _) => Verdict::Deny {
                reason: format!("forbidden: {reason}"),
            },
            (L0ReadOnly, _) => Verdict::Allow,
            (_, ApprovalMode::ReadOnly) => Verdict::Deny {
                reason: format!(
                    "read-only mode does not allow changes ({reason}); describe the change to the user instead"
                ),
            },
            (_, ApprovalMode::Cautious) => ask(),
            (L1LowMutation, _) => Verdict::Allow,
            (L2ServiceImpacting, ApprovalMode::Standard) => ask(),
            (L2ServiceImpacting, ApprovalMode::FullAuto) => Verdict::Allow,
            (L3Dangerous, ApprovalMode::FullAuto) if self.auto_approve_dangerous => Verdict::Allow,
            (L3Dangerous, _) => ask(),
        }
    }
}

/// Command-prefix grants scoped to one target and one session. Never
/// persisted.
#[derive(Debug, Clone, Default)]
pub struct SessionGrants {
    grants: HashSet<(String, String)>,
}

impl SessionGrants {
    pub fn grant(&mut self, target: &str, prefix: &str) {
        self.grants
            .insert((target.to_string(), normalize_prefix(prefix)));
    }

    pub fn is_granted(&self, target: &str, prefix: Option<&str>) -> bool {
        let Some(prefix) = prefix else {
            return false;
        };
        self.grants
            .contains(&(target.to_string(), normalize_prefix(prefix)))
    }

    pub fn clear(&mut self) {
        self.grants.clear();
    }

    pub fn len(&self) -> usize {
        self.grants.len()
    }

    pub fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }
}

fn normalize_prefix(prefix: &str) -> String {
    prefix
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use AgentRiskLevel::*;

    fn settings(mode: ApprovalMode) -> ApprovalSettings {
        ApprovalSettings {
            mode,
            auto_approve_dangerous: false,
        }
    }

    #[test]
    fn approval_matrix_matches_design() {
        use ApprovalMode::*;
        let allow = |m, r| settings(m).verdict(r, "x") == Verdict::Allow;
        let ask = |m, r| matches!(settings(m).verdict(r, "x"), Verdict::Ask { .. });
        let deny = |m, r| matches!(settings(m).verdict(r, "x"), Verdict::Deny { .. });

        for mode in [ReadOnly, Cautious, Standard, FullAuto] {
            assert!(allow(mode, L0ReadOnly));
            assert!(deny(mode, L4Forbidden));
        }
        assert!(deny(ReadOnly, L1LowMutation));
        assert!(ask(Cautious, L1LowMutation));
        assert!(ask(Cautious, L2ServiceImpacting));
        assert!(ask(Cautious, L3Dangerous));
        assert!(allow(Standard, L1LowMutation));
        assert!(ask(Standard, L2ServiceImpacting));
        assert!(ask(Standard, L3Dangerous));
        assert!(allow(FullAuto, L2ServiceImpacting));
        assert!(ask(FullAuto, L3Dangerous));

        let dangerous = ApprovalSettings {
            mode: FullAuto,
            auto_approve_dangerous: true,
        };
        assert_eq!(dangerous.verdict(L3Dangerous, "x"), Verdict::Allow);
        let ignored = ApprovalSettings {
            mode: Standard,
            auto_approve_dangerous: true,
        };
        assert!(matches!(
            ignored.verdict(L3Dangerous, "x"),
            Verdict::Ask { .. }
        ));
    }

    #[test]
    fn legacy_modes_migrate_per_design() {
        use miaominal_settings::AiAgentMode;
        assert_eq!(ApprovalMode::from(AiAgentMode::Ask), ApprovalMode::ReadOnly);
        assert_eq!(
            ApprovalMode::from(AiAgentMode::Execute),
            ApprovalMode::Standard
        );
        assert_eq!(
            ApprovalMode::from(AiAgentMode::NonBlocking),
            ApprovalMode::Cautious
        );
        assert_eq!(
            ApprovalMode::from(AiAgentMode::FullAuto),
            ApprovalMode::FullAuto
        );
        for mode in [
            ApprovalMode::ReadOnly,
            ApprovalMode::Cautious,
            ApprovalMode::Standard,
            ApprovalMode::FullAuto,
        ] {
            assert_eq!(ApprovalMode::from(AiAgentMode::from(mode)), mode);
        }
        let parsed: AiAgentMode = serde_json::from_str("\"standard\"").unwrap();
        assert_eq!(parsed, AiAgentMode::Execute);
        assert_eq!(serde_json::to_string(&parsed).unwrap(), "\"execute\"");
    }

    #[test]
    fn grants_are_scoped_to_target_and_prefix() {
        let mut grants = SessionGrants::default();
        grants.grant("prod-web", "systemctl  Restart");
        assert!(grants.is_granted("prod-web", Some("systemctl restart")));
        assert!(!grants.is_granted("prod-db", Some("systemctl restart")));
        assert!(!grants.is_granted("prod-web", Some("systemctl stop")));
        assert!(!grants.is_granted("prod-web", None));
    }
}
