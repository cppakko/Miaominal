package dev.akko.miaominal

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.NativeBackendClient
import dev.akko.miaominal.data.NativeBackendState
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * P1 acceptance: the application-scoped client owns a single backend, so page
 * and Activity recreation never creates another native handle.
 */
@RunWith(AndroidJUnit4::class)
class NativeBackendClientTest {
    @get:Rule
    val compose = createEmptyComposeRule()

    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    private suspend fun awaitRunning(): NativeBackendState.Running =
        withTimeout(30_000) {
            NativeBackendClient.get(context).state.first { it is NativeBackendState.Running }
        } as NativeBackendState.Running

    @Test
    fun activityRecreationReusesTheSameBackend() = runBlocking {
        val client = NativeBackendClient.get(context)
        val running = awaitRunning()

        // The accessor used by every composition returns the same instance.
        assertSame(client, NativeBackendClient.get(context))

        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.recreate()
        }

        val afterRecreation = client.state.value as NativeBackendState.Running
        assertEquals(running.generation, afterRecreation.generation)
        assertTrue(afterRecreation.sequence >= running.sequence)
    }

    @Test
    fun restartReplacesTheHandleOnlyWhenRequested() = runBlocking {
        val client = NativeBackendClient.get(context)
        val before = awaitRunning()

        val restarted = client.restart() as NativeBackendState.Running

        assertEquals(before.generation + 1, restarted.generation)
        assertTrue(restarted.abiVersion.startsWith("miaominal-android/"))
        assertTrue(restarted.dataDir.isNotBlank())
        assertTrue(restarted.cacheDir.isNotBlank())

        // A further ensureStarted must not create yet another backend.
        client.ensureStarted()
        val after = client.state.value as NativeBackendState.Running
        assertEquals(restarted.generation, after.generation)
    }

    @Test
    fun cancellingTheCallerDoesNotInterruptARestart() = runBlocking {
        val client = NativeBackendClient.get(context)
        val before = awaitRunning()

        // Simulate a page-scoped caller (rememberCoroutineScope in the About card).
        val pageScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        val caller = pageScope.launch { client.restart() }
        // Wait until the replacement is in flight, then tear the page down.
        withTimeout(30_000) {
            client.state.first {
                it is NativeBackendState.Starting || it is NativeBackendState.Stopped
            }
        }
        caller.cancelAndJoin()
        pageScope.cancel()

        // Cleanup and startup belong to the application scope: the restart must
        // still finish and leave exactly one running backend.
        val after = withTimeout(30_000) {
            client.state.first {
                it is NativeBackendState.Running && it.generation == before.generation + 1
            }
        } as NativeBackendState.Running

        assertEquals(before.generation + 1, after.generation)
        assertTrue(after.sequence >= 1uL)
    }

    @Test
    fun aboutPageShowsTheBackendStateAndRestartsIt() = runBlocking<Unit> {
        val client = NativeBackendClient.get(context)
        val before = awaitRunning()

        // Business settings live in the backend now; skip onboarding through
        // the same typed interface the UI uses.
        client.currentBackend()?.updateSettings(
            dev.akko.miaominal.nativebridge.SettingsUpdate(
                language = null,
                recentHostCount = null,
                autoMonitoring = null,
                monitorHistory = null,
                closeLastSessionBehavior = null,
                vaultAutoLockMinutes = null,
                onboardingCompleted = true,
                terminalFont = null,
                cjkFallbackFont = null,
                terminalFontSize = null,
                terminalLineHeight = null,
                themeId = null,
                seedColor = null
            )
        )
        ActivityScenario.launch(MainActivity::class.java).use {
            compose.onNode(hasText("设置") and hasClickAction()).performClick()
            compose.onNode(hasText("关于 Miaominal") and hasClickAction()).performClick()

            // The backend card is developer diagnostics: hidden until the
            // version badge is tapped seven times.
            compose.onNodeWithText("诊断信息").assertDoesNotExist()
            compose.onNodeWithText("移动应用版本").assertIsDisplayed()
            compose.onNodeWithText("Miaominal 后端版本").assertIsDisplayed()
            val versionBadge = compose.onNodeWithTag("about-app-version-badge")
            repeat(7) { versionBadge.performClick() }

            compose.onNodeWithText("诊断信息").assertIsDisplayed()
            compose.onNodeWithText("事件序号").assertExists()
            compose.onNode(hasText("第 ${before.generation} 代", substring = true)).assertExists()

            compose.onNodeWithText("重启后端").performScrollTo().performClick()
            compose.waitUntil(timeoutMillis = 30_000) {
                (client.state.value as? NativeBackendState.Running)?.generation ==
                    before.generation + 1
            }
            compose.onNode(hasText("第 ${before.generation + 1} 代", substring = true))
                .assertExists()
        }
    }
}
