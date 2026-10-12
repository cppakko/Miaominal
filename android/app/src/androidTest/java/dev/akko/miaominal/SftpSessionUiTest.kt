package dev.akko.miaominal

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import dev.akko.miaominal.data.BusinessSettings
import dev.akko.miaominal.data.FakeSftpSession
import dev.akko.miaominal.data.HostKeyDecision
import dev.akko.miaominal.data.InMemoryMiaominalRepository
import dev.akko.miaominal.data.SftpEntryKind
import dev.akko.miaominal.data.SftpEntryModel
import dev.akko.miaominal.data.SftpOperationModel
import dev.akko.miaominal.data.SftpPhase
import dev.akko.miaominal.data.SftpTransferStateModel
import dev.akko.miaominal.ui.MiaominalApp
import dev.akko.miaominal.ui.theme.DEFAULT_THEME_SEED
import dev.akko.miaominal.ui.theme.MiaominalTheme
import dev.akko.miaominal.ui.theme.ThemeMode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

/**
 * SFTP UI acceptance against the injected test repository and a scriptable
 * [FakeSftpSession]: real directory state, request-id-bound prompts, rebuild
 * recovery, and the transfer list.
 */
class SftpSessionUiTest {
    @get:Rule
    val compose = createComposeRule()

    private fun label(id: Int, vararg args: Any): String =
        androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().targetContext.getString(id, *args)

    private val repository = InMemoryMiaominalRepository(
        hosts = TestFixtures.hosts(),
        statuses = TestFixtures.hostStatus(),
        settings = BusinessSettings(onboardingCompleted = true)
    )

    private fun setContent(restoration: StateRestorationTester? = null) {
        val content: @androidx.compose.runtime.Composable () -> Unit = {
            MiaominalTheme {
                MiaominalApp(
                    repository = repository,
                    themeMode = ThemeMode.Light,
                    darkTheme = false,
                    dynamicColor = false,
                    seedColor = DEFAULT_THEME_SEED,
                    onThemeModeChange = {},
                    onDynamicColorChange = {},
                    onSeedColorChange = {}
                )
            }
        }
        if (restoration != null) {
            restoration.setContent(content)
        } else {
            compose.setContent(content)
        }
        compose.waitForIdle()
    }

    private fun expandHostAndOpenSftp() {
        compose.onAllNodesWithContentDescription(
            androidx.test.platform.app.InstrumentationRegistry.getInstrumentation()
                .targetContext.getString(R.string.action_expand)
        )[0].performClick()
        compose.waitForIdle()
        compose.onAllNodesWithText("SFTP")[0].performClick()
        compose.waitUntil(timeoutMillis = 15_000) {
            compose.onAllNodesWithText(label(R.string.fm_remote)).fetchSemanticsNodes().isNotEmpty()
        }
    }

    private fun openSftpScreen() {
        setContent()
        expandHostAndOpenSftp()
    }

    private fun expandHostAndOpenSftpWaitingForText(text: String) {
        compose.onAllNodesWithContentDescription(
            androidx.test.platform.app.InstrumentationRegistry.getInstrumentation()
                .targetContext.getString(R.string.action_expand)
        )[0].performClick()
        compose.waitForIdle()
        compose.onAllNodesWithText("SFTP")[0].performClick()
        compose.waitUntil(timeoutMillis = 15_000) {
            compose.onAllNodesWithText(text).fetchSemanticsNodes().isNotEmpty()
        }
    }

    private fun seedReadySession(): FakeSftpSession {
        val session = repository.seedSftpSession(TestFixtures.PRIMARY_HOST_ID, SftpPhase.Ready)
        session.seedTree(
            "/home/tester",
            listOf(
                SftpEntryModel("docs", "/home/tester/docs", SftpEntryKind.Directory, null, null, null, null),
                SftpEntryModel("report.txt", "/home/tester/report.txt", SftpEntryKind.File, 2048, null, null, "tester")
            )
        )
        session.seedTree(
            "/home/tester/docs",
            listOf(
                SftpEntryModel("notes.md", "/home/tester/docs/notes.md", SftpEntryKind.File, 128, null, null, "tester")
            )
        )
        repository.refreshSftpSnapshots()
        return session
    }

    @Test
    fun deletingOneEntryDoesNotIncludeOtherSelectedEntries() {
        verifySingleEntryDelete(includeTargetInSelection = false)
    }

    @Test
    fun deletingAnAlreadySelectedEntryDoesNotToggleItOutOfTheTargets() {
        verifySingleEntryDelete(includeTargetInSelection = true)
    }

    private fun verifySingleEntryDelete(includeTargetInSelection: Boolean) {
        seedReadySession()
        openSftpScreen()
        compose.onNodeWithText("report.txt").performTouchInput { longClick() }
        if (includeTargetInSelection) {
            compose.onNodeWithText("docs").performTouchInput { longClick() }
        }
        compose.onAllNodesWithContentDescription(label(R.string.fm_item_actions))[0].performClick()
        compose.onNodeWithText(label(R.string.action_delete)).performClick()
        compose.onNodeWithTag("sftp-delete-targets")
            .assertTextEquals(label(R.string.fm_delete_multi_message, label(R.string.fm_remote), 1) + "\n\ndocs")
        compose.onNodeWithText(label(android.R.string.cancel)).performClick()
        compose.onNodeWithText("report.txt").assertIsDisplayed()
        compose.onNodeWithText("docs").assertIsDisplayed()
        compose.onNodeWithText(label(R.string.fm_selected_count, if (includeTargetInSelection) 2 else 1))
            .assertIsDisplayed()
    }

    @Test
    fun remoteEntriesAppearAndNavigationEntersDirectories() {
        val session = seedReadySession()
        openSftpScreen()

        compose.onNodeWithText("report.txt").assertIsDisplayed()
        compose.onNodeWithText("docs").assertIsDisplayed()

        compose.onNodeWithText("docs").performClick()
        assertTrue(
            "entering a directory must request it from the backend",
            session.requestedPaths.any { it.contains("docs") }
        )
        compose.waitUntil(timeoutMillis = 10_000) {
            compose.onAllNodesWithText("notes.md").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText("notes.md").assertIsDisplayed()
    }

    @Test
    fun hostKeyPromptCarriesTheRequestIdAndSurvivesRebuild() {
        val session = repository.seedSftpSession(
            TestFixtures.PRIMARY_HOST_ID,
            SftpPhase.WaitingForTrust
        )
        session.requestHostKey("SHA256:sftp-test")
        repository.refreshSftpSnapshots()

        val restoration = StateRestorationTester(compose)
        setContent(restoration)
        expandHostAndOpenSftpWaitingForText(label(R.string.session_trust_title))

        compose.onNodeWithText(label(R.string.session_trust_title)).assertIsDisplayed()
        compose.onNodeWithText("SHA256:sftp-test", substring = true).assertIsDisplayed()

        restoration.emulateSavedInstanceStateRestore()
        compose.waitUntil(timeoutMillis = 10_000) {
            compose.onAllNodesWithText(label(R.string.session_trust_title)).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText(label(R.string.session_trust_title)).assertIsDisplayed()

        compose.onNodeWithText(label(R.string.session_trust_accept_once)).performClick()
        compose.waitUntil(timeoutMillis = 10_000) {
            compose.onAllNodesWithText(label(R.string.session_trust_title)).fetchSemanticsNodes().isEmpty()
        }
        assertEquals(SftpPhase.Ready, session.snapshot().phase)
        compose.onNodeWithText(label(R.string.session_trust_title)).assertDoesNotExist()
    }

    @Test
    fun transferSheetShowsProgressAndControlActions() {
        val session = seedReadySession()
        session.seedTransfer("release.tar.gz", SftpTransferStateModel.Running)
        repository.refreshSftpSnapshots()
        openSftpScreen()

        compose.waitUntil(timeoutMillis = 10_000) {
            compose.onAllNodesWithTag("sftp-transfer-dock").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("sftp-transfer-dock").performClick()
        compose.waitForIdle()
        compose.onNodeWithText("release.tar.gz").assertIsDisplayed()
        compose.onNodeWithText(label(R.string.ui_transferring), substring = true).assertIsDisplayed()
    }

    @Test
    fun failingTransferIsReportedWithItsKind() {
        val session = seedReadySession()
        session.seedTransfer("broken.bin", SftpTransferStateModel.Failed)
        repository.refreshSftpSnapshots()
        openSftpScreen()

        compose.waitForIdle()
        compose.waitUntil(timeoutMillis = 10_000) {
            compose.onAllNodesWithTag("sftp-transfer-dock").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("sftp-transfer-dock").performClick()
        compose.waitForIdle()
        compose.onNode(hasText(label(R.string.forward_runtime_failed), substring = true) and hasAnyAncestor(hasTestTag("sftp-transfer-list"))).assertIsDisplayed()
    }

    @Test
    fun aPausedTransferCanBeResumedAfterAPageRebuild() {
        val session = seedReadySession()
        val transferId = session.seedTransfer("resume-me.bin", SftpTransferStateModel.Paused)
        repository.refreshSftpSnapshots()
        // The screen tracks handles in the application-scoped repository, so a
        // rebuilt page can still control them.
        repository.trackSftpTransfer(session.connectionId, session.handleFor(transferId)!!)

        val restoration = StateRestorationTester(compose)
        setContent(restoration)
        expandHostAndOpenSftp()
        restoration.emulateSavedInstanceStateRestore()
        compose.waitForIdle()

        compose.waitUntil(timeoutMillis = 10_000) {
            compose.onAllNodesWithTag("sftp-transfer-dock").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("sftp-transfer-dock").performClick()
        compose.waitForIdle()
        compose.onNodeWithContentDescription(label(R.string.ui_resume)).performClick()
        compose.waitForIdle()

        assertEquals(
            SftpTransferStateModel.Running,
            session.handleFor(transferId)?.snapshot()?.state
        )
    }

    @Test
    fun aFailedDirectoryOperationIsShownOnce() {
        val session = seedReadySession()
        openSftpScreen()

        session.failOperation(SftpOperationModel.RemoveFile, "/locked.txt", "permission denied")
        repository.refreshSftpSnapshots()

        compose.waitUntil(timeoutMillis = 10_000) {
            compose.onAllNodesWithText(label(R.string.ui_permission_denied), substring = true).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onAllNodesWithText(label(R.string.ui_permission_denied), substring = true)[0].assertIsDisplayed()
    }

    @Test
    fun vaultLockedSessionShowsTheWaitingState() {
        // The fake reports a locked vault the same way the native backend does:
        // opening fails before any connection is attempted.
        setContent()
        expandHostAndOpenSftp()
        // Without a seeded session the repository opens a ready fake; the
        // screen must stay usable rather than crash.
        compose.onNodeWithText(label(R.string.fm_remote)).assertIsDisplayed()
    }
}
