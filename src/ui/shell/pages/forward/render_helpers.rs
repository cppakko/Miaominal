use super::super::super::*;
use super::utils::rule_summary;
use crate::ui::i18n;

pub(super) fn build_forward_rule_context_menu(
    menu: PopupMenu,
    entity: Entity<AppView>,
    profile_id: String,
    rule_id: String,
    connected: bool,
) -> PopupMenu {
    let connect_entity = entity.clone();
    let edit_entity = entity.clone();
    let duplicate_entity = entity.clone();
    let remove_entity = entity;
    let connect_profile_id = profile_id.clone();
    let connect_rule_id = rule_id.clone();
    let edit_profile_id = profile_id.clone();
    let edit_rule_id = rule_id.clone();
    let duplicate_profile_id = profile_id.clone();
    let duplicate_rule_id = rule_id.clone();
    let remove_profile_id = profile_id;
    let remove_rule_id = rule_id;

    menu.item(
        PopupMenuItem::new(if connected {
            i18n::string("forwarding.menu.disconnect")
        } else {
            i18n::string("forwarding.menu.connect")
        })
        .on_click(move |_, _window, cx| {
            let entity = connect_entity.clone();
            let profile_id = connect_profile_id.clone();
            let rule_id = connect_rule_id.clone();
            entity.update(cx, |this, cx| {
                if connected {
                    this.disconnect_port_forward_rule(&profile_id, &rule_id, cx);
                } else {
                    this.connect_port_forward_rule(&profile_id, &rule_id, cx);
                }
            });
        }),
    )
    .item(
        PopupMenuItem::new(i18n::string("forwarding.menu.edit")).on_click(move |_, window, cx| {
            let entity = edit_entity.clone();
            let profile_id = edit_profile_id.clone();
            let rule_id = edit_rule_id.clone();
            entity.update(cx, |this, cx| {
                this.edit_port_forward_rule(profile_id.clone(), rule_id.clone(), window, cx);
            });
        }),
    )
    .item(
        PopupMenuItem::new(i18n::string("forwarding.menu.duplicate")).on_click(move |_, _, cx| {
            let entity = duplicate_entity.clone();
            let profile_id = duplicate_profile_id.clone();
            let rule_id = duplicate_rule_id.clone();
            entity.update(cx, |this, cx| {
                this.duplicate_port_forward_rule(&profile_id, &rule_id, cx);
            });
        }),
    )
    .item(PopupMenuItem::separator())
    .item(
        PopupMenuItem::new(i18n::string("forwarding.menu.remove")).on_click(move |_, _, cx| {
            let entity = remove_entity.clone();
            let profile_id = remove_profile_id.clone();
            let rule_id = remove_rule_id.clone();
            entity.update(cx, |this, cx| {
                this.request_port_forward_rule_removal(&profile_id, &rule_id, cx);
            });
        }),
    )
}

fn forward_rule_custom_label(rule: &PortForwardRule) -> Option<String> {
    let summary = rule_summary(rule);
    let label = rule.label.trim();

    if label.is_empty() || label == summary {
        None
    } else {
        Some(label.to_string())
    }
}

pub(super) fn forward_rule_display_label(rule: &PortForwardRule) -> String {
    forward_rule_custom_label(rule).unwrap_or_else(|| rule_summary(rule))
}

pub(super) fn truncate_with_ellipsis(value: &str, max_chars: usize) -> String {
    let chars: Vec<_> = value.chars().collect();
    if chars.len() <= max_chars {
        return value.to_string();
    }

    let visible: String = chars
        .into_iter()
        .take(max_chars.saturating_sub(3))
        .collect();
    format!("{visible}...")
}

pub(super) fn route_direction_icon(kind: PortForwardKind) -> AppIcon {
    match kind {
        PortForwardKind::Local => AppIcon::Upload,
        PortForwardKind::Remote => AppIcon::Download,
    }
}

pub(super) fn render_forward_endpoint_editor(
    title: impl Into<SharedString>,
    host_input: Entity<InputState>,
    port_input: Entity<InputState>,
) -> impl IntoElement {
    let title = title.into();
    let roles = settings::current_theme().material.roles;

    v_flex()
        .w_full()
        .min_w(px(0.0))
        .gap_3()
        .child(
            div()
                .text_size(settings::scaled_font_size(11.0))
                .text_color(rgb(roles.on_surface_variant))
                .child(title),
        )
        .child(
            h_flex()
                .w_full()
                .min_w(px(0.0))
                .gap_2()
                .flex_wrap()
                .items_start()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_1()
                        .child(
                            div()
                                .text_size(settings::scaled_font_size(11.0))
                                .text_color(rgb(roles.on_surface_variant))
                                .child(i18n::string("forwarding.fields.host")),
                        )
                        .child(surface_text_input(&host_input, TextInputSurface::Low).large()),
                )
                .child(
                    v_flex()
                        .w(px(120.0))
                        .min_w(px(120.0))
                        .flex_shrink_0()
                        .gap_1()
                        .child(
                            div()
                                .text_size(settings::scaled_font_size(11.0))
                                .text_color(rgb(roles.on_surface_variant))
                                .child(i18n::string("forwarding.fields.port")),
                        )
                        .child(surface_text_input(&port_input, TextInputSurface::Low).large()),
                ),
        )
}
