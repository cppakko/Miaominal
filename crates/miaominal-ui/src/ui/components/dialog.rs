use crate::ui::assets::AppIcon;
use gpui::{
    AnyElement, FontWeight, IntoElement as _, ParentElement as _, SharedString, Styled, div,
    prelude::FluentBuilder as _, px, rgb,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{Icon, v_flex};

const BASIC_DIALOG_MAX_WIDTH: f32 = 560.0;
const BASIC_DIALOG_RADIUS: f32 = 28.0;
const BOTTOM_POPUP_MAX_WIDTH: f32 = 680.0;
const BOTTOM_POPUP_RADIUS: f32 = 32.0;
const BOTTOM_POPUP_MIN_HEIGHT: f32 = 560.0;

#[derive(Clone, Copy)]
pub(crate) enum BasicDialogHeaderAlignment {
    Start,
    Center,
}

#[derive(Clone, Copy)]
pub(crate) struct BasicDialogIcon {
    pub icon: AppIcon,
    pub tint: u32,
}

#[derive(Clone, Copy)]
pub(crate) enum BasicDialogActionTone {
    Default,
    Destructive,
}

pub(crate) fn basic_dialog_panel(
    title: String,
    supporting_text: Option<String>,
    body: Option<AnyElement>,
    actions: AnyElement,
    icon: Option<BasicDialogIcon>,
    header_alignment: BasicDialogHeaderAlignment,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let centered_header = matches!(header_alignment, BasicDialogHeaderAlignment::Center);

    let title = div()
        .w_full()
        .min_w(px(0.0))
        .text_size(miaominal_settings::FontSize::DisplayLarge.scaled())
        .line_height(miaominal_settings::scaled_line_height(28.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(roles.on_surface))
        .when(centered_header, |this| this.text_center())
        .child(title);

    let supporting_text = supporting_text.map(|supporting_text| {
        div()
            .w_full()
            .min_w(px(0.0))
            .text_size(miaominal_settings::FontSize::Heading.scaled())
            .line_height(miaominal_settings::scaled_line_height(20.0))
            .text_color(rgb(roles.on_surface_variant))
            .when(centered_header, |this| this.text_center())
            .child(supporting_text)
            .into_any_element()
    });

    let mut header = v_flex().w_full().min_w(px(0.0)).gap_4();
    if centered_header {
        header = header.items_center();
    }
    if let Some(icon) = icon {
        header = header.child(
            div()
                .size(px(52.0))
                .rounded(px(16.0))
                .bg(dialog_color_with_alpha(icon.tint, 0x28))
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(icon.tint))
                .child(Icon::new(icon.icon).size(px(24.0))),
        );
    }
    header = header
        .child(title)
        .when_some(supporting_text, |this, supporting_text| {
            this.child(supporting_text)
        });

    let mut content = v_flex().w_full().min_w(px(0.0)).gap_4().child(header);
    if let Some(body) = body {
        content = content.child(body);
    }

    div()
        .w_full()
        .px_4()
        .child(
            v_flex()
                .w_full()
                .min_w(px(0.0))
                .max_w(px(BASIC_DIALOG_MAX_WIDTH))
                .mx_auto()
                .p_6()
                .gap_6()
                .rounded(px(BASIC_DIALOG_RADIUS))
                .bg(rgb(roles.surface_container_high))
                .child(content)
                .child(div().w_full().child(actions)),
        )
        .into_any_element()
}

pub(crate) fn bottom_popup_panel(
    title: String,
    supporting_text: Option<String>,
    body: Option<AnyElement>,
    actions: AnyElement,
    viewport_height: f32,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let compact_panel_height = viewport_height.max(1.0);
    let use_compact_layout = compact_panel_height < BOTTOM_POPUP_MIN_HEIGHT;

    let title = div()
        .w_full()
        .min_w(px(0.0))
        .text_size(miaominal_settings::FontSize::DisplayLarge.scaled())
        .line_height(miaominal_settings::scaled_line_height(28.0))
        .text_color(rgb(roles.on_surface))
        .child(title);

    let supporting_text = supporting_text.map(|supporting_text| {
        div()
            .w_full()
            .min_w(px(0.0))
            .text_size(miaominal_settings::FontSize::Heading.scaled())
            .line_height(miaominal_settings::scaled_line_height(20.0))
            .text_color(rgb(roles.on_surface_variant))
            .child(supporting_text)
            .into_any_element()
    });

    let header = v_flex()
        .w_full()
        .min_w(px(0.0))
        .gap_5()
        .child(
            div().w_full().flex().justify_center().child(
                div()
                    .w(px(56.0))
                    .h(px(5.0))
                    .rounded_full()
                    .bg(dialog_color_with_alpha(roles.on_surface_variant, 0x34)),
            ),
        )
        .child(
            v_flex()
                .w_full()
                .min_w(px(0.0))
                .gap_3()
                .child(title)
                .when_some(supporting_text, |this, supporting_text| {
                    this.child(supporting_text)
                }),
        );

    let mut panel = v_flex()
        .w_full()
        .min_w(px(0.0))
        .max_w(px(BOTTOM_POPUP_MAX_WIDTH))
        .mx_auto()
        .px_6()
        .pt_6()
        .pb_8()
        .gap_6()
        .rounded_t(px(BOTTOM_POPUP_RADIUS))
        .bg(rgb(roles.surface_container_highest))
        .when(use_compact_layout, |this| this.h(px(compact_panel_height)))
        .when(!use_compact_layout, |this| {
            this.min_h(px(BOTTOM_POPUP_MIN_HEIGHT))
        })
        .child(header);

    if let Some(body) = body {
        panel = panel.child(
            div()
                .flex_1()
                .w_full()
                .min_h_0()
                .child(div().size_full().overflow_y_scrollbar().child(body)),
        );
    }

    panel = panel.child(div().w_full().child(actions));

    div()
        .w_full()
        .px_4()
        .child(div().relative().w_full().min_w(px(0.0)).child(panel))
        .into_any_element()
}

pub(crate) fn basic_dialog_action_button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    tone: BasicDialogActionTone,
) -> Button {
    let roles = miaominal_settings::current_theme().material.roles;
    let text_color = match tone {
        BasicDialogActionTone::Default => roles.primary,
        BasicDialogActionTone::Destructive => roles.error,
    };

    Button::new(id.into())
        .ghost()
        .border_0()
        .rounded(px(20.0))
        .text_color(rgb(text_color))
        .label(label.into())
}

fn dialog_color_with_alpha(color: u32, alpha: u8) -> gpui::Rgba {
    gpui::rgba(((color & 0x00ff_ffff) << 8) | alpha as u32)
}
