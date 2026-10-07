package dev.akko.miaominal.ui.theme

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class SyncActionColorsTest {
    @Test fun downloadRemainsReadableOnItsOwnOpaqueSurfaceAcrossThemes() {
        listOf(0xFF2F6FED.toInt(), 0xFFB3261E.toInt(), 0xFF34A853.toInt(), 0xFF000000.toInt()).forEach { seed ->
            listOf(false, true).forEach { dark -> listOf(0.0, 0.5, 1.0).forEach { level ->
                val scheme = if (level == 0.0) seededColorScheme(seed, dark) else contrastColorScheme(seed, dark, level)
                val colors = syncDownloadButtonColors(scheme)
                listOf(colors.contentColor to colors.containerColor,
                    colors.disabledContentColor to colors.disabledContainerColor).forEach { (text, background) ->
                    assertEquals(1f, background.alpha, 0f)
                    assertEquals(1f, text.alpha, 0f)
                    val ratio = contrast(text, background)
                    assertTrue("seed=$seed dark=$dark level=$level contrast=$ratio", ratio >= 4.5)
                }
            } }
        }
    }

    private fun contrast(a: Color, b: Color): Double =
        (maxOf(a.luminance(), b.luminance()) + 0.05).toDouble() / (minOf(a.luminance(), b.luminance()) + 0.05)
}
