package dev.akko.miaominal.ui.theme

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import org.junit.Assert.*
import org.junit.Test

class UiPreferencesPolicyTest {
    @Test fun missingPreferencesUsePlatformDefaultsAndIndependentTerminalSize() {
        assertEquals(UiContrast.Standard, parseUiContrast(null, 33))
        assertEquals(UiContrast.System, parseUiContrast(null, 34))
        assertEquals(UiContrast.System, parseUiContrast("invalid", 35))
        assertEquals(UiContrast.High, parseUiContrast("High", 33))
        assertEquals(1f, terminalFontScale(false, 2f)); assertEquals(2f, terminalFontScale(true, 2f))
    }
    private fun contrast(a: Color, b: Color): Double =
        (maxOf(a.luminance(), b.luminance()) + 0.05).toDouble() / (minOf(a.luminance(), b.luminance()) + 0.05)

    @Test fun generatedSchemesMeetTextAndEssentialGraphicContrast() {
        listOf(0xFF2F6FED.toInt(), 0xFFB3261E.toInt(), 0xFF34A853.toInt(), 0xFF000000.toInt()).forEach { seed ->
            listOf(false, true).forEach { dark -> listOf(0.0, 0.5, 1.0).forEach { level ->
                val s = if (level == 0.0) seededColorScheme(seed, dark) else contrastColorScheme(seed, dark, level)
                listOf(s.onSurface to s.surface, s.onSurfaceVariant to s.surfaceContainerHighest,
                    s.onPrimary to s.primary, s.onPrimaryContainer to s.primaryContainer,
                    s.onError to s.error, s.onErrorContainer to s.errorContainer).forEach { (text, background) ->
                    assertTrue("seed=$seed dark=$dark level=$level contrast=${contrast(text, background)}", contrast(text, background) >= 4.5)
                }
                assertTrue(contrast(s.outline, s.surface) >= 3.0)
            } }
        }
    }
}
