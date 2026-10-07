package dev.akko.miaominal.ui.model

import dev.akko.miaominal.data.*
import org.junit.Assert.*
import org.junit.Test

class UiUxPolicyTest {
    private fun file(name: String, size: Long? = null, modified: Long? = null, directory: Boolean = false) =
        SftpEntryModel(name, "/$name", if (directory) SftpEntryKind.Directory else SftpEntryKind.File, size, modified, null, null)

    @Test fun selectionModeNeverNavigatesIntoDirectories() {
        assertEquals(FileTapAction.OpenDirectory, fileTapAction(false, true))
        assertEquals(FileTapAction.OpenDetails, fileTapAction(false, false))
        assertEquals(FileTapAction.ToggleSelection, fileTapAction(true, true))
        assertEquals(FileTapAction.ToggleSelection, fileTapAction(true, false))
    }

    @Test fun directoryFirstSortsAndPlacesMissingMetadataLast() {
        val files = listOf(file("unknown"), file("small", 1, 9), file("big", 99, 1), file("Folder", directory = true))
        assertEquals(listOf("Folder", "big", "small", "unknown"), visibleFiles(files, "", FileSort.Size).map { it.name })
        assertEquals(listOf("Folder", "small", "big", "unknown"), visibleFiles(files, "", FileSort.Modified).map { it.name })
        assertEquals(listOf("Folder", "big", "small", "unknown"), visibleFiles(files, "", FileSort.Name).map { it.name })
        assertEquals(listOf("Folder"), visibleFiles(files, " FOLD ", FileSort.Name).map { it.name })
    }

    @Test fun formReportsTheFirstErrorInVisualOrder() {
        assertEquals(EditorErrorField.Name, firstEditorError("", "", "0", false))
        assertEquals(EditorErrorField.Host, firstEditorError("name", " ", "0", false))
        listOf("0", "65536", "abc", "").forEach { assertEquals(EditorErrorField.Port, firstEditorError("name", "host", it, false)) }
        assertEquals(EditorErrorField.Environment, firstEditorError("name", "host", "22", false))
        listOf("1", "65535").forEach { assertNull(firstEditorError("name", "host", it, true)) }
    }

    @Test fun readingSkipsWideCellPlaceholdersWithoutLosingUnicode() {
        val row = TerminalRowModel(0, listOf(TerminalCellModel("中", 0, 0, 0),
            TerminalCellModel(" ", 0, 0, TerminalCellFlags.SPACER), TerminalCellModel("é🙂  ", 0, 0, 0)))
        assertEquals("中é🙂", terminalRowText(row))
    }

    @Test fun layoutUsesBothWidthAndHeightAndSafeFoldRegions() {
        assertFalse(supportsTwoPanes(839, 480)); assertFalse(supportsTwoPanes(840, 479))
        assertTrue(supportsTwoPanes(840, 480))
        assertEquals(WorkspaceRegion(420, 0, 1000, 800), safeWorkspaceRegion(1000, 800, WorkspaceRegion(400, 0, 420, 800), false))
        assertEquals(WorkspaceRegion(0, 0, 1000, 450), safeWorkspaceRegion(1000, 800, WorkspaceRegion(0, 450, 1000, 470), true))
        assertEquals(WorkspaceRegion(0, 0, 600, 800), safeWorkspaceRegion(600, 800, WorkspaceRegion(900, 0, 910, 800), false))
    }

    @Test fun endedSshNeedsNoConfirmationButReadyDoes() {
        assertTrue(requiresSshCloseConfirmation(SessionPhase.Ready))
        assertFalse(requiresSshCloseConfirmation(SessionPhase.Closed))
        assertFalse(requiresSshCloseConfirmation(SessionPhase.Failed))
        assertFalse(requiresSshCloseConfirmation(null))
    }
}
