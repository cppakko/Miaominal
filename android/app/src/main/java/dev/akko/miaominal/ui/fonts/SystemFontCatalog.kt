package dev.akko.miaominal.ui.fonts

import android.graphics.Paint
import android.graphics.Typeface
import android.graphics.fonts.Font
import android.graphics.fonts.SystemFonts
import kotlin.math.abs

internal data class SystemFontFamilyOption(
    val familyName: String,
    val monospace: Boolean,
    val cjk: Boolean
)

internal fun loadSystemFontFamilies(): List<SystemFontFamilyOption> {
    val families = linkedMapOf<String, SystemFontFamilyOption>()

    SystemFonts.getAvailableFonts().forEach { font ->
        val fileName = font.file?.name ?: return@forEach
        val familyName = fontFamilyNameFromFile(fileName)
        if (familyName.isBlank()) return@forEach

        val key = familyName.lowercase()
        val current = families[key]
        val monospace = fileName.contains("mono", ignoreCase = true) || font.isMonospace()
        val cjk = font.isCjkFont(fileName)
        families[key] = SystemFontFamilyOption(
            familyName = current?.familyName ?: familyName,
            monospace = current?.monospace == true || monospace,
            cjk = current?.cjk == true || cjk
        )
    }

    return families.values.sortedWith(
        compareBy(String.CASE_INSENSITIVE_ORDER) { it.familyName }
    )
}

internal fun fontFamilyNameFromFile(fileName: String): String {
    val stem = fileName.substringBeforeLast('.')
        .substringBefore('[')
        .replace(fontStyleSuffix, "")

    return stem
        .replace(Regex("(?<=[a-z0-9])(?=[A-Z])"), " ")
        .replace('-', ' ')
        .replace('_', ' ')
        .replace(Regex("\\s+"), " ")
        .trim()
}

private fun Font.isMonospace(): Boolean {
    val fontFile = file ?: return false
    return runCatching {
        val typeface = Typeface.Builder(fontFile)
            .setTtcIndex(ttcIndex)
            .build()
        val paint = Paint().apply {
            this.typeface = typeface
            textSize = 100f
        }
        abs(paint.measureText("iiiiiiii") - paint.measureText("WWWWWWWW")) < 0.5f
    }.getOrDefault(false)
}

private fun Font.isCjkFont(fileName: String): Boolean {
    val languageTags = localeList.toLanguageTags().lowercase()
    if (languageTags.split(',').any { tag ->
            tag.startsWith("zh") || tag.startsWith("ja") || tag.startsWith("ko")
        }
    ) {
        return true
    }

    val normalizedName = fileName.lowercase()
    return listOf("cjk", "hans", "hant", "chinese", "japanese", "korean", "fallback")
        .any(normalizedName::contains)
}

private val fontStyleSuffix = Regex(
    pattern = "(?:[-_ ]?(?:thin|extralight|extra-light|light|regular|medium|semibold|semi-bold|" +
        "demibold|bold|extrabold|extra-bold|black|italic|oblique|bolditalic|bold-italic|" +
        "condensed|variable))+$",
    option = RegexOption.IGNORE_CASE
)
