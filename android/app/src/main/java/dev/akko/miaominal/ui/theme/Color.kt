package dev.akko.miaominal.ui.theme

import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.ui.graphics.Color

/**
 * Brand seed color.
 *
 * This is the only configurable color knob in the system. Changing it requires
 * regenerating the 49-role M3 palette tokens (DESIGN.md is the source of truth
 * — do not hand-tune individual roles after editing the seed).
 */
val BrandSeed = Color(DEFAULT_THEME_SEED)

// --- Dark scheme (primary surface) — full 49-role M3 palette ---
//
// Derived from BrandSeed through M3 Dynamic Color (Tonal Spot variant).
// Source: DESIGN.md frontmatter `colors` block.

val DarkColorScheme = darkColorScheme(
    primary = Color(0xFFB1C5FF),
    onPrimary = Color(0xFF172E60),
    primaryContainer = Color(0xFF304578),
    onPrimaryContainer = Color(0xFFDAE2FF),
    inversePrimary = Color(0xFF485D92),
    secondary = Color(0xFFC0C6DD),
    onSecondary = Color(0xFF2A3042),
    secondaryContainer = Color(0xFF404659),
    onSecondaryContainer = Color(0xFFDCE2F9),
    tertiary = Color(0xFFE1BBDD),
    onTertiary = Color(0xFF412742),
    tertiaryContainer = Color(0xFF5A3D59),
    onTertiaryContainer = Color(0xFFFED7F9),
    background = Color(0xFF121318),
    onBackground = Color(0xFFE2E2E9),
    surface = Color(0xFF121318),
    onSurface = Color(0xFFE2E2E9),
    surfaceVariant = Color(0xFF44464F),
    onSurfaceVariant = Color(0xFFC5C6D0),
    surfaceTint = Color(0xFFB1C5FF),
    inverseSurface = Color(0xFFE2E2E9),
    inverseOnSurface = Color(0xFF2F3036),
    error = Color(0xFFFFB4AB),
    onError = Color(0xFF690005),
    errorContainer = Color(0xFF93000A),
    onErrorContainer = Color(0xFFFFDAD6),
    outline = Color(0xFF8F9099),
    outlineVariant = Color(0xFF44464F),
    scrim = Color(0xFF000000),
    surfaceBright = Color(0xFF38393F),
    surfaceDim = Color(0xFF121318),
    surfaceContainerLowest = Color(0xFF0D0E13),
    surfaceContainerLow = Color(0xFF1A1B21),
    surfaceContainer = Color(0xFF1E1F25),
    surfaceContainerHigh = Color(0xFF282A2F),
    surfaceContainerHighest = Color(0xFF33343A),
    primaryFixed = Color(0xFFDAE2FF),
    primaryFixedDim = Color(0xFFB1C5FF),
    onPrimaryFixed = Color(0xFF001847),
    onPrimaryFixedVariant = Color(0xFF304578),
    secondaryFixed = Color(0xFFDCE2F9),
    secondaryFixedDim = Color(0xFFC0C6DD),
    onSecondaryFixed = Color(0xFF151B2C),
    onSecondaryFixedVariant = Color(0xFF404659),
    tertiaryFixed = Color(0xFFFED7F9),
    tertiaryFixedDim = Color(0xFFE1BBDD),
    onTertiaryFixed = Color(0xFF2B122C),
    onTertiaryFixedVariant = Color(0xFF5A3D59)
)

// --- Light scheme — full 49-role M3 palette (opposite of dark) ---

val LightColorScheme = lightColorScheme(
    primary = Color(0xFF485D92),
    onPrimary = Color(0xFFFFFFFF),
    primaryContainer = Color(0xFFDAE2FF),
    onPrimaryContainer = Color(0xFF001847),
    inversePrimary = Color(0xFFB1C5FF),
    secondary = Color(0xFF585E71),
    onSecondary = Color(0xFFFFFFFF),
    secondaryContainer = Color(0xFFDCE2F9),
    onSecondaryContainer = Color(0xFF151B2C),
    tertiary = Color(0xFF735472),
    onTertiary = Color(0xFFFFFFFF),
    tertiaryContainer = Color(0xFFFED7F9),
    onTertiaryContainer = Color(0xFF2B122C),
    background = Color(0xFFFAF8FF),
    onBackground = Color(0xFF1A1B21),
    surface = Color(0xFFFAF8FF),
    onSurface = Color(0xFF1A1B21),
    surfaceVariant = Color(0xFFE1E2EC),
    onSurfaceVariant = Color(0xFF44464F),
    surfaceTint = Color(0xFF485D92),
    inverseSurface = Color(0xFF2F3036),
    inverseOnSurface = Color(0xFFF1F0F7),
    error = Color(0xFFBA1A1A),
    onError = Color(0xFFFFFFFF),
    errorContainer = Color(0xFFFFDAD6),
    onErrorContainer = Color(0xFF410002),
    outline = Color(0xFF757780),
    outlineVariant = Color(0xFFC5C6D0),
    scrim = Color(0xFF000000),
    surfaceBright = Color(0xFFFAF8FF),
    surfaceDim = Color(0xFFDAD9E0),
    surfaceContainerLowest = Color(0xFFFFFFFF),
    surfaceContainerLow = Color(0xFFF4F3FA),
    surfaceContainer = Color(0xFFEEEDF4),
    surfaceContainerHigh = Color(0xFFE8E7EF),
    surfaceContainerHighest = Color(0xFFE2E2E9),
    primaryFixed = Color(0xFFDAE2FF),
    primaryFixedDim = Color(0xFFB1C5FF),
    onPrimaryFixed = Color(0xFF001847),
    onPrimaryFixedVariant = Color(0xFF304578),
    secondaryFixed = Color(0xFFDCE2F9),
    secondaryFixedDim = Color(0xFFC0C6DD),
    onSecondaryFixed = Color(0xFF151B2C),
    onSecondaryFixedVariant = Color(0xFF404659),
    tertiaryFixed = Color(0xFFFED7F9),
    tertiaryFixedDim = Color(0xFFE1BBDD),
    onTertiaryFixed = Color(0xFF2B122C),
    onTertiaryFixedVariant = Color(0xFF5A3D59)
)

// --- Domain colors (intentionally outside the 49-role theme system) ---
//
// Reserved for the terminal window and the online status dot only. They must
// never migrate into chrome, buttons, or surfaces — see DESIGN.md "Colors".

val TerminalBackground = Color(0xFF0B1020)
val TerminalForeground = Color(0xFFDCE6FF)
val TerminalAccent = Color(0xFF7DD3FC)
val StatusOnline = Color(0xFF2E9B63)
