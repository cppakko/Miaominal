package dev.akko.miaominal

import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performClick
import androidx.test.espresso.Espresso
import dev.akko.miaominal.data.BusinessSettings
import dev.akko.miaominal.data.FakeTerminalSession
import dev.akko.miaominal.data.InMemoryMiaominalRepository
import dev.akko.miaominal.ui.MiaominalApp
import dev.akko.miaominal.ui.theme.DEFAULT_THEME_SEED
import dev.akko.miaominal.ui.theme.MiaominalTheme
import dev.akko.miaominal.ui.theme.ThemeMode
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread

/**
 * Display-clock backpressure: at most one pull per advanced Compose frame,
 * and leaving the terminal must stop the frame pump.
 *
 * A background thread keeps bumping the fake session revision so the pump can
 * never idle on its long-poll timeout; the observed pull rate is therefore the
 * frame coalescing behavior, independent of the device's refresh rate.
 */
class FrameBudgetUiTest {
    @get:Rule
    val compose = createComposeRule()

    private val repository = InMemoryMiaominalRepository(
        hosts = TestFixtures.hosts(),
        statuses = TestFixtures.hostStatus(),
        settings = BusinessSettings(onboardingCompleted = true)
    )

    @Test
    fun framePullsFollowDisplayTicksAndStopWhenLeaving() {
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
        compose.onNodeWithText(TestFixtures.PRIMARY_HOST_NAME).performClick()
        compose.onNodeWithText("连接").performScrollTo().performClick()
        compose.waitUntil(timeoutMillis = 15_000) {
            compose.onAllNodesWithTag("terminal-surface").fetchSemanticsNodes().isNotEmpty()
        }

        val session = repository.sessionHandle(repository.sessions.value.single().connectionId)
            as FakeTerminalSession
        val stopPressure = AtomicBoolean(false)
        val pressure = thread(name = "terminal-frame-pressure") {
            while (!stopPressure.get()) {
                session.revision += 1
                Thread.sleep(2)
            }
        }
        compose.mainClock.autoAdvance = false
        try {
            val firstCall = session.renderCalls
            repeat(DISPLAY_TICKS) {
                compose.mainClock.advanceTimeByFrame()
                Thread.sleep(16) // Let background conversion complete between display ticks.
            }
            val calls = session.renderCalls - firstCall
            assertTrue(
                "expected at most one pull per display tick: $calls pulls over $DISPLAY_TICKS ticks",
                calls in 1L..(DISPLAY_TICKS + 1).toLong()
            )
        } finally {
            stopPressure.set(true)
            pressure.join(2_000)
            compose.mainClock.autoAdvance = true
        }

        // Leaving the terminal disposes the frame pump.
        Espresso.pressBack()
        compose.waitForIdle()
        val afterLeaving = session.renderCalls
        Thread.sleep(1_000)
        assertTrue(
            "frame pump kept running after leaving the terminal",
            session.renderCalls - afterLeaving <= 2
        )
    }

    private companion object {
        const val DISPLAY_TICKS = 40
    }
}
