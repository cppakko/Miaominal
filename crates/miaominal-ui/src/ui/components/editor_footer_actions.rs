use crate::ui::assets::AppIcon;
use crate::ui::components::shell_tokens::{ShellColors, ShellMetrics, ShellTypography, space};
use gpui_kit::component::{
    Icon, Sizable as _,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    h_flex,
};
use gpui_kit::{
    AnyElement, App, Div, ElementId, FontWeight, ParentElement, SharedString, Styled, Window, div,
};

pub(crate) const EDITOR_FOOTER_ACTION_HEIGHT: f32 = 32.0;

#[derive(Clone, Copy)]
pub(crate) enum EditorFooterActionTone {
    Text,
    Primary,
    Destructive,
}

pub(crate) fn editor_footer_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Option<AppIcon>,
    tone: EditorFooterActionTone,
    cx: &App,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    let colors = ShellColors::current();
    let primary = matches!(tone, EditorFooterActionTone::Primary);
    let foreground = match tone {
        EditorFooterActionTone::Primary => colors.on_primary,
        EditorFooterActionTone::Text => colors.primary,
        EditorFooterActionTone::Destructive => colors.danger,
    };
    let background = if primary {
        colors.primary
    } else {
        foreground.opacity(0.0)
    };
    let label = label.into();
    let mut content = h_flex()
        .gap(space(8.0))
        .text_size(space(ShellTypography::BODY_SIZE))
        .line_height(space(ShellTypography::BODY_LINE_HEIGHT));
    if let Some(icon) = icon {
        content = content.child(Icon::new(icon).size(space(ShellMetrics::EDITOR_ACTION_ICON_SIZE)));
    }

    Button::new(id)
        .large()
        .custom(
            ButtonCustomVariant::new(cx)
                .color(background)
                .foreground(foreground)
                .hover(if primary {
                    background.blend(foreground.opacity(0.08))
                } else {
                    foreground.opacity(0.08)
                })
                .active(if primary {
                    background.blend(foreground.opacity(0.12))
                } else {
                    foreground.opacity(0.12)
                }),
        )
        .h(space(ShellMetrics::EDITOR_ACTION_HEIGHT))
        .px(space(if primary { 20.0 } else { 16.0 }))
        .rounded(miaominal_settings::scaled_font_size(
            ShellMetrics::EDITOR_ACTION_RADIUS,
        ))
        .border_0()
        .bg(background)
        .text_color(foreground)
        .font_weight(FontWeight::MEDIUM)
        .accessibility_label(label.clone())
        .child(content.child(label))
        .on_click(move |_, window, cx| on_click(window, cx))
}

pub(crate) fn editor_footer_actions(actions: impl IntoIterator<Item = AnyElement>) -> Div {
    div()
        .w_full()
        .flex()
        .items_center()
        .justify_end()
        .gap(space(8.0))
        .flex_wrap()
        .children(actions)
}
