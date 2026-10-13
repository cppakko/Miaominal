use gpui_kit::App;
use miaominal_core::profile::PortForwardRule;
use std::rc::Rc;

/// Access to runtime resources; rule drafts and catalog persistence stay in Forwarding.
pub(crate) trait ForwardingRuntime {
    fn has_rule_session(&self, profile_id: &str, rule_id: &str, cx: &App) -> bool;
    fn online_session_count_for_profile(&self, profile_id: &str, cx: &App) -> usize;
    fn sync_rules(&self, profile_id: &str, rules: &[PortForwardRule], cx: &mut App) -> usize;
    fn retire_rule_session(&self, profile_id: &str, rule_id: &str, cx: &mut App) -> bool;
}

#[derive(Clone)]
pub(crate) struct ForwardingRuntimePort(Rc<dyn ForwardingRuntime>);
impl ForwardingRuntimePort {
    pub(crate) fn new(runtime: Rc<dyn ForwardingRuntime>) -> Self {
        Self(runtime)
    }
    pub(crate) fn has_rule_session(&self, profile_id: &str, rule_id: &str, cx: &App) -> bool {
        self.0.has_rule_session(profile_id, rule_id, cx)
    }
    pub(crate) fn online_session_count_for_profile(&self, profile_id: &str, cx: &App) -> usize {
        self.0.online_session_count_for_profile(profile_id, cx)
    }
    pub(crate) fn sync_rules(
        &self,
        profile_id: &str,
        rules: &[PortForwardRule],
        cx: &mut App,
    ) -> usize {
        self.0.sync_rules(profile_id, rules, cx)
    }
    pub(crate) fn retire_rule_session(
        &self,
        profile_id: &str,
        rule_id: &str,
        cx: &mut App,
    ) -> bool {
        self.0.retire_rule_session(profile_id, rule_id, cx)
    }
}
