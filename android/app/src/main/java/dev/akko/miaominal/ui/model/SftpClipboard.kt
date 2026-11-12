package dev.akko.miaominal.ui.model

import androidx.compose.runtime.saveable.listSaver
import dev.akko.miaominal.data.SftpEntryKind
import dev.akko.miaominal.data.SftpEntryModel

internal data class SftpClipboard(val connectionId: Long, val moving: Boolean, val entries: List<SftpEntryModel>) {
    companion object {
        val Saver = listSaver<SftpClipboard?, Any>(
            save = { clipboard -> if (clipboard == null) emptyList() else listOf(clipboard.connectionId, clipboard.moving) +
                clipboard.entries.flatMap { listOf(it.name, it.path, it.kind.name) } },
            restore = { values -> if (values.isEmpty()) null else SftpClipboard(values[0] as Long, values[1] as Boolean,
                values.drop(2).chunked(3).map {
                    SftpEntryModel(it[0] as String, it[1] as String, SftpEntryKind.valueOf(it[2] as String), null, null, null, null)
                }) }
        )
    }
}

internal fun uniqueSftpPasteName(name: String, occupied: Set<String>): String {
    if (name !in occupied) return name
    val dot = name.lastIndexOf('.').takeIf { it > 0 }
    val stem = dot?.let { name.substring(0, it) } ?: name
    val extension = dot?.let { name.substring(it) } ?: ""
    var index = 1
    while ("$stem ($index)$extension" in occupied) index++
    return "$stem ($index)$extension"
}
