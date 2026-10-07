package dev.akko.miaominal.ui.theme

import org.junit.Assert.assertNotEquals
import org.junit.Test

class SeedColorSchemeTest {
    @Test
    fun differentSeedsProduceDifferentPrimaryColors() {
        val blue = seededColorScheme(0xFF2F6FED.toInt(), darkTheme = false)
        val red = seededColorScheme(0xFFB3261E.toInt(), darkTheme = false)

        assertNotEquals(blue.primary, red.primary)
    }

    @Test
    fun lightAndDarkSchemesUseDifferentSurfaceTones() {
        val light = seededColorScheme(DEFAULT_THEME_SEED, darkTheme = false)
        val dark = seededColorScheme(DEFAULT_THEME_SEED, darkTheme = true)

        assertNotEquals(light.surface, dark.surface)
        assertNotEquals(light.onSurface, dark.onSurface)
    }
}
