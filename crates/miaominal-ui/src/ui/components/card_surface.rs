use gpui::{Div, Styled, div, px, rgb};

pub(crate) fn card_surface(background: u32, radius: f32) -> Div {
    div().rounded(px(radius)).bg(rgb(background))
}
