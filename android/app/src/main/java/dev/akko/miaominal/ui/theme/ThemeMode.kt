package dev.akko.miaominal.ui.theme

const val DEFAULT_THEME_SEED: Int = 0xFF2F6FED.toInt()

enum class ThemeMode {
    System,
    Light,
    Dark;

    companion object {
        fun fromStoredValue(value: String?): ThemeMode =
            entries.firstOrNull { it.name == value } ?: System
    }
}
