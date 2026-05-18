use super::super::super::super::*;

pub(in crate::ui::shell::pages::forward) fn rule_summary(rule: &PortForwardRule) -> String {
    format!(
        "{}:{} -> {}:{}",
        rule.listen_host, rule.listen_port, rule.target_host, rule.target_port
    )
}
