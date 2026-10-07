package dev.akko.miaominal.ui.theme

import androidx.compose.material3.ColorScheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.ui.graphics.Color
import com.google.android.material.color.utilities.CorePalette
import com.google.android.material.color.utilities.TonalPalette

fun seededColorScheme(seedColor: Int, darkTheme: Boolean): ColorScheme {
    val palette = CorePalette.of(seedColor)
    return if (darkTheme) palette.darkScheme() else palette.lightScheme()
}

private fun CorePalette.lightScheme() = lightColorScheme(
    primary = a1.at(40),
    onPrimary = a1.at(100),
    primaryContainer = a1.at(90),
    onPrimaryContainer = a1.at(10),
    inversePrimary = a1.at(80),
    secondary = a2.at(40),
    onSecondary = a2.at(100),
    secondaryContainer = a2.at(90),
    onSecondaryContainer = a2.at(10),
    tertiary = a3.at(40),
    onTertiary = a3.at(100),
    tertiaryContainer = a3.at(90),
    onTertiaryContainer = a3.at(10),
    background = n1.at(99),
    onBackground = n1.at(10),
    surface = n1.at(99),
    onSurface = n1.at(10),
    surfaceVariant = n2.at(90),
    onSurfaceVariant = n2.at(30),
    surfaceTint = a1.at(40),
    inverseSurface = n1.at(20),
    inverseOnSurface = n1.at(95),
    error = error.at(40),
    onError = error.at(100),
    errorContainer = error.at(90),
    onErrorContainer = error.at(10),
    outline = n2.at(50),
    outlineVariant = n2.at(80),
    scrim = n1.at(0),
    surfaceBright = n1.at(98),
    surfaceDim = n1.at(87),
    surfaceContainerLowest = n1.at(100),
    surfaceContainerLow = n1.at(96),
    surfaceContainer = n1.at(94),
    surfaceContainerHigh = n1.at(92),
    surfaceContainerHighest = n1.at(90),
    primaryFixed = a1.at(90),
    primaryFixedDim = a1.at(80),
    onPrimaryFixed = a1.at(10),
    onPrimaryFixedVariant = a1.at(30),
    secondaryFixed = a2.at(90),
    secondaryFixedDim = a2.at(80),
    onSecondaryFixed = a2.at(10),
    onSecondaryFixedVariant = a2.at(30),
    tertiaryFixed = a3.at(90),
    tertiaryFixedDim = a3.at(80),
    onTertiaryFixed = a3.at(10),
    onTertiaryFixedVariant = a3.at(30)
)

private fun CorePalette.darkScheme() = darkColorScheme(
    primary = a1.at(80),
    onPrimary = a1.at(20),
    primaryContainer = a1.at(30),
    onPrimaryContainer = a1.at(90),
    inversePrimary = a1.at(40),
    secondary = a2.at(80),
    onSecondary = a2.at(20),
    secondaryContainer = a2.at(30),
    onSecondaryContainer = a2.at(90),
    tertiary = a3.at(80),
    onTertiary = a3.at(20),
    tertiaryContainer = a3.at(30),
    onTertiaryContainer = a3.at(90),
    background = n1.at(6),
    onBackground = n1.at(90),
    surface = n1.at(6),
    onSurface = n1.at(90),
    surfaceVariant = n2.at(30),
    onSurfaceVariant = n2.at(80),
    surfaceTint = a1.at(80),
    inverseSurface = n1.at(90),
    inverseOnSurface = n1.at(20),
    error = error.at(80),
    onError = error.at(20),
    errorContainer = error.at(30),
    onErrorContainer = error.at(90),
    outline = n2.at(60),
    outlineVariant = n2.at(30),
    scrim = n1.at(0),
    surfaceBright = n1.at(24),
    surfaceDim = n1.at(6),
    surfaceContainerLowest = n1.at(4),
    surfaceContainerLow = n1.at(10),
    surfaceContainer = n1.at(12),
    surfaceContainerHigh = n1.at(17),
    surfaceContainerHighest = n1.at(22),
    primaryFixed = a1.at(90),
    primaryFixedDim = a1.at(80),
    onPrimaryFixed = a1.at(10),
    onPrimaryFixedVariant = a1.at(30),
    secondaryFixed = a2.at(90),
    secondaryFixedDim = a2.at(80),
    onSecondaryFixed = a2.at(10),
    onSecondaryFixedVariant = a2.at(30),
    tertiaryFixed = a3.at(90),
    tertiaryFixedDim = a3.at(80),
    onTertiaryFixed = a3.at(10),
    onTertiaryFixedVariant = a3.at(30)
)

private fun TonalPalette.at(tone: Int) = Color(this.tone(tone))
