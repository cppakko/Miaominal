//! Platform-neutral terminal colors.
//!
//! Snapshots use this type instead of a UI-specific color so the terminal core
//! can run on Android without a desktop toolkit. The component values match the
//! sRGB byte values divided by 255, which is what the desktop renderer used to
//! convert from before painting.

/// An RGBA color with components in the `0.0..=1.0` range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerminalColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl TerminalColor {
    pub const BLACK: Self = Self::new(0.0, 0.0, 0.0, 1.0);
    pub const WHITE: Self = Self::new(1.0, 1.0, 1.0, 1.0);

    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Build an opaque color from the `0xRRGGBB` representation used by themes.
    pub fn from_hex(hex: u32) -> Self {
        let [_, r, g, b] = hex.to_be_bytes();
        Self::from_rgb8(r, g, b)
    }

    pub fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        Self {
            r: f32::from(r) / 255.0,
            g: f32::from(g) / 255.0,
            b: f32::from(b) / 255.0,
            a: 1.0,
        }
    }

    pub const fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// Component-wise blend towards `target`; the alpha of `self` is kept.
    ///
    /// This mirrors the previous desktop implementation, which mixed the two
    /// colors in RGB space.
    pub(crate) fn mix(self, target: Self, amount: f32) -> Self {
        Self {
            r: self.r + (target.r - self.r) * amount,
            g: self.g + (target.g - self.g) * amount,
            b: self.b + (target.b - self.b) * amount,
            a: self.a,
        }
    }

    /// Scale the RGB channels, used for ANSI dim colors.
    pub(crate) fn dimmed(self, factor: f32) -> Self {
        Self {
            r: self.r * factor,
            g: self.g * factor,
            b: self.b * factor,
            a: self.a,
        }
    }
}
