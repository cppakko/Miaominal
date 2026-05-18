use crate::ui::theme::MaterialTheme;

#[derive(Clone, Copy)]
pub(in crate::ui::shell) struct GroupAccentPalette {
    pub(in crate::ui::shell) accent: u32,
    pub(in crate::ui::shell) on_accent: u32,
    pub(in crate::ui::shell) accent_container: u32,
    pub(in crate::ui::shell) on_accent_container: u32,
}

/// Hash the name so filtering and reordering do not change its accent palette.
pub(in crate::ui::shell) fn group_accent_palette(
    name: &str,
    material: &MaterialTheme,
) -> GroupAccentPalette {
    let slot = fnv1a_slot(name);
    let roles = material.roles;

    match slot {
        0 => GroupAccentPalette {
            accent: roles.primary,
            on_accent: roles.on_primary,
            accent_container: roles.primary_container,
            on_accent_container: roles.on_primary_container,
        },
        1 => GroupAccentPalette {
            accent: roles.secondary,
            on_accent: roles.on_secondary,
            accent_container: roles.secondary_container,
            on_accent_container: roles.on_secondary_container,
        },
        2 => GroupAccentPalette {
            accent: roles.tertiary,
            on_accent: roles.on_tertiary,
            accent_container: roles.tertiary_container,
            on_accent_container: roles.on_tertiary_container,
        },
        3 => GroupAccentPalette {
            accent: material.extended.info.color,
            on_accent: material.extended.info.on_color,
            accent_container: material.extended.info.color_container,
            on_accent_container: material.extended.info.on_color_container,
        },
        _ => GroupAccentPalette {
            accent: material.extended.warning.color,
            on_accent: material.extended.warning.on_color,
            accent_container: material.extended.warning.color_container,
            on_accent_container: material.extended.warning.on_color_container,
        },
    }
}

fn fnv1a_slot(name: &str) -> usize {
    let mut hash: u64 = 14695981039346656037;
    for byte in name.bytes() {
        let b = if byte.is_ascii_uppercase() {
            byte + 32
        } else {
            byte
        };
        if b.is_ascii_alphanumeric() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(1099511628211);
        }
    }
    (hash % 5) as usize
}
