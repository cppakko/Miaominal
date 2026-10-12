package dev.akko.miaominal

import android.graphics.Bitmap
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.*
import dev.akko.miaominal.ui.LocalMiaominalRepository
import dev.akko.miaominal.ui.components.LocalTwoPaneAvailable
import dev.akko.miaominal.ui.model.HostProfileUi
import dev.akko.miaominal.ui.screens.SftpScreen
import dev.akko.miaominal.ui.theme.MiaominalTheme
import org.junit.Rule
import org.junit.Test
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import java.io.File

/** Exercises the browser against screen state and the scriptable repository. */
class SftpBrowserUiTest {
    @get:Rule val compose = createComposeRule()
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val repository = InMemoryMiaominalRepository(hosts = TestFixtures.hosts())
    private val dark = mutableStateOf(false)
    private val profile = HostProfileUi(
        id = TestFixtures.PRIMARY_HOST_ID, name = "Development server", address = "192.168.1.24",
        username = "ubuntu", group = "", latency = "", lastConnected = "", online = true
    )

    private fun open(withTransfer: Boolean = false, restoration: StateRestorationTester? = null,
        phase: SftpPhase = SftpPhase.Ready): FakeSftpSession {
        val session = repository.seedSftpSession(profile.id, phase)
        session.seedTree("/home/ubuntu", listOf(
            SftpEntryModel("Documents", "/home/ubuntu/Documents", SftpEntryKind.Directory, null, null, null, "ubuntu"),
            SftpEntryModel("Projects", "/home/ubuntu/Projects", SftpEntryKind.Directory, null, null, null, "ubuntu"),
            SftpEntryModel("Backups", "/home/ubuntu/Backups", SftpEntryKind.Directory, null, null, null, "ubuntu"),
            SftpEntryModel("compose.yaml", "/home/ubuntu/compose.yaml", SftpEntryKind.File, 2048, null, null, "ubuntu"),
            SftpEntryModel("release.tar.gz", "/home/ubuntu/release.tar.gz", SftpEntryKind.File, 48_234_496, null, null, "ubuntu"),
            SftpEntryModel("README.md", "/home/ubuntu/README.md", SftpEntryKind.File, 4096, null, null, "ubuntu")
        ))
        session.listDirectory("/home/ubuntu")
        if (withTransfer) session.seedTransfer("release.tar.gz", SftpTransferStateModel.Running)
        repository.refreshSftpSnapshots()
        val content: @androidx.compose.runtime.Composable () -> Unit = {
            MiaominalTheme(darkTheme = dark.value, dynamicColor = false) {
                CompositionLocalProvider(
                    LocalMiaominalRepository provides repository,
                    LocalTwoPaneAvailable provides (LocalConfiguration.current.screenWidthDp >= 840)
                ) {
                    SftpScreen(profile = profile, onDismiss = {}, modifier = Modifier.testTag("sftp-browser"))
                }
            }
        }
        if (restoration != null) restoration.setContent(content) else compose.setContent(content)
        compose.waitUntil(15_000) {
            if (phase == SftpPhase.Ready) compose.onAllNodesWithText("Backups").fetchSemanticsNodes().isNotEmpty()
            else compose.onAllNodesWithTag("sftp-browser-loading").fetchSemanticsNodes().isNotEmpty()
        }
        return session
    }

    @Test fun connectingStatusAdvancesToTheReadyFileBrowser() {
        val session = open(phase = SftpPhase.Connecting)
        compose.onNodeWithTag("sftp-browser-loading").assertIsDisplayed()
        compose.onNodeWithText("Backups").assertDoesNotExist()
        screenshot("sftp-connecting-light")
        compose.runOnIdle {
            dark.value = true
            session.setPhase(SftpPhase.OpeningSubsystem)
        }
        compose.waitUntil(10_000) {
            compose.onAllNodesWithText(context.getString(R.string.ui_opening_sftp_subsystem)).fetchSemanticsNodes().isNotEmpty()
        }
        screenshot("sftp-opening-dark")
        compose.runOnIdle { session.setPhase(SftpPhase.Ready) }
        compose.waitUntil(10_000) { compose.onAllNodesWithText("Backups").fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithTag("sftp-browser-loading").assertDoesNotExist()
        compose.onNodeWithText("Backups").assertIsDisplayed()
    }

    @Test fun directoryLoadingShowsStatusAndPreservesFilesDuringRefresh() {
        val session = open()
        val entries = session.directorySnapshot().entries
        compose.runOnIdle { session.seedDirectory("/home/ubuntu", emptyList(), loading = true) }
        compose.waitUntil(10_000) {
            compose.onAllNodesWithText(context.getString(R.string.sftp_browser_loading_directory)).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText(context.getString(R.string.sftp_no_entries)).assertDoesNotExist()
        screenshot("sftp-directory-loading")
        compose.runOnIdle { session.seedDirectory("/home/ubuntu", entries) }
        compose.waitUntil(10_000) { compose.onAllNodesWithText("Backups").fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithTag("sftp-browser-loading").assertDoesNotExist()
        compose.runOnIdle { session.seedDirectory("/home/ubuntu", entries, loading = true) }
        compose.waitUntil(10_000) {
            compose.onAllNodes(SemanticsMatcher.expectValue(SemanticsProperties.ProgressBarRangeInfo,
                ProgressBarRangeInfo.Indeterminate)).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText("Backups").assertIsDisplayed()
        compose.onNodeWithTag("sftp-browser-loading").assertDoesNotExist()
    }

    @Test fun directoryFilterCanBeClearedAfterNoResults() {
        open()
        compose.onNodeWithTag("sftp-directory-search").assertDoesNotExist()
        compose.onNodeWithTag("sftp-search-action").performClick()
        compose.onNodeWithTag("sftp-directory-search").performTextInput("no-such-file")
        compose.onNodeWithText(context.getString(R.string.ux_empty_filter)).assertIsDisplayed()
        compose.onNodeWithContentDescription(context.getString(R.string.sftp_browser_clear_search)).performClick()
        compose.onNodeWithContentDescription(context.getString(R.string.sftp_browser_close_search)).performClick()
        compose.onNodeWithTag("sftp-directory-search").assertDoesNotExist()
        compose.onNodeWithText("compose.yaml").assertIsDisplayed()
    }

    @Test fun filteredSelectAllIncludesOnlyMatchingFiles() {
        open()
        compose.onNodeWithTag("sftp-search-action").performClick()
        compose.onNodeWithTag("sftp-directory-search").performTextInput("compose")
        compose.onNodeWithTag("sftp-directory-options").performClick()
        compose.onNodeWithText(context.getString(R.string.ux_choose)).performClick()
        compose.onNodeWithText(context.getString(R.string.fm_select_all)).performClick()
        compose.onNodeWithText(context.getString(R.string.fm_selected_count, 1)).assertIsDisplayed()
        compose.onNode(hasContentDescription(context.getString(R.string.fm_more_actions)) and
            hasAnyAncestor(hasTestTag("sftp-selection-bar"))).performClick()
        compose.onNodeWithText(context.getString(R.string.fm_delete_selected)).performClick()
        compose.onNodeWithTag("sftp-delete-targets").assertTextEquals(
            context.getString(R.string.fm_delete_multi_message, context.getString(R.string.fm_remote), 1) + "\n\ncompose.yaml")
        compose.onNodeWithText(context.getString(android.R.string.cancel)).performClick()
    }

    @Test fun changingDirectoriesHidesPreviousFilesUntilTheRequestedListingArrives() {
        val session = open()
        compose.runOnIdle { session.deferDirectoryListings = true }
        compose.onNodeWithText("Projects").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithTag("sftp-browser-loading").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText("Backups").assertDoesNotExist()
        compose.onNodeWithText("compose.yaml").assertDoesNotExist()
        compose.onAllNodesWithText("Projects")[0].assertIsDisplayed()
        compose.onNodeWithText(context.getString(R.string.sftp_browser_loading_short)).assertIsDisplayed()
        screenshot("sftp-changing-directory")
        compose.runOnIdle {
            session.seedDirectory("/home/ubuntu/Projects", listOf(
                SftpEntryModel("notes.md", "/home/ubuntu/Projects/notes.md", SftpEntryKind.File, 128, null, null, "ubuntu")))
        }
        compose.waitUntil(10_000) { compose.onAllNodesWithText("notes.md").fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithText("notes.md").assertIsDisplayed()
        assertEquals("/home/ubuntu/Projects", session.requestedPaths.last())
    }

    @Test fun filteringAndDirectoryNavigationResetScrollAndSelection() {
        val session = open()
        fun files(prefix: String, path: String) = (0 until 80).map { index ->
            val name = "$prefix-${index.toString().padStart(3, '0')}.txt"
            SftpEntryModel(name, "$path/$name", SftpEntryKind.File, 128, null, null, "ubuntu")
        }
        compose.runOnIdle {
            session.seedTree("/home", files("parent", "/home"))
            session.seedDirectory("/home/ubuntu", files("report", "/home/ubuntu"))
        }
        compose.waitUntil(10_000) { compose.onAllNodesWithText("report-000.txt").fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithTag("sftp-remote-files").performScrollToIndex(40)
        compose.onNodeWithText("report-000.txt").assertDoesNotExist()
        compose.onNodeWithTag("sftp-search-action").performClick()
        compose.onNodeWithTag("sftp-directory-search").performTextInput("report")
        compose.onNodeWithText("report-000.txt").assertIsDisplayed()
        compose.onNodeWithTag("sftp-directory-options").performClick()
        compose.onNodeWithText(context.getString(R.string.ux_choose)).performClick()
        compose.onNodeWithText(context.getString(R.string.fm_select_all)).performClick()
        compose.onNodeWithText(context.getString(R.string.fm_selected_count, 80)).assertIsDisplayed()
        compose.onNodeWithContentDescription(context.getString(R.string.sftp_browser_close_search)).performClick()
        compose.onNodeWithText("home").performClick()
        compose.waitUntil(10_000) { compose.onAllNodesWithText("parent-000.txt").fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithText("parent-000.txt").assertIsDisplayed()
        compose.onNodeWithTag("sftp-selection-bar").assertDoesNotExist()
        compose.onNodeWithTag("sftp-directory-search").assertDoesNotExist()
    }

    @Test fun releasedSwipesSnapToOneCompletePane() {
        open()
        val remoteLeft = compose.onNodeWithText("Backups").fetchSemanticsNode().boundsInRoot.left
        val pager = compose.onNodeWithTag("sftp-pane-pager")
        pager.performTouchInput {
            swipe(Offset(width * 0.2f, height * 0.75f), Offset(width * 0.45f, height * 0.75f), durationMillis = 600)
        }
        compose.waitForIdle()
        screenshot("sftp-swipe-returned")
        assertEquals(remoteLeft, compose.onNodeWithText("Backups").fetchSemanticsNode().boundsInRoot.left, 1f)
        pager.performTouchInput {
            swipe(Offset(width * 0.15f, height * 0.75f), Offset(width * 0.85f, height * 0.75f), durationMillis = 600)
        }
        screenshot("sftp-swipe-released")
        compose.onNodeWithText(context.getString(R.string.sftp_local_pick)).assertIsDisplayed()
        pager.performTouchInput {
            swipe(Offset(width * 0.85f, height * 0.75f), Offset(width * 0.15f, height * 0.75f), durationMillis = 600)
        }
        compose.waitForIdle()
        assertEquals(remoteLeft, compose.onNodeWithText("Backups").fetchSemanticsNode().boundsInRoot.left, 1f)
    }

    @Test fun paneIndicatorTracksDragAndReturnsWhenCancelled() {
        open()
        val remoteLeft = compose.onNodeWithText("Backups").fetchSemanticsNode().boundsInRoot.left
        val pager = compose.onNodeWithTag("sftp-pane-pager")
        val indicator = compose.onNodeWithTag("sftp-pane-indicator")
        val restingLeft = indicator.fetchSemanticsNode().boundsInRoot.left
        assertIndicatorAlignedWith("sftp-pane-remote")
        pager.performTouchInput {
            down(Offset(width * 0.15f, height * 0.75f))
            for (step in 1..10) moveTo(Offset(width * (0.15f + 0.035f * step), height * 0.75f), delayMillis = 40)
        }
        assertIndicatorTracksRemotePane(remoteLeft)
        compose.onNodeWithTag("sftp-pane-remote").assertIsSelected()
        val draggedLeft = indicator.fetchSemanticsNode().boundsInRoot.left
        screenshot("sftp-indicator-mid-drag")
        pager.performTouchInput {
            for (step in 1..5) moveTo(Offset(width * (0.5f - 0.035f * step), height * 0.75f), delayMillis = 40)
        }
        assertIndicatorTracksRemotePane(remoteLeft)
        assertTrue(indicator.fetchSemanticsNode().boundsInRoot.left > draggedLeft + 10f)
        pager.performTouchInput { cancel() }
        compose.waitForIdle()
        assertEquals(restingLeft, indicator.fetchSemanticsNode().boundsInRoot.left, 1f)
        assertIndicatorAlignedWith("sftp-pane-remote")
    }

    @Test fun paneIndicatorTracksTabClickAnimation() {
        open()
        val remoteLeft = compose.onNodeWithText("Backups").fetchSemanticsNode().boundsInRoot.left
        compose.mainClock.autoAdvance = false
        try {
            compose.onNodeWithTag("sftp-pane-local").performClick()
            compose.mainClock.advanceTimeBy(96)
            assertIndicatorTracksRemotePane(remoteLeft)
        } finally {
            compose.mainClock.autoAdvance = true
        }
        compose.waitForIdle()
        compose.onNodeWithTag("sftp-pane-local").assertIsSelected()
        assertIndicatorAlignedWith("sftp-pane-local")
        compose.onNodeWithTag("sftp-pane-remote").performClick()
        compose.waitForIdle()
        assertIndicatorAlignedWith("sftp-pane-remote")
    }

    @Test fun cancelledSwipeReturnsToACompletePane() {
        open()
        val remoteLeft = compose.onNodeWithText("Backups").fetchSemanticsNode().boundsInRoot.left
        val pager = compose.onNodeWithTag("sftp-pane-pager")
        pager.performTouchInput {
            down(Offset(width * 0.15f, height * 0.75f))
            for (step in 1..10) moveTo(Offset(width * (0.15f + 0.035f * step), height * 0.75f), delayMillis = 40)
        }
        assertTrue(compose.onNodeWithText("Backups").fetchSemanticsNode().boundsInRoot.left > remoteLeft + 50f)
        pager.performTouchInput { cancel() }
        compose.waitForIdle()
        assertEquals(remoteLeft, compose.onNodeWithText("Backups").fetchSemanticsNode().boundsInRoot.left, 1f)
    }

    @Test fun restoringDuringASwipeDoesNotLeaveHalfAFilePane() {
        val restoration = StateRestorationTester(compose)
        open(restoration = restoration)
        val remoteLeft = compose.onNodeWithText("Backups").fetchSemanticsNode().boundsInRoot.left
        val pager = compose.onNodeWithTag("sftp-pane-pager")
        pager.performTouchInput {
            down(Offset(width * 0.15f, height * 0.75f))
            for (step in 1..10) moveTo(Offset(width * (0.15f + 0.035f * step), height * 0.75f), delayMillis = 40)
        }
        assertTrue(compose.onNodeWithText("Backups").fetchSemanticsNode().boundsInRoot.left > remoteLeft + 50f)
        restoration.emulateSavedInstanceStateRestore()
        pager.performTouchInput { cancel() }
        compose.waitForIdle()
        assertEquals(remoteLeft, compose.onNodeWithText("Backups").fetchSemanticsNode().boundsInRoot.left, 1f)
    }

    @Test fun createMenuOpensTheNewFolderDialog() {
        open()
        compose.onNodeWithTag("sftp-create-action").performClick()
        compose.onNodeWithTag("sftp-create-menu").assertIsDisplayed()
        screenshot("sftp-create-menu-light")
        compose.onNodeWithText(context.getString(R.string.fm_new_folder)).performClick()
        compose.onNodeWithText(context.getString(R.string.fm_folder_name)).assertIsDisplayed()
    }

    @Test fun createMenuClosesWithScrimCloseButtonAndBack() {
        open()
        compose.onNodeWithTag("sftp-create-action").performClick()
        compose.onNodeWithText(context.getString(R.string.sftp_upload_files)).assertIsDisplayed()
        compose.onNodeWithText(context.getString(R.string.sftp_upload_folder)).assertIsDisplayed()
        compose.runOnIdle { dark.value = true }
        screenshot("sftp-create-menu-dark")
        compose.onNodeWithTag("sftp-create-menu-close").performClick()
        compose.onNodeWithTag("sftp-create-menu").assertDoesNotExist()
        compose.onNodeWithTag("sftp-create-action").performClick()
        compose.onNodeWithTag("sftp-create-scrim").performTouchInput { click(Offset(8f, 8f)) }
        compose.onNodeWithTag("sftp-create-menu").assertDoesNotExist()
        compose.onNodeWithTag("sftp-create-action").performClick()
        androidx.test.espresso.Espresso.pressBack()
        compose.onNodeWithTag("sftp-create-menu").assertDoesNotExist()
        compose.onNodeWithText("Backups").assertIsDisplayed()
    }

    @Test fun sortingAndSelectionRemainAvailableFromTheCompactToolbar() {
        open()
        compose.onNodeWithTag("sftp-directory-options").performClick()
        compose.onNodeWithTag("sftp-directory-options-menu").assertIsDisplayed()
        compose.onNodeWithTag("sftp-sort-name").assertIsSelected()
        screenshot("sftp-filter-panel-light")
        compose.onNodeWithTag("sftp-sort-size").performClick()
        compose.onNodeWithTag("sftp-directory-options").assertTextContains(context.getString(R.string.sftp_browser_sort_size))
        compose.onNodeWithTag("sftp-directory-options-menu").assertIsDisplayed()
        assertSortDirection("size", ascending = false)
        screenshot("sftp-sort-size-descending")
        compose.onNodeWithTag("sftp-sort-size").performClick()
        assertSortDirection("size", ascending = true)
        screenshot("sftp-sort-size-ascending")
        compose.onNodeWithTag("sftp-sort-modified").performClick()
        assertSortDirection("modified", ascending = false)
        screenshot("sftp-sort-modified-descending")
        compose.onNodeWithText(context.getString(R.string.ux_choose)).performClick()
        compose.onNodeWithText(context.getString(R.string.fm_selected_count, 0)).assertIsDisplayed()
    }

    @Test fun repeatedSortClicksReverseFilesAndRestoreTheSelectedDirection() {
        val restoration = StateRestorationTester(compose)
        val session = open(restoration = restoration)
        val entries = session.directorySnapshot().entries.map { entry ->
            entry.copy(modifiedAtMs = when (entry.name) {
                "compose.yaml" -> 1L
                "release.tar.gz" -> 5L
                "README.md" -> 9L
                else -> null
            })
        }
        compose.runOnIdle { session.seedDirectory("/home/ubuntu", entries) }
        compose.onNodeWithTag("sftp-directory-options").performClick()
        assertSortDirection("name", ascending = true)
        compose.onNodeWithTag("sftp-sort-name").performClick()
        assertSortDirection("name", ascending = false)
        assertFilesInOrder("Projects", "Documents", "Backups", "release.tar.gz", "README.md", "compose.yaml")
        compose.onNodeWithTag("sftp-sort-name").performClick()
        assertSortDirection("name", ascending = true)
        assertFilesInOrder("Backups", "Documents", "Projects", "compose.yaml", "README.md", "release.tar.gz")
        compose.onNodeWithTag("sftp-sort-size").performClick()
        assertSortDirection("size", ascending = false)
        assertFilesInOrder("Backups", "Documents", "Projects", "release.tar.gz", "README.md", "compose.yaml")
        compose.onNodeWithTag("sftp-sort-size").performClick()
        assertSortDirection("size", ascending = true)
        assertFilesInOrder("Backups", "Documents", "Projects", "compose.yaml", "README.md", "release.tar.gz")
        compose.onNodeWithTag("sftp-sort-modified").performClick()
        assertSortDirection("modified", ascending = false)
        assertFilesInOrder("Backups", "Documents", "Projects", "README.md", "release.tar.gz", "compose.yaml")
        compose.onNodeWithTag("sftp-sort-modified").performClick()
        assertSortDirection("modified", ascending = true)
        assertFilesInOrder("Backups", "Documents", "Projects", "compose.yaml", "release.tar.gz", "README.md")
        androidx.test.espresso.Espresso.pressBack()
        restoration.emulateSavedInstanceStateRestore()
        compose.onNodeWithTag("sftp-directory-options").assertTextContains(context.getString(R.string.sftp_browser_sort_modified))
        compose.onNodeWithTag("sftp-directory-options").performClick()
        assertSortDirection("modified", ascending = true)
        assertFilesInOrder("Backups", "Documents", "Projects", "compose.yaml", "release.tar.gz", "README.md")
        compose.onNodeWithTag("sftp-sort-modified").performClick()
        assertSortDirection("modified", ascending = false)
        assertFilesInOrder("Backups", "Documents", "Projects", "README.md", "release.tar.gz", "compose.yaml")
    }

    @Test fun optionsPanelCanOpenAndClearTheDirectoryFilter() {
        open()
        compose.onNodeWithTag("sftp-directory-options").performClick()
        compose.runOnIdle { dark.value = true }
        screenshot("sftp-filter-panel-dark")
        compose.onNodeWithText(context.getString(R.string.ux_filter_directory)).performClick()
        compose.onNodeWithTag("sftp-directory-search").performTextInput("compose")
        compose.onNodeWithText("Backups").assertDoesNotExist()
        compose.onNodeWithText("compose.yaml").assertIsDisplayed()
        compose.onNodeWithTag("sftp-directory-options").performClick()
        compose.onNodeWithText(context.getString(R.string.sftp_browser_clear_search)).performClick()
        compose.onNodeWithText("Backups").assertIsDisplayed()
    }

    @Test fun localFolderPickerAndEmptyQueueStayAccessible() {
        open()
        compose.onNodeWithText(context.getString(R.string.fm_local)).performClick()
        compose.onNodeWithText(context.getString(R.string.sftp_local_pick)).assertIsDisplayed()
        screenshot("sftp-local")
        compose.onNodeWithTag("sftp-transfer-dock").performClick()
        compose.onNodeWithText(context.getString(R.string.sftp_browser_transfer_empty)).assertIsDisplayed()
    }

    @Test fun authorizedStorageOpensLocalWithoutAskingForAFolder() {
        val restoration = StateRestorationTester(compose)
        repository.localStorageRoot = "file:///storage/emulated/0"
        repository.safLocalEntries = listOf(SafLocalEntry("Download", "Download", true, null, null))
        open(restoration = restoration)
        compose.onNodeWithTag("sftp-pane-local").performClick()
        compose.onNodeWithText("Download").assertIsDisplayed()
        compose.onAllNodesWithText(context.getString(R.string.sftp_storage_internal)).assertCountEquals(2)
        compose.onNodeWithText(context.getString(R.string.sftp_storage_allow)).assertDoesNotExist()
        restoration.emulateSavedInstanceStateRestore()
        compose.onNodeWithText("Download").assertIsDisplayed()
        compose.onNodeWithTag("sftp-local-files").assertIsDisplayed()
        screenshot("sftp-local-storage")
    }

    @Test fun downloadsUseAuthorizedStorageWithoutOpeningTheFolderPicker() {
        val root = "file:///storage/emulated/0"
        repository.localStorageRoot = root
        repository.safLocalEntries = listOf(SafLocalEntry("Download", "Download", true, null, null))
        val session = open()
        compose.onNodeWithText("compose.yaml").performClick()
        compose.onNodeWithText(context.getString(R.string.fm_download)).performClick()
        compose.waitUntil(10_000) { session.transfersSnapshot().isNotEmpty() }
        val transfer = session.transfersSnapshot().single()
        val request = repository.transferActions.request(TransferKey(transfer.connectionId, transfer.transferId))
        assertEquals(TransferRequest.Download(profile.id, "/home/ubuntu/compose.yaml", root, "compose.yaml"), request)
        compose.onNodeWithTag("sftp-local-files").assertIsDisplayed()
        compose.onNodeWithText(context.getString(R.string.sftp_storage_allow)).assertDoesNotExist()
    }

    @Test fun selectedFilesLeaveTransferDockReachable() {
        open()
        compose.onNodeWithText("compose.yaml").performTouchInput { longClick() }
        compose.onNodeWithText(context.getString(R.string.fm_selected_count, 1)).assertIsDisplayed()
        compose.onNodeWithText(context.getString(R.string.fm_download)).assertIsDisplayed()
        compose.onNodeWithTag("sftp-transfer-dock").assertIsDisplayed()
        screenshot("sftp-selection")
        compose.runOnIdle { dark.value = true }
        screenshot("sftp-selection-dark")
        compose.onNodeWithText(context.getString(R.string.fm_select_all)).performClick()
        compose.onNodeWithText(context.getString(R.string.fm_selected_count, 6)).assertIsDisplayed()
        compose.onNode(hasContentDescription(context.getString(R.string.fm_more_actions)) and
            hasAnyAncestor(hasTestTag("sftp-selection-bar"))).performClick()
        compose.onNodeWithText(context.getString(R.string.fm_delete_selected)).assertIsDisplayed()
        androidx.test.espresso.Espresso.pressBack()
        compose.onNodeWithTag("sftp-selection-close").performClick()
        compose.onNodeWithTag("sftp-selection-bar").assertDoesNotExist()
        compose.onNodeWithText("Backups").assertIsDisplayed()
    }

    @Test fun lightAndDarkThemesKeepFilesAndTransfersAccessible() {
        open(withTransfer = true)
        compose.onNodeWithTag("sftp-create-action").assertIsDisplayed()
        screenshot("sftp-light")
        compose.runOnIdle { dark.value = true }
        compose.waitForIdle()
        compose.onNodeWithText("Backups").assertIsDisplayed()
        screenshot("sftp-dark")
        compose.onNodeWithTag("sftp-transfer-dock").performClick()
        compose.onNodeWithContentDescription(context.getString(R.string.ui_pause)).assertIsDisplayed()
        screenshot("sftp-transfers")
    }

    private fun assertSortDirection(sort: String, ascending: Boolean) {
        val direction = context.getString(if (ascending) R.string.sftp_browser_sort_ascending else R.string.sftp_browser_sort_descending)
        compose.onNodeWithTag("sftp-directory-options-menu").assertIsDisplayed()
        compose.onNodeWithTag("sftp-sort-$sort").assertIsSelected().assert(
            SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, direction))
    }

    private fun assertFilesInOrder(vararg names: String) {
        val positions = names.map { name -> name to compose.onNodeWithText(name).fetchSemanticsNode().boundsInRoot.top }
        assertEquals(names.toList(), positions.sortedBy { it.second }.map { it.first })
    }

    private fun assertIndicatorAlignedWith(tabTag: String) {
        val tab = compose.onNodeWithTag(tabTag).fetchSemanticsNode().boundsInRoot
        val indicator = compose.onNodeWithTag("sftp-pane-indicator").fetchSemanticsNode().boundsInRoot
        assertEquals(tab.left, indicator.left, 1f)
        assertEquals(tab.width, indicator.width, 1f)
    }

    private fun assertIndicatorTracksRemotePane(restingRemoteLeft: Float) {
        val localTab = compose.onNodeWithTag("sftp-pane-local").fetchSemanticsNode().boundsInRoot
        val remoteTab = compose.onNodeWithTag("sftp-pane-remote").fetchSemanticsNode().boundsInRoot
        val indicatorLeft = compose.onNodeWithTag("sftp-pane-indicator").fetchSemanticsNode().boundsInRoot.left
        val pagerWidth = compose.onNodeWithTag("sftp-pane-pager").fetchSemanticsNode().boundsInRoot.width
        val fileLeft = compose.onNodeWithText("Backups").fetchSemanticsNode().boundsInRoot.left
        val pageProgress = (fileLeft - restingRemoteLeft) / pagerWidth
        val indicatorProgress = (remoteTab.left - indicatorLeft) / (remoteTab.left - localTab.left)
        assertTrue("The gesture or animation must move the file pane", pageProgress in 0.05f..0.95f)
        assertEquals("The indicator must move with the file pane", pageProgress, indicatorProgress, 0.04f)
    }

    private fun screenshot(name: String) {
        val folder = File(context.getExternalFilesDir(null), "sftp-previews").apply { mkdirs() }
        File(folder, "$name.png").outputStream().use {
            compose.waitForIdle()
            val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
            // Popup windows can finish drawing after Compose becomes idle.
            automation.waitForIdle(250, 5_000)
            val bitmap = requireNotNull(automation.takeScreenshot())
            try { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) } finally { bitmap.recycle() }
        }
    }
}
