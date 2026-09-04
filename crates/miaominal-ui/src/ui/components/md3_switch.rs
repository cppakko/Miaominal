use gpui_kit::component::switch::Switch;
use gpui_kit::{Styled, rgb};

pub(crate) fn md3_switch(id: impl Into<gpui_kit::ElementId>) -> Switch {
    let material = miaominal_settings::current_theme().material;
    let (switch_color, _border_color) = if material.dark {
        (rgb(material.roles.primary), rgb(material.roles.outline))
    } else {
        (rgb(material.roles.primary), rgb(material.roles.primary))
    };

    Switch::new(id).color(switch_color).rounded_3xl()
}
