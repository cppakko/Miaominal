use super::super::*;
use crate::ui::i18n;

pub(in crate::ui::shell::layout) fn render_session_agent_token_usage(
    agent: &SessionAgentState,
    settings_store: &SettingsStore,
    text_muted: u32,
) -> gpui::AnyElement {
    let Some(usage) = &agent.last_usage else {
        return div().into_any_element();
    };
    if usage.input_tokens == 0 && usage.output_tokens == 0 {
        return div().into_any_element();
    }

    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;

    let provider_context_window = settings_store
        .settings()
        .selected_ai_provider_id
        .as_ref()
        .and_then(|id| {
            settings_store
                .settings()
                .ai_providers
                .iter()
                .find(|p| &p.id == id)
        })
        .and_then(|p| p.context_window);

    let total_tokens = usage.input_tokens + usage.output_tokens;

    // Determine text color based on context window usage
    let (text_color, bg_color) = if let Some(max) = provider_context_window {
        if max > 0 {
            let usage_pct = usage.input_tokens as f64 / max as f64;
            if usage_pct >= 0.9 {
                // Critical: red
                (roles.on_error_container, Some(roles.error_container))
            } else if usage_pct >= 0.7 {
                // Warning: yellow/amber
                (roles.on_tertiary_container, Some(roles.tertiary_container))
            } else {
                // Normal: muted
                (text_muted, None)
            }
        } else {
            (text_muted, None)
        }
    } else {
        (text_muted, None)
    };

    let text = match provider_context_window {
        Some(max) if max > 0 => {
            let pct = usage.input_tokens as f64 / max as f64 * 100.0;
            format!(
                "↑{} ↓{} / {} ({:.0}%)",
                format_token_count(usage.input_tokens),
                format_token_count(usage.output_tokens),
                format_token_count(max),
                pct
            )
        }
        _ => format!(
            "↑{} ↓{} ({})",
            format_token_count(usage.input_tokens),
            format_token_count(usage.output_tokens),
            format_token_count(total_tokens)
        ),
    };

    let mut container = div()
        .text_size(miaominal_settings::FontSize::Body.scaled())
        .text_color(rgb(text_color))
        .child(text);

    // Add background if warning/error
    if let Some(bg) = bg_color {
        container = container.bg(rgb(bg)).px_2().py(px(2.0)).rounded(px(4.0));
    }

    container.into_any_element()
}

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

pub(in crate::ui::shell::layout) fn format_token_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
