package dev.akko.miaominal

import androidx.annotation.StringRes
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.test.espresso.Espresso
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.BusinessSettings
import dev.akko.miaominal.data.FakeSftpSession
import dev.akko.miaominal.data.InMemoryMiaominalRepository
import dev.akko.miaominal.data.SftpEntryKind
import dev.akko.miaominal.data.SftpEntryModel
import dev.akko.miaominal.data.SftpPhase
import dev.akko.miaominal.ui.MiaominalApp
import dev.akko.miaominal.ui.theme.DEFAULT_THEME_SEED
import dev.akko.miaominal.ui.theme.MiaominalTheme
import dev.akko.miaominal.ui.theme.ThemeMode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

/**
 * End-to-end UI workflows against an injected test backend
 * (`InMemoryMiaominalRepository`): no native backend,
 * no removed demo screens, and no shared business data.
 */
class UiWorkflowTest {
    @get:Rule
    val compose = createComposeRule()

    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private fun string(@StringRes id: Int): String = context.getString(id)

    private val repository = InMemoryMiaominalRepository(
        hosts = TestFixtures.hosts(),
        statuses = TestFixtures.hostStatus(),
        forwards = TestFixtures.forwardRules(),
        snippets = TestFixtures.snippets(),
        keys = TestFixtures.managedKeys(),
        knownHosts = TestFixtures.knownHosts(),
        settings = BusinessSettings(onboardingCompleted = true)
    )

    @org.junit.Before
    fun setUp() {
        compose.setContent {
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
        compose.waitForIdle()
    }

    private fun field(@StringRes label: Int) =
        compose.onNode(hasSetTextAction() and hasText(string(label)))

    private fun awaitTerminalSurface() {
        compose.waitUntil(timeoutMillis = 15_000) {
            compose.onAllNodesWithTag("terminal-surface").fetchSemanticsNodes().isNotEmpty()
        }
    }

    @Test
    fun aNewHostIsValidatedAndSaved() {
        compose.onNodeWithContentDescription(string(R.string.action_add_host)).performClick()
        compose.waitForIdle()

        field(R.string.profile_name).performTextInput("review-host")
        field(R.string.profile_host).performTextInput("review.example.com")
        field(R.string.profile_username).performTextInput("review")
        field(R.string.profile_port).performTextReplacement("99999")
        compose.onNodeWithText(string(R.string.profile_save)).performClick()
        compose.waitForIdle()
        compose.onNodeWithText(string(R.string.profile_port_error)).assertIsDisplayed()

        field(R.string.profile_port).performTextReplacement("2222")
        compose.onNodeWithText(string(R.string.profile_save)).performClick()
        compose.waitUntil(timeoutMillis = 10_000) {
            repository.hosts.value.any { it.name == "review-host" }
        }

        // The saved host appears in the list with the edited connection details.
        val saved = repository.hosts.value.first { it.name == "review-host" }
        assertEquals("review.example.com", saved.host)
        assertEquals(2222, saved.port)
        assertEquals("review", saved.username)
        compose.waitUntil(timeoutMillis = 10_000) {
            compose.onAllNodesWithText("review@review.example.com:2222").fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithText("review@review.example.com:2222").assertIsDisplayed()
    }

    @Test
    fun closingDraftAllowsContinuingOrDiscarding() {
        compose.onNodeWithText(TestFixtures.PRIMARY_HOST_NAME).assertIsDisplayed()
        compose.onNodeWithContentDescription(string(R.string.action_add_host)).performClick()
        compose.waitForIdle()
        field(R.string.profile_name).performTextInput("draft.example.com")

        compose.onNodeWithContentDescription(string(R.string.profile_close)).performClick()
        compose.waitForIdle()
        compose.onNodeWithText(string(R.string.profile_discard_keep)).performClick()
        compose.waitForIdle()
        field(R.string.profile_name).assertTextContains("draft.example.com")

        compose.onNodeWithContentDescription(string(R.string.profile_close)).performClick()
        compose.waitForIdle()
        compose.onNodeWithText(string(R.string.profile_discard_confirm)).performClick()
        compose.waitForIdle()
        compose.onNodeWithText(TestFixtures.PRIMARY_HOST_NAME).assertIsDisplayed()

        compose.onNodeWithContentDescription(string(R.string.action_add_host)).performClick()
        compose.waitForIdle()
        field(R.string.profile_name).assertTextEquals(string(R.string.profile_name), "")
    }

    @Test
    fun terminalSessionSurvivesTheSftpRoundTrip() {
        compose.onNodeWithText(TestFixtures.PRIMARY_HOST_NAME).performClick()
        compose.onNodeWithText("连接").performScrollTo().performClick()
        awaitTerminalSurface()
        val connectionId = repository.sessions.value.single().connectionId

        compose.onNodeWithContentDescription(string(R.string.terminal_open_sftp)).performClick()
        compose.waitUntil(timeoutMillis = 15_000) {
            compose.onAllNodesWithText(string(R.string.fm_remote)).fetchSemanticsNodes().isNotEmpty()
        }
        Espresso.pressBack()
        compose.waitForIdle()

        compose.onNodeWithTag("terminal-surface").assertIsDisplayed()
        assertEquals(listOf(connectionId), repository.sessions.value.map { it.connectionId })
        assertTrue(repository.sessionHandle(connectionId) != null)
    }

    @Test
    fun sftpNavigatesRemoteDirectoriesAndBackKeepsTheSession() {
        val session = seedReadySftpSession()
        compose.onNodeWithText(TestFixtures.PRIMARY_HOST_NAME).performClick()
        compose.onNodeWithText("连接").performScrollTo().performClick()
        awaitTerminalSurface()

        compose.onNodeWithContentDescription(string(R.string.terminal_open_sftp)).performClick()
        compose.waitUntil(timeoutMillis = 15_000) {
            compose.onAllNodesWithText("report.txt").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText("docs").performClick()
        assertTrue(
            "entering a directory must request it from the backend",
            session.requestedPaths.any { it.contains("docs") }
        )
        compose.waitUntil(timeoutMillis = 10_000) {
            compose.onAllNodesWithText("notes.md").fetchSemanticsNodes().isNotEmpty()
        }

        Espresso.pressBack()
        compose.waitForIdle()
        compose.onNodeWithTag("terminal-surface").assertIsDisplayed()
    }

    private fun seedReadySftpSession(): FakeSftpSession {
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
}
