package dev.akko.miaominal.ui.screens

import androidx.annotation.StringRes
import dev.akko.miaominal.R
import dev.akko.miaominal.data.DefaultTerminalQuickKeys

internal data class TerminalKey(
    val label: String,
    val value: String,
    @param:StringRes val description: Int
)

internal val AllTerminalKeys = listOf(
    TerminalKey("Esc", "ESC", R.string.term_key_escape),
    TerminalKey("Tab", "TAB", R.string.term_key_tab),
    TerminalKey("^C", "INTERRUPT", R.string.term_key_interrupt),
    TerminalKey("←", "LEFT", R.string.term_key_left),
    TerminalKey("↑", "UP", R.string.term_key_history_previous),
    TerminalKey("↓", "DOWN", R.string.term_key_history_next),
    TerminalKey("→", "RIGHT", R.string.term_key_right),
    TerminalKey("Home", "HOME", R.string.term_key_home),
    TerminalKey("End", "END", R.string.term_key_end),
    TerminalKey("PgUp", "PAGEUP", R.string.term_key_page_up),
    TerminalKey("PgDn", "PAGEDOWN", R.string.term_key_page_down),
    TerminalKey("^U", "ERASE", R.string.term_key_erase),
    TerminalKey("^L", "CLEAR", R.string.term_key_clear),
    TerminalKey("/", "/", R.string.term_key_slash),
    TerminalKey("|", "|", R.string.term_key_pipe),
    TerminalKey("~", "~", R.string.term_key_tilde),
    TerminalKey("-", "-", R.string.term_key_dash),
    TerminalKey("*", "*", R.string.term_key_star),
    TerminalKey("!", "!", R.string.term_key_bang),
    TerminalKey("A−", "SMALLER", R.string.term_font_smaller),
    TerminalKey("A+", "LARGER", R.string.term_font_larger)
)

internal fun normalizeTerminalQuickKeys(values: List<String>): List<String> {
    val supported = AllTerminalKeys.mapTo(mutableSetOf(), TerminalKey::value)
    return values.filter { it in supported }.distinct().ifEmpty { DefaultTerminalQuickKeys }
}

