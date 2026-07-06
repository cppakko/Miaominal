use super::super::*;
use crate::ui::i18n;

pub(in crate::ui::shell::layout) fn format_relative_chat_time(timestamp: i64) -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(timestamp);
    let elapsed = now.saturating_sub(timestamp).max(0);

    if elapsed < 60 {
        i18n::string("workspace.panel.agent.time.just_now")
    } else if elapsed < 3_600 {
        i18n::string_args(
            "workspace.panel.agent.time.minutes_ago",
            &[("count", &(elapsed / 60).to_string())],
        )
    } else if elapsed < 86_400 {
        i18n::string_args(
            "workspace.panel.agent.time.hours_ago",
            &[("count", &(elapsed / 3_600).to_string())],
        )
    } else if elapsed < 604_800 {
        i18n::string_args(
            "workspace.panel.agent.time.days_ago",
            &[("count", &(elapsed / 86_400).to_string())],
        )
    } else {
        format_local_timestamp(Some(
            SystemTime::UNIX_EPOCH + Duration::from_secs(timestamp.max(0) as u64),
        ))
        .to_string()
    }
}

pub(in crate::ui::shell::layout) fn format_duration_ms(ms: u128) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else {
        let seconds = ms as f64 / 1_000.0;
        format!("{seconds:.1}s")
    }
}

pub(in crate::ui::shell::layout) fn estimate_session_agent_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    chars.saturating_add(3) / 4
}
