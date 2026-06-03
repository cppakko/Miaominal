use gpui::{Background, Bounds, Hsla, Pixels, Point, Window, fill, px, size};
use miaominal_terminal::TerminalSnapshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineStyle {
    Light,
    Heavy,
    Double,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BoxGlyph {
    left: Option<LineStyle>,
    right: Option<LineStyle>,
    up: Option<LineStyle>,
    down: Option<LineStyle>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum BlockGlyph {
    Full,
    Upper(f32),
    Lower(f32),
    Left(f32),
    Right(f32),
    Shade(f32),
    Quadrants {
        upper_left: bool,
        upper_right: bool,
        lower_left: bool,
        lower_right: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum CustomGlyph {
    Box(BoxGlyph),
    Block(BlockGlyph),
}

pub(super) fn is_custom_glyph(ch: char) -> bool {
    CustomGlyph::from_char(ch).is_some()
}

pub(super) fn paint_custom_glyphs(
    snapshot: &TerminalSnapshot,
    origin: Point<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    window: &mut Window,
) {
    let scale = window.scale_factor();

    for (row, cells) in snapshot.cells.iter().enumerate() {
        for (col, cell) in cells.iter().enumerate() {
            if cell.spacer || cell.character == '\0' {
                continue;
            }

            let Some(glyph) = CustomGlyph::from_char(cell.character) else {
                continue;
            };

            let bounds = cell_bounds(origin, cell_width, line_height, col, row, scale);
            match glyph {
                CustomGlyph::Box(glyph) => paint_box_glyph(glyph, bounds, cell.fg, scale, window),
                CustomGlyph::Block(glyph) => {
                    paint_block_glyph(glyph, bounds, cell.fg, scale, window)
                }
            }
        }
    }
}

impl CustomGlyph {
    fn from_char(ch: char) -> Option<Self> {
        box_glyph(ch)
            .map(CustomGlyph::Box)
            .or_else(|| block_glyph(ch).map(CustomGlyph::Block))
    }
}

fn box_glyph(ch: char) -> Option<BoxGlyph> {
    use LineStyle::{Double, Heavy, Light};

    let glyph = |left, right, up, down| {
        Some(BoxGlyph {
            left,
            right,
            up,
            down,
        })
    };
    let l = Some(Light);
    let h = Some(Heavy);
    let d = Some(Double);

    match ch {
        '\u{2500}' => glyph(l, l, None, None),
        '\u{2501}' => glyph(h, h, None, None),
        '\u{2502}' => glyph(None, None, l, l),
        '\u{2503}' => glyph(None, None, h, h),
        '\u{250C}' | '\u{256D}' => glyph(None, l, None, l),
        '\u{2510}' | '\u{256E}' => glyph(l, None, None, l),
        '\u{2514}' | '\u{2570}' => glyph(None, l, l, None),
        '\u{2518}' | '\u{256F}' => glyph(l, None, l, None),
        '\u{250F}' => glyph(None, h, None, h),
        '\u{2513}' => glyph(h, None, None, h),
        '\u{2517}' => glyph(None, h, h, None),
        '\u{251B}' => glyph(h, None, h, None),
        '\u{251C}' => glyph(None, l, l, l),
        '\u{2524}' => glyph(l, None, l, l),
        '\u{252C}' => glyph(l, l, None, l),
        '\u{2534}' => glyph(l, l, l, None),
        '\u{253C}' => glyph(l, l, l, l),
        '\u{2523}' => glyph(None, h, h, h),
        '\u{252B}' => glyph(h, None, h, h),
        '\u{2533}' => glyph(h, h, None, h),
        '\u{253B}' => glyph(h, h, h, None),
        '\u{254B}' => glyph(h, h, h, h),
        '\u{2550}' => glyph(d, d, None, None),
        '\u{2551}' => glyph(None, None, d, d),
        '\u{2554}' => glyph(None, d, None, d),
        '\u{2557}' => glyph(d, None, None, d),
        '\u{255A}' => glyph(None, d, d, None),
        '\u{255D}' => glyph(d, None, d, None),
        '\u{2560}' => glyph(None, d, d, d),
        '\u{2563}' => glyph(d, None, d, d),
        '\u{2566}' => glyph(d, d, None, d),
        '\u{2569}' => glyph(d, d, d, None),
        '\u{256C}' => glyph(d, d, d, d),
        '\u{2574}' => glyph(l, None, None, None),
        '\u{2575}' => glyph(None, None, l, None),
        '\u{2576}' => glyph(None, l, None, None),
        '\u{2577}' => glyph(None, None, None, l),
        '\u{2578}' => glyph(h, None, None, None),
        '\u{2579}' => glyph(None, None, h, None),
        '\u{257A}' => glyph(None, h, None, None),
        '\u{257B}' => glyph(None, None, None, h),
        '\u{257C}' => glyph(h, l, None, None),
        '\u{257D}' => glyph(None, None, h, l),
        '\u{257E}' => glyph(l, h, None, None),
        '\u{257F}' => glyph(None, None, l, h),
        _ => None,
    }
}

fn block_glyph(ch: char) -> Option<BlockGlyph> {
    match ch {
        '\u{2580}' => Some(BlockGlyph::Upper(0.5)),
        '\u{2581}' => Some(BlockGlyph::Lower(0.125)),
        '\u{2582}' => Some(BlockGlyph::Lower(0.25)),
        '\u{2583}' => Some(BlockGlyph::Lower(0.375)),
        '\u{2584}' => Some(BlockGlyph::Lower(0.5)),
        '\u{2585}' => Some(BlockGlyph::Lower(0.625)),
        '\u{2586}' => Some(BlockGlyph::Lower(0.75)),
        '\u{2587}' => Some(BlockGlyph::Lower(0.875)),
        '\u{2588}' => Some(BlockGlyph::Full),
        '\u{2589}' => Some(BlockGlyph::Left(0.875)),
        '\u{258A}' => Some(BlockGlyph::Left(0.75)),
        '\u{258B}' => Some(BlockGlyph::Left(0.625)),
        '\u{258C}' => Some(BlockGlyph::Left(0.5)),
        '\u{258D}' => Some(BlockGlyph::Left(0.375)),
        '\u{258E}' => Some(BlockGlyph::Left(0.25)),
        '\u{258F}' => Some(BlockGlyph::Left(0.125)),
        '\u{2590}' => Some(BlockGlyph::Right(0.5)),
        '\u{2591}' => Some(BlockGlyph::Shade(0.25)),
        '\u{2592}' => Some(BlockGlyph::Shade(0.5)),
        '\u{2593}' => Some(BlockGlyph::Shade(0.75)),
        '\u{2594}' => Some(BlockGlyph::Upper(0.125)),
        '\u{2595}' => Some(BlockGlyph::Right(0.125)),
        '\u{2596}' => Some(quadrants(false, false, true, false)),
        '\u{2597}' => Some(quadrants(false, false, false, true)),
        '\u{2598}' => Some(quadrants(true, false, false, false)),
        '\u{2599}' => Some(quadrants(true, false, true, true)),
        '\u{259A}' => Some(quadrants(true, false, false, true)),
        '\u{259B}' => Some(quadrants(true, true, true, false)),
        '\u{259C}' => Some(quadrants(true, true, false, true)),
        '\u{259D}' => Some(quadrants(false, true, false, false)),
        '\u{259E}' => Some(quadrants(false, true, true, false)),
        '\u{259F}' => Some(quadrants(false, true, true, true)),
        _ => None,
    }
}

fn quadrants(
    upper_left: bool,
    upper_right: bool,
    lower_left: bool,
    lower_right: bool,
) -> BlockGlyph {
    BlockGlyph::Quadrants {
        upper_left,
        upper_right,
        lower_left,
        lower_right,
    }
}

fn paint_box_glyph(
    glyph: BoxGlyph,
    bounds: Bounds<Pixels>,
    color: Hsla,
    scale: f32,
    window: &mut Window,
) {
    let left = bounds.origin.x;
    let top = bounds.origin.y;
    let right = bounds.origin.x + bounds.size.width;
    let bottom = bounds.origin.y + bounds.size.height;
    let center_x = snap_to_physical(left + bounds.size.width * 0.5, scale);
    let center_y = snap_to_physical(top + bounds.size.height * 0.5, scale);

    if let Some(style) = glyph.left {
        paint_horizontal(left, center_x, center_y, style, color, scale, window);
    }
    if let Some(style) = glyph.right {
        paint_horizontal(center_x, right, center_y, style, color, scale, window);
    }
    if let Some(style) = glyph.up {
        paint_vertical(center_x, top, center_y, style, color, scale, window);
    }
    if let Some(style) = glyph.down {
        paint_vertical(center_x, center_y, bottom, style, color, scale, window);
    }
}

fn paint_horizontal(
    left: Pixels,
    right: Pixels,
    center_y: Pixels,
    style: LineStyle,
    color: Hsla,
    scale: f32,
    window: &mut Window,
) {
    match style {
        LineStyle::Light => paint_centered_rect_h(
            left,
            right,
            center_y,
            device_pixels(1.0, scale),
            color,
            scale,
            window,
        ),
        LineStyle::Heavy => paint_centered_rect_h(
            left,
            right,
            center_y,
            device_pixels(2.0, scale),
            color,
            scale,
            window,
        ),
        LineStyle::Double => {
            let width = device_pixels(1.0, scale);
            paint_centered_rect_h(left, right, center_y - width, width, color, scale, window);
            paint_centered_rect_h(left, right, center_y + width, width, color, scale, window);
        }
    }
}

fn paint_vertical(
    center_x: Pixels,
    top: Pixels,
    bottom: Pixels,
    style: LineStyle,
    color: Hsla,
    scale: f32,
    window: &mut Window,
) {
    match style {
        LineStyle::Light => paint_centered_rect_v(
            center_x,
            top,
            bottom,
            device_pixels(1.0, scale),
            color,
            scale,
            window,
        ),
        LineStyle::Heavy => paint_centered_rect_v(
            center_x,
            top,
            bottom,
            device_pixels(2.0, scale),
            color,
            scale,
            window,
        ),
        LineStyle::Double => {
            let width = device_pixels(1.0, scale);
            paint_centered_rect_v(center_x - width, top, bottom, width, color, scale, window);
            paint_centered_rect_v(center_x + width, top, bottom, width, color, scale, window);
        }
    }
}

fn paint_centered_rect_h(
    left: Pixels,
    right: Pixels,
    center_y: Pixels,
    height: Pixels,
    color: Hsla,
    scale: f32,
    window: &mut Window,
) {
    paint_rect(
        snap_to_physical(left, scale),
        snap_to_physical(center_y - height * 0.5, scale),
        snap_to_physical(right, scale),
        snap_to_physical(center_y + height * 0.5, scale),
        color,
        window,
    );
}

fn paint_centered_rect_v(
    center_x: Pixels,
    top: Pixels,
    bottom: Pixels,
    width: Pixels,
    color: Hsla,
    scale: f32,
    window: &mut Window,
) {
    paint_rect(
        snap_to_physical(center_x - width * 0.5, scale),
        snap_to_physical(top, scale),
        snap_to_physical(center_x + width * 0.5, scale),
        snap_to_physical(bottom, scale),
        color,
        window,
    );
}

fn paint_block_glyph(
    glyph: BlockGlyph,
    bounds: Bounds<Pixels>,
    color: Hsla,
    scale: f32,
    window: &mut Window,
) {
    let left = bounds.origin.x;
    let top = bounds.origin.y;
    let right = bounds.origin.x + bounds.size.width;
    let bottom = bounds.origin.y + bounds.size.height;
    let width = bounds.size.width;
    let height = bounds.size.height;

    match glyph {
        BlockGlyph::Full => paint_rect(left, top, right, bottom, color, window),
        BlockGlyph::Upper(fraction) => {
            paint_rect(
                left,
                top,
                right,
                snap_to_physical(top + height * fraction, scale),
                color,
                window,
            );
        }
        BlockGlyph::Lower(fraction) => {
            paint_rect(
                left,
                snap_to_physical(bottom - height * fraction, scale),
                right,
                bottom,
                color,
                window,
            );
        }
        BlockGlyph::Left(fraction) => {
            paint_rect(
                left,
                top,
                snap_to_physical(left + width * fraction, scale),
                bottom,
                color,
                window,
            );
        }
        BlockGlyph::Right(fraction) => {
            paint_rect(
                snap_to_physical(right - width * fraction, scale),
                top,
                right,
                bottom,
                color,
                window,
            );
        }
        BlockGlyph::Shade(alpha) => {
            let mut shade = color;
            shade.a *= alpha;
            paint_rect(left, top, right, bottom, shade, window);
        }
        BlockGlyph::Quadrants {
            upper_left,
            upper_right,
            lower_left,
            lower_right,
        } => {
            let mid_x = snap_to_physical(left + width * 0.5, scale);
            let mid_y = snap_to_physical(top + height * 0.5, scale);
            if upper_left {
                paint_rect(left, top, mid_x, mid_y, color, window);
            }
            if upper_right {
                paint_rect(mid_x, top, right, mid_y, color, window);
            }
            if lower_left {
                paint_rect(left, mid_y, mid_x, bottom, color, window);
            }
            if lower_right {
                paint_rect(mid_x, mid_y, right, bottom, color, window);
            }
        }
    }
}

fn paint_rect(
    left: Pixels,
    top: Pixels,
    right: Pixels,
    bottom: Pixels,
    color: Hsla,
    window: &mut Window,
) {
    if right <= left || bottom <= top {
        return;
    }

    window.paint_quad(fill(
        Bounds {
            origin: Point { x: left, y: top },
            size: size(right - left, bottom - top),
        },
        Background::from(color),
    ));
}

fn cell_bounds(
    origin: Point<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    col: usize,
    row: usize,
    scale: f32,
) -> Bounds<Pixels> {
    let left = snap_to_physical(origin.x + cell_width * col as f32, scale);
    let right = snap_to_physical(origin.x + cell_width * (col + 1) as f32, scale);
    let top = snap_to_physical(origin.y + line_height * row as f32, scale);
    let bottom = snap_to_physical(origin.y + line_height * (row + 1) as f32, scale);
    Bounds {
        origin: Point { x: left, y: top },
        size: size(right - left, bottom - top),
    }
}

fn snap_to_physical(value: Pixels, scale: f32) -> Pixels {
    if scale > 0.0 {
        px((f32::from(value) * scale).round() / scale)
    } else {
        value
    }
}

fn device_pixels(value: f32, scale: f32) -> Pixels {
    if scale > 0.0 {
        px(value / scale)
    } else {
        px(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_glyphs_cover_common_box_drawing_and_blocks() {
        assert!(is_custom_glyph('\u{2502}'));
        assert!(is_custom_glyph('\u{250C}'));
        assert!(is_custom_glyph('\u{2551}'));
        assert!(is_custom_glyph('\u{2588}'));
        assert!(is_custom_glyph('\u{2596}'));
        assert!(!is_custom_glyph('a'));
    }

    #[test]
    fn box_glyph_profiles_keep_directional_connections() {
        use LineStyle::{Double, Light};

        assert_eq!(
            box_glyph('\u{250C}'),
            Some(BoxGlyph {
                left: None,
                right: Some(Light),
                up: None,
                down: Some(Light),
            })
        );
        assert_eq!(
            box_glyph('\u{2551}'),
            Some(BoxGlyph {
                left: None,
                right: None,
                up: Some(Double),
                down: Some(Double),
            })
        );
    }

    #[test]
    fn block_glyph_profiles_cover_fractional_cells() {
        assert_eq!(block_glyph('\u{2584}'), Some(BlockGlyph::Lower(0.5)));
        assert_eq!(block_glyph('\u{2590}'), Some(BlockGlyph::Right(0.5)));
        assert_eq!(
            block_glyph('\u{2596}'),
            Some(BlockGlyph::Quadrants {
                upper_left: false,
                upper_right: false,
                lower_left: true,
                lower_right: false,
            })
        );
    }
}
