use gpui_kit::component::select::{Select, SelectDelegate, SelectState};
use gpui_kit::{Rems, Styled, px, rems, rgb};

/// Default minimum and maximum widths (in interface-scaled `rem`) for
/// standalone selects placed in auto-sized horizontal slots such as settings.
pub(crate) const MD3_SELECT_MIN_WIDTH_REMS: f32 = 11.5;
pub(crate) const MD3_SELECT_MAX_WIDTH_REMS: f32 = 20.0;

pub(crate) fn md3_select<D>(state: &gpui_kit::Entity<SelectState<D>>) -> Select<D>
where
    D: SelectDelegate + 'static,
{
    let roles = miaominal_settings::current_theme().material.roles;

    Select::new(state)
        .appearance(false)
        .rounded(px(14.0))
        .bg(rgb(roles.surface_container_highest))
        .text_color(rgb(roles.on_surface))
}

/// Computes an interface-font-scaled width (`rems`) that comfortably fits the
/// widest option label (including trigger/list horizontal padding and trailing
/// caret/check icon) so choosing a short option never shrinks the trigger or
/// truncates longer options inside the dropdown popup.
pub(crate) fn md3_select_options_width<S: AsRef<str>>(
    labels: impl IntoIterator<Item = S>,
    min_rems: f32,
    max_rems: f32,
) -> Rems {
    let widest_label_rems = labels
        .into_iter()
        .map(|label| estimate_option_label_width_rems(label.as_ref()))
        .fold(0.0_f32, f32::max);
    // Account for Large/Medium select horizontal padding, trailing caret/check
    // icon + gap, and dropdown list outer padding.
    let total_rems = (widest_label_rems + 5.25).clamp(min_rems, max_rems);
    rems(total_rems)
}

/// Builds an `md3_select` whose trigger width and dropdown `menu_width` are
/// both sized to fit the widest option label in `labels`.
pub(crate) fn md3_select_for_options<D, S: AsRef<str>>(
    state: &gpui_kit::Entity<SelectState<D>>,
    labels: impl IntoIterator<Item = S>,
) -> Select<D>
where
    D: SelectDelegate + 'static,
{
    let width =
        md3_select_options_width(labels, MD3_SELECT_MIN_WIDTH_REMS, MD3_SELECT_MAX_WIDTH_REMS);
    md3_select(state).w(width).menu_width(width)
}

fn estimate_option_label_width_rems(label: &str) -> f32 {
    label
        .chars()
        .map(|ch| {
            if ch.is_ascii() {
                if matches!(
                    ch,
                    'i' | 'l' | 'I' | '.' | ',' | ':' | ';' | '!' | '|' | ' '
                ) {
                    0.36
                } else if ch.is_ascii_uppercase() || matches!(ch, 'm' | 'w' | 'M' | 'W') {
                    0.68
                } else {
                    0.58
                }
            } else {
                1.05
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_width_accommodates_longest_label_even_with_short_entries() {
        let schemes = [
            "跟随主题",
            "Dracula",
            "Nord",
            "Catppuccin Mocha",
            "Solarized Dark",
            "Tokyo Night",
            "One Dark",
        ];
        let scheme_width = md3_select_options_width(
            schemes,
            MD3_SELECT_MIN_WIDTH_REMS,
            MD3_SELECT_MAX_WIDTH_REMS,
        );
        let short_only = md3_select_options_width(
            ["Nord"],
            MD3_SELECT_MIN_WIDTH_REMS,
            MD3_SELECT_MAX_WIDTH_REMS,
        );

        assert!(scheme_width.0 > short_only.0);
        assert!(scheme_width.0 >= 14.0);
        assert!(scheme_width.0 <= MD3_SELECT_MAX_WIDTH_REMS);
    }
}
