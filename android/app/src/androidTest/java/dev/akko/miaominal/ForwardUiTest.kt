package dev.akko.miaominal

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.isToggleable
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performClick
import dev.akko.miaominal.data.BusinessSettings
import dev.akko.miaominal.data.FakeTerminalSession
import dev.akko.miaominal.data.ForwardFailureKindModel
import dev.akko.miaominal.data.ForwardFailureModel
import dev.akko.miaominal.data.ForwardKind
import dev.akko.miaominal.data.ForwardPhaseModel
import dev.akko.miaominal.data.HostKeyDecision
import dev.akko.miaominal.data.InMemoryMiaominalRepository
import dev.akko.miaominal.data.MiaominalRepository
import dev.akko.miaominal.ui.MiaominalApp
import dev.akko.miaominal.ui.theme.DEFAULT_THEME_SEED
import dev.akko.miaominal.ui.theme.MiaominalTheme
import dev.akko.miaominal.ui.theme.ThemeMode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

/**
 * UI acceptance against the injected test repository: forwarding
 * start/stop, typed failure display with the failing node, the global trust
 * prompt, and the pull-based monitor sheet that pauses on close.
 */
class ForwardUiTest {
    @get:Rule
    val compose = createComposeRule()

    private val repository = InMemoryMiaominalRepository(
        hosts = TestFixtures.hosts(),
        statuses = TestFixtures.hostStatus(),
        forwards = TestFixtures.forwardRules(),
        settings = BusinessSettings(onboardingCompleted = true)
    )

    private fun setContent(target: MiaominalRepository = repository) {
        compose.setContent {
            MiaominalTheme {
                MiaominalApp(
                    repository = target,
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

    private fun openForwardPage() {
        compose.onNodeWithText("工具").performClick()
        compose.waitForIdle()
        compose.onNodeWithText("端口转发").performClick()
        compose.waitForIdle()
        compose.onNodeWithText("本地 PostgreSQL").assertIsDisplayed()
    }

    @Test
    fun theRuleSwitchStartsAndStopsTheForwardSession() {
        setContent()
        openForwardPage()

        // The stored fixture rule is enabled, but a stored flag never starts a
        // session: the switch mirrors the live runtime state and starts off.
        val ruleSwitch = compose.onAllNodes(isToggleable()).onFirst()
        ruleSwitch.assertIsOff()

        ruleSwitch.performClick()
        compose.waitForIdle()
        val session = repository.forwardSessions.value.single()
        assertEquals(ForwardPhaseModel.Listening, session.phase)
        assertEquals(TestFixtures.PRIMARY_HOST_ID, session.profileId)
        assertEquals("forward-1", session.ruleId)
        assertTrue(repository.forwards.value.single().enabled)
        assertTrue(compose.onAllNodesWithText("运行中").fetchSemanticsNodes().isNotEmpty())
        compose.onAllNodes(isToggleable()).onFirst().assertIsOn()

        compose.onAllNodes(isToggleable()).onFirst().performClick()
        compose.waitForIdle()
        assertTrue(repository.forwardSessions.value.none { it.phase.isActive })
        assertTrue(!repository.forwards.value.single().enabled)
    }

    @Test
    fun theOpenInBrowserActionIsOfferedForLocalRules() {
        setContent()
        openForwardPage()
        compose.onNodeWithContentDescription("展开详情").performClick()
        compose.waitForIdle()

        compose.onNodeWithText("在浏览器中打开").assertIsDisplayed()
    }

    @Test
    fun theOpenInBrowserActionIsHiddenForRemoteRules() {
        val remoteRule = TestFixtures.forwardRules().first().copy(
            id = "forward-remote",
            kind = ForwardKind.Remote,
            label = "远端 Web"
        )
        val remoteRepository = InMemoryMiaominalRepository(
            hosts = TestFixtures.hosts(),
            statuses = TestFixtures.hostStatus(),
            forwards = listOf(remoteRule),
            settings = BusinessSettings(onboardingCompleted = true)
        )

        setContent(remoteRepository)
        compose.onNodeWithText("工具").performClick()
        compose.waitForIdle()
        compose.onNodeWithText("端口转发").performClick()
        compose.waitForIdle()
        compose.onNodeWithText("远端 Web").assertIsDisplayed()
        compose.onNodeWithContentDescription("展开详情").performClick()
        compose.waitForIdle()

        compose.onNodeWithText("在浏览器中打开").assertDoesNotExist()
    }

    @Test
    fun rulesWithTheSameIdOnDifferentHostsRender() {
        // The Rust side numbers rules per host ("pf-1", "pf-2", ...), so two hosts
        // can legitimately share one rule id. The list must key by host + rule
        // instead of the bare id, or Compose aborts the page with a duplicate key.
        val sharedId = "pf-1"
        val duplicateRules = listOf(
            TestFixtures.forwardRules().first().copy(id = sharedId),
            TestFixtures.forwardRules().first().copy(
                id = sharedId,
                profileId = "host-2",
                label = "NAS 面板",
                listenPort = 18080,
                targetHost = "192.168.1.8",
                targetPort = 8080
            )
        )
        val duplicated = InMemoryMiaominalRepository(
            hosts = TestFixtures.hosts(),
            statuses = TestFixtures.hostStatus(),
            forwards = duplicateRules,
            settings = BusinessSettings(onboardingCompleted = true)
        )

        setContent(duplicated)
        compose.onNodeWithText("工具").performClick()
        compose.waitForIdle()
        compose.onNodeWithText("端口转发").performClick()
        compose.waitForIdle()

        compose.onNodeWithText("本地 PostgreSQL").assertIsDisplayed()
        compose.onNodeWithText("NAS 面板").assertIsDisplayed()
    }

    @Test
    fun aTypedFailureShowsTheFailingNode() {
        val session = repository.seedForwardSession(
            profileId = TestFixtures.PRIMARY_HOST_ID,
            ruleId = "forward-1",
            phase = ForwardPhaseModel.Failed,
            label = "本地 PostgreSQL"
        )
        session.failure = ForwardFailureModel(
            kind = ForwardFailureKindModel.PortOccupied,
            nodeLabel = "jump-2 (2/3)",
            hopIndex = 2,
            hopTotal = 3,
            message = "failed to bind on 127.0.0.1:15432"
        )
        repository.refreshForwardSnapshots()

        setContent()
        openForwardPage()
        compose.onNodeWithContentDescription("展开详情").performClick()
        compose.waitForIdle()

        compose.onNodeWithText("失败").assertIsDisplayed()
        compose.onNodeWithText("监听端口被占用").assertIsDisplayed()
        compose.onNodeWithText("jump-2 (2/3)").assertIsDisplayed()
    }

    @Test
    fun theTrustPromptIsGlobalAndAnswersTheCurrentRequest() {
        val session = repository.seedForwardSession(
            profileId = TestFixtures.PRIMARY_HOST_ID,
            ruleId = "forward-1",
            phase = ForwardPhaseModel.WaitingForTrust,
            label = "本地 PostgreSQL"
        )
        session.awaitTrust(requestId = 21L)
        repository.refreshForwardSnapshots()

        // The prompt appears even before the forwarding page is opened.
        setContent()
        compose.onNodeWithText("确认主机密钥").assertIsDisplayed()
        compose.onNodeWithText("SHA256:forward", substring = true).assertIsDisplayed()

        compose.onNodeWithText("仅本次信任").performClick()
        compose.waitForIdle()

        assertEquals(listOf(21L to HostKeyDecision.AcceptOnce), session.hostKeyAnswers)
        assertTrue(session.request == null)
    }
    @Test
    fun theMonitorSheetPullsSamplesAndPausesOnClose() {
        setContent()

        compose.onNodeWithText(TestFixtures.PRIMARY_HOST_NAME).performClick()
        compose.onNodeWithText("连接").performScrollTo().performClick()
        compose.waitUntil(timeoutMillis = 15_000) {
            compose.onAllNodesWithTag("terminal-surface").fetchSemanticsNodes().isNotEmpty()
        }
        val session = latestSession()

        compose.onNodeWithContentDescription("查看监控").performClick()
        compose.waitForIdle()
        // Wait for the first poll to enable sampling.
        compose.waitUntil(timeoutMillis = 5_000) { session.monitoringEnabled }
        session.emitMonitor(cpuPercent = 42.5)

        compose.waitUntil(timeoutMillis = 10_000) {
            compose.onAllNodesWithText("CPU").fetchSemanticsNodes().isNotEmpty()
        }
        assertTrue(
            compose.onAllNodesWithText("42.5%", substring = true).fetchSemanticsNodes().isNotEmpty()
        )

        // Closing the sheet pauses sampling in the backend.
        androidx.test.espresso.Espresso.pressBack()
        compose.waitUntil(timeoutMillis = 5_000) { !session.monitoringEnabled }
    }

    @Test
    fun theMonitorSheetShowsTypedFailuresInsteadOfRawStrings() {
        setContent()

        compose.onNodeWithText(TestFixtures.PRIMARY_HOST_NAME).performClick()
        compose.onNodeWithText("连接").performScrollTo().performClick()
        compose.waitUntil(timeoutMillis = 15_000) {
            compose.onAllNodesWithTag("terminal-surface").fetchSemanticsNodes().isNotEmpty()
        }
        val session = latestSession()

        compose.onNodeWithContentDescription("查看监控").performClick()
        compose.waitUntil(timeoutMillis = 5_000) { session.monitoringEnabled }
        // The failure arrives while sampling is active.
        session.failMonitor()
        compose.waitUntil(timeoutMillis = 10_000) {
            compose.onAllNodesWithText("远端系统暂不支持监控采样。").fetchSemanticsNodes().isNotEmpty()
        }
        androidx.test.espresso.Espresso.pressBack()
    }

    private fun latestSession(): FakeTerminalSession =
        repository.sessionHandle(repository.sessions.value.last().connectionId) as FakeTerminalSession
}
