package dev.akko.miaominal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.requiredSize
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.unit.dp
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.BusinessSettings
import dev.akko.miaominal.data.InMemoryMiaominalRepository
import dev.akko.miaominal.ui.MiaominalApp
import dev.akko.miaominal.ui.theme.DEFAULT_THEME_SEED
import dev.akko.miaominal.ui.theme.MiaominalTheme
import dev.akko.miaominal.ui.theme.ThemeMode
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

/** Real navigation shell with a fake repository; no native sessions or saved user data. */
class SettingsNavigationTransitionTest {
    @get:Rule val compose = createComposeRule()
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private var wide by mutableStateOf(false)

    private fun showApp() {
        val repository = InMemoryMiaominalRepository(
            settings = BusinessSettings(onboardingCompleted = true)
        )
        compose.setContent {
            Box(Modifier.requiredSize(if (wide) 900.dp else 360.dp, 640.dp)) {
                MiaominalTheme(dynamicColor = false) {
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
        }
        compose.waitForIdle()
        compose.mainClock.autoAdvance = false
    }

    private fun settle() {
        compose.mainClock.advanceTimeBy(450)
        compose.waitForIdle()
    }

    @Test fun proxyManagerReturnsToTheConnectionsPagePosition() {
        showApp()
        compose.onNodeWithText(context.getString(R.string.nav_settings)).performClick(); settle()
        compose.onNodeWithText(context.getString(R.string.setting_connections)).performClick(); settle()
        compose.onNodeWithTag("connections-settings-list").performScrollToNode(hasText(context.getString(R.string.settings_manage_proxies)))
        settle()
        compose.onNodeWithText(context.getString(R.string.settings_manage_proxies)).performClick(); settle()
        compose.onNodeWithText(context.getString(R.string.tool_proxies)).assertIsDisplayed()
        compose.onNodeWithContentDescription(context.getString(R.string.action_back)).performClick(); settle()
        compose.onNodeWithTag("setting-detail:Connections").assertIsDisplayed()
        compose.onNodeWithText(context.getString(R.string.settings_manage_proxies)).assertIsDisplayed()
    }

    @Test fun switchingTabsKeepsChromeAndContentBoundsEvenWhenInterrupted() {
        showApp()
        val tags = listOf("main-top-bar", "main-navigation-bar", "main-content")
        val bounds = tags.associateWith { compose.onNodeWithTag(it).fetchSemanticsNode().boundsInRoot }
        // Interrupt Settings while both outgoing/incoming tab contents are composed.
        for (destination in listOf(R.string.nav_settings, R.string.nav_tools, R.string.nav_settings, R.string.nav_hosts)) {
            compose.onNodeWithText(context.getString(destination)).performClick()
            repeat(4) {
                compose.mainClock.advanceTimeBy(32)
                compose.waitForIdle()
                tags.forEach { tag ->
                    compose.onAllNodesWithTag(tag).assertCountEquals(1)
                    assertEquals("$tag must not jump during a tab transition", bounds[tag],
                        compose.onNodeWithTag(tag).fetchSemanticsNode().boundsInRoot)
                }
            }
        }
        settle()
    }

    @Test fun compactDetailAnimatesAboveTheShellAndReturnsToTheSameListBounds() {
        showApp()
        compose.onNodeWithText(context.getString(R.string.nav_settings)).performClick()
        settle()
        val listBounds = compose.onNodeWithTag("settings-list").fetchSemanticsNode().boundsInRoot
        compose.onNodeWithText(context.getString(R.string.setting_appearance)).performClick()
        compose.mainClock.advanceTimeBy(80)
        compose.onNodeWithTag("setting-detail:Appearance").assertExists()
        compose.onAllNodesWithTag("main-navigation-bar").assertCountEquals(1)
        assertEquals(listBounds, compose.onNodeWithTag("settings-list").fetchSemanticsNode().boundsInRoot)
        settle()
        compose.onNodeWithContentDescription(context.getString(R.string.action_back)).performClick()
        compose.mainClock.advanceTimeBy(80)
        compose.onNodeWithTag("setting-detail:Appearance").assertExists()
        settle()
        compose.onNodeWithTag("setting-detail:Appearance").assertDoesNotExist()
        assertEquals(listBounds, compose.onNodeWithTag("settings-list").fetchSemanticsNode().boundsInRoot)
    }

    @Test fun resizingDuringDetailEntryKeepsOneSavedDetailAndTheSelection() {
        showApp()
        compose.onNodeWithText(context.getString(R.string.nav_settings)).performClick()
        settle()
        compose.onNodeWithText(context.getString(R.string.setting_connections)).performClick()
        compose.mainClock.advanceTimeBy(80)
        compose.runOnIdle { wide = true }
        settle()
        compose.onAllNodesWithTag("setting-detail:Connections").assertCountEquals(1)
        compose.onAllNodesWithTag("settings-list").assertCountEquals(1)
        compose.runOnIdle { wide = false }
        settle()
        compose.onAllNodesWithTag("setting-detail:Connections").assertCountEquals(1)
        compose.onAllNodesWithTag("main-navigation-bar").assertCountEquals(1)
    }
}
