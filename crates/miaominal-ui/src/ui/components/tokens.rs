//! Shared conversion at the existing product-theme color boundary.

pub(in crate::ui) fn color_with_alpha(color: u32, alpha: u8) -> gpui_kit::Rgba {
    gpui_kit::rgba(((color & 0x00ff_ffff) << 8) | alpha as u32)
}
