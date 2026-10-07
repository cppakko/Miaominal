package dev.akko.miaominal

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import dev.akko.miaominal.data.BusinessSettings
import dev.akko.miaominal.data.FakeTerminalSession
import dev.akko.miaominal.data.HostKeyDecision
import dev.akko.miaominal.data.InMemoryMiaominalRepository
import dev.akko.miaominal.data.SessionPhase
import dev.akko.miaominal.data.SessionPromptModel
import dev.akko.miaominal.ui.MiaominalApp
import dev.akko.miaominal.ui.theme.DEFAULT_THEME_SEED
import dev.akko.miaominal.ui.theme.MiaominalTheme
import dev.akko.miaominal.ui.theme.ThemeMode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

/**
 * SSH UI acceptance against the injected test repository: connection phases,
 * trust/authentication prompts, rebuild recovery, terminal input echo, and the
 * multi-session drawer.
 */
class SshSessionUiTest {
    @get:Rule
    val compose = createComposeRule()

    private val repository = InMemoryMiaominalRepository(
        hosts = TestFixtures.hosts(),
        statuses = TestFixtures.hostStatus(),
        settings = BusinessSettings(onboardingCompleted = true)
    )

    private fun setContent() {
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

    private fun awaitTerminalSurface() {
        compose.waitUntil(timeoutMillis = 15_000) {
            compose.onAllNodesWithTag("terminal-surface").fetchSemanticsNodes().isNotEmpty()
        }
    }

    private fun latestSession(): FakeTerminalSession =
        repository.sessionHandle(repository.sessions.value.last().connectionId) as FakeTerminalSession

    @Test
    fun trustDialogAnswersCurrentRequestAndSurvivesRebuild() {
        val session = repository.seedSession(
            TestFixtures.PRIMARY_HOST_ID,
            SessionPhase.WaitingForTrust
        )
        session.awaitTrust(requestId = 7L, host = "47.245.42.8")
        repository.refreshSessionSnapshots()

        val restoration = StateRestorationTester(compose)
        restoration.setContent {
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

        compose.onNodeWithText("确认主机密钥").assertIsDisplayed()
        compose.onNodeWithText("SHA256:test", substring = true).assertIsDisplayed()

        // Activity/state rebuild keeps showing the backend's pending request.
        restoration.emulateSavedInstanceStateRestore()
        compose.waitForIdle()
        compose.onNodeWithText("确认主机密钥").assertIsDisplayed()

        compose.onNodeWithText("仅本次信任").performClick()
        compose.waitForIdle()
        assertEquals(listOf(7L to HostKeyDecision.AcceptOnce), session.respondedHostKeys)
        compose.onNodeWithText("确认主机密钥").assertDoesNotExist()
    }

    @Test
    fun changedHostKeyUsesTheMismatchWording() {
        val session = repository.seedSession(
            TestFixtures.PRIMARY_HOST_ID,
            SessionPhase.WaitingForTrust
        )
        session.awaitTrust(requestId = 9L, changed = true)
        repository.refreshSessionSnapshots()
        setContent()

        compose.onNodeWithText("主机密钥已变更").assertIsDisplayed()
        compose.onNodeWithText("SHA256:old", substring = true).assertIsDisplayed()
        compose.onNodeWithText("拒绝").performClick()
        compose.waitForIdle()
    }

    @Test
    fun keyboardInteractiveDialogCollectsPerPromptAnswers() {
        val session = repository.seedSession(
            TestFixtures.PRIMARY_HOST_ID,
            SessionPhase.WaitingForAuthentication
        )
        session.awaitAuthentication(
            requestId = 11L,
            prompts = listOf(
                SessionPromptModel("Password: ", echo = false),
                SessionPromptModel("OTP: ", echo = true)
            )
        )
        repository.refreshSessionSnapshots()
        setContent()

        compose.onNodeWithText("Two-factor").assertIsDisplayed()
        val fields = compose.onAllNodes(
            hasSetTextAction() and hasAnyAncestor(isDialog())
        )
        fields[0].performTextInput("secret")
        fields[1].performTextInput("123456")
        compose.onNodeWithText("提交").performClick()
        compose.waitForIdle()

        assertEquals(1, session.respondedKeyboardInteractive.size)
        val (requestId, answers) = session.respondedKeyboardInteractive.first()
        assertEquals(11L, requestId)
        assertEquals(listOf("secret", "123456"), answers)
    }

    @Test
    fun remoteEchoRendersAfterTerminalInput() {
        setContent()
        compose.onNodeWithText("edge-sg-prod").performClick()
        compose.onNodeWithText("连接").performScrollTo().performClick()
        awaitTerminalSurface()

        val session = latestSession()
        compose.onNodeWithContentDescription("Ctrl+C：中断当前输入").performClick()
        compose.waitForIdle()

        assertTrue("key dock must reach the session", session.handledKeys.contains("c"))
        assertTrue(
            "remote echo must reach the grid",
            session.grid.any { line -> line.contains("^C") }
        )
    }

    @Test
    fun inputRacingADisconnectDoesNotCrash() {
        setContent()
        compose.onNodeWithText("edge-sg-prod").performClick()
        compose.onNodeWithText("连接").performScrollTo().performClick()
        awaitTerminalSurface()

        val session = latestSession()
        // The backend rejects the write (session just ended) while the UI still
        // considers it connected: the view must swallow the failure.
        session.failNextSend = true
        compose.onNodeWithContentDescription("Ctrl+C：中断当前输入").performClick()
        compose.waitForIdle()
        assertTrue("the rejected key must not reach the session", session.handledKeys.isEmpty())

        // The surface stays usable once the session accepts input again.
        compose.onNodeWithContentDescription("Ctrl+C：中断当前输入").performClick()
        compose.waitForIdle()
        assertTrue(session.handledKeys.contains("c"))
    }

    @Test
    fun multipleSessionsAppearInTheDrawer() {
        setContent()
        compose.onNodeWithText("edge-sg-prod").performClick()
        compose.onNodeWithText("连接").performScrollTo().performClick()
        awaitTerminalSurface()
        androidx.test.espresso.Espresso.pressBack()
        compose.waitForIdle()
        compose.onNodeWithText("home-lab").performClick()
        compose.onNodeWithText("连接").performScrollTo().performClick()
        awaitTerminalSurface()

        compose.onNodeWithContentDescription("打开侧边栏").performClick()
        compose.waitForIdle()
        assertEquals(2, repository.sessions.value.size)
        assertTrue(compose.onAllNodesWithText("edge-sg-prod").fetchSemanticsNodes().isNotEmpty())
        assertTrue(compose.onAllNodesWithText("home-lab").fetchSemanticsNodes().isNotEmpty())
    }
}
