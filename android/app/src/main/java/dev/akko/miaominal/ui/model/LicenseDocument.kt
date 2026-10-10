package dev.akko.miaominal.ui.model

import androidx.annotation.StringRes
import dev.akko.miaominal.R

internal enum class LicenseDocument(
    @param:StringRes val titleRes: Int,
    @param:StringRes val descriptionRes: Int,
    val assetPath: String,
) {
    Project(R.string.about_project_license, R.string.about_project_license_desc, "licenses/PROJECT_LICENSE.txt"),
    ThirdParty(R.string.about_third_party_licenses, R.string.about_third_party_licenses_desc, "licenses/THIRD_PARTY_NOTICES.txt"),
}

/** Small text blocks keep Compose from measuring a megabyte-sized Text item. */
internal fun licenseTextBlocks(text: String): List<String> {
    val blocks = mutableListOf<String>()
    var start = 0
    while (start < text.length) {
        var end = minOf(start + 2048, text.length)
        if (end < text.length) {
            if (text[end - 1].isHighSurrogate() && text[end].isLowSurrogate()) end--
            var newline = end - 1
            while (newline >= start && text[newline] != '\n') newline--
            if (newline >= start) end = newline + 1
        }
        blocks.add(text.substring(start, end))
        start = end
    }
    return blocks
}
