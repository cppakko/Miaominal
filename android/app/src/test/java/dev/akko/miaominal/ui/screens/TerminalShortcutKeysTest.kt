package dev.akko.miaominal.ui.screens

import dev.akko.miaominal.data.DefaultTerminalQuickKeys
import org.junit.Assert.assertEquals
import org.junit.Test

class TerminalShortcutKeysTest {
    @Test
    fun preservesSupportedOrderAndRemovesDuplicates() {
        assertEquals(
            listOf("TAB", "ESC", "HOME"),
            normalizeTerminalQuickKeys(listOf("TAB", "unknown", "ESC", "TAB", "HOME"))
        )
    }

    @Test
    fun fallsBackToDefaultsWhenNoSupportedKeysRemain() {
        assertEquals(DefaultTerminalQuickKeys, normalizeTerminalQuickKeys(listOf("unknown")))
    }
}

