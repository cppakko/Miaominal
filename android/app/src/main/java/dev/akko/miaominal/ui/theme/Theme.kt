package dev.akko.miaominal.ui.theme

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.graphics.toArgb

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalContext

@Composable
fun MiaominalTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    dynamicColor: Boolean = true,
    seedColor: Int = DEFAULT_THEME_SEED,
    content: @Composable () -> Unit
) {
    val uiPreferences = LocalUiPreferences.current ?: rememberUiPreferences(LocalContext.current)
    val baseScheme = when {
        dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> {
            val context = LocalContext.current
            if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        }

        else -> seededColorScheme(seedColor = seedColor, darkTheme = darkTheme)
    }

    val level = uiPreferences.contrastLevel
    val colorScheme = if (level == 0.0) baseScheme else contrastColorScheme(
        if (dynamicColor) baseScheme.primary.toArgb() else seedColor, darkTheme, level)
    CompositionLocalProvider(LocalUiPreferences provides uiPreferences) {
    MaterialTheme(
        colorScheme = colorScheme,
        typography = Typography,
        shapes = Shapes,
        content = content
    )
    }
}
