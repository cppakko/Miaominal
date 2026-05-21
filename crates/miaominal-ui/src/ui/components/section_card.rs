use crate::ui::assets::AppIcon;
use gpui::*;
use gpui_component::*;

#[derive(IntoElement)]
pub struct SectionCard {
    icon: AppIcon,
    title: SharedString,
    content: AnyElement,
}

impl SectionCard {
    pub fn new(icon: AppIcon, title: impl Into<SharedString>, content: impl IntoElement) -> Self {
        Self {
            icon,
            title: title.into(),
            content: content.into_any_element(),
        }
    }
}

impl RenderOnce for SectionCard {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let roles = miaominal_settings::current_theme().material.roles;

        div()
            .w_full()
            .rounded(px(20.0))
            .bg(rgb(roles.surface_container_highest))
            .p_4()
            .child(
                v_flex()
                    .gap_4()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .size(px(32.0))
                                    .rounded(px(10.0))
                                    .bg(rgb(roles.surface_container_low))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(rgb(roles.on_surface_variant))
                                    .child(Icon::new(self.icon).small()),
                            )
                            .child(
                                div()
                                    .text_size(miaominal_settings::scaled_font_size(14.0))
                                    .text_color(rgb(roles.on_surface))
                                    .child(self.title),
                            ),
                    )
                    .child(self.content),
            )
    }
}
