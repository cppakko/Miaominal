use miaominal_core::profile::PortForwardRule;

pub(in crate::ui::features::forwarding) fn rule_summary(rule: &PortForwardRule) -> String {
    format!(
        "{}:{} -> {}:{}",
        rule.listen_host, rule.listen_port, rule.target_host, rule.target_port
    )
}
