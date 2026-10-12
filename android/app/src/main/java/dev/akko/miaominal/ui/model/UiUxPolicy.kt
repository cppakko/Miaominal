package dev.akko.miaominal.ui.model

import dev.akko.miaominal.data.SafLocalEntry
import dev.akko.miaominal.data.SftpEntryModel
import dev.akko.miaominal.data.TerminalCellFlags
import dev.akko.miaominal.data.TerminalRowModel

enum class FileSort { Name, Size, Modified }
enum class FileTapAction { ToggleSelection, OpenDirectory, OpenDetails }
fun fileTapAction(selecting: Boolean, directory: Boolean) = when {
    selecting -> FileTapAction.ToggleSelection
    directory -> FileTapAction.OpenDirectory
    else -> FileTapAction.OpenDetails
}

fun visibleFiles(entries: List<SftpEntryModel>, query: String, sort: FileSort,
    ascending: Boolean = sort == FileSort.Name): List<SftpEntryModel> {
    val order = fileSortOrder(sort, ascending, SftpEntryModel::name, SftpEntryModel::size, SftpEntryModel::modifiedAtMs)
    return entries.filter { it.name.contains(query.trim(), ignoreCase = true) }
        .sortedWith(compareByDescending<SftpEntryModel> { it.isDirectory }.then(order)
            .thenBy { it.name.lowercase(java.util.Locale.ROOT) }.thenBy { it.path })
}

fun visibleLocalFiles(entries: List<SafLocalEntry>, query: String, sort: FileSort,
    ascending: Boolean = sort == FileSort.Name): List<SafLocalEntry> {
    val order = fileSortOrder(sort, ascending, SafLocalEntry::name, SafLocalEntry::size, SafLocalEntry::modifiedAtMs)
    return entries.filter { it.name.contains(query.trim(), ignoreCase = true) }
        .sortedWith(compareByDescending<SafLocalEntry> { it.isDirectory }.then(order)
            .thenBy { it.name.lowercase(java.util.Locale.ROOT) }.thenBy { it.relativePath })
}

private fun <T> fileSortOrder(sort: FileSort, ascending: Boolean, name: (T) -> String,
    size: (T) -> Long?, modified: (T) -> Long?): Comparator<T> {
    val nameOrder = compareBy<T> { name(it).lowercase(java.util.Locale.ROOT) }
    // Missing metadata stays last in both directions, within each directory/file group.
    val numberOrder = nullsLast(if (ascending) naturalOrder<Long>() else reverseOrder<Long>())
    return when (sort) {
        FileSort.Name -> if (ascending) nameOrder else nameOrder.reversed()
        FileSort.Size -> compareBy(numberOrder, size)
        FileSort.Modified -> compareBy(numberOrder, modified)
    }
}

fun terminalRowText(row: TerminalRowModel): String = row.cells
    .filter { it.flags and TerminalCellFlags.SPACER == 0 }
    .joinToString("") { it.text }.trimEnd()

fun supportsTwoPanes(width: Int, height: Int): Boolean = width >= 840 && height >= 480

data class WorkspaceRegion(val left: Int, val top: Int, val right: Int, val bottom: Int)

/** Use the larger unobstructed region, including tabletop posture. Out-of-window folds are ignored. */
fun safeWorkspaceRegion(width: Int, height: Int, hinge: WorkspaceRegion?, horizontal: Boolean): WorkspaceRegion {
    val full = WorkspaceRegion(0, 0, width, height)
    if (hinge == null) return full
    if (horizontal) {
        if (hinge.top <= 0 || hinge.bottom >= height) return full
        return if (hinge.top >= height - hinge.bottom) full.copy(bottom = hinge.top)
            else full.copy(top = hinge.bottom)
    }
    if (hinge.left <= 0 || hinge.right >= width) return full
    return if (hinge.left >= width - hinge.right) full.copy(right = hinge.left)
        else full.copy(left = hinge.right)
}

fun requiresSshCloseConfirmation(phase: dev.akko.miaominal.data.SessionPhase?) = phase == dev.akko.miaominal.data.SessionPhase.Ready
fun unfinishedTransferCount(transfers: List<dev.akko.miaominal.data.SftpTransferModel>) = transfers.count { !it.state.isTerminal }

enum class EditorErrorField { Name, Host, Port, Environment }

fun firstEditorError(name: String, host: String, port: String, environmentValid: Boolean): EditorErrorField? = when {
    name.isBlank() -> EditorErrorField.Name
    host.isBlank() -> EditorErrorField.Host
    port.toIntOrNull()?.let { it in 1..65535 } != true -> EditorErrorField.Port
    !environmentValid -> EditorErrorField.Environment
    else -> null
}
