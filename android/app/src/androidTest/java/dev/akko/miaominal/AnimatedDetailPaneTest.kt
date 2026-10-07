package dev.akko.miaominal

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTextInput
import dev.akko.miaominal.ui.components.AnimatedDetailPane
import dev.akko.miaominal.ui.theme.MiaominalMotion
import dev.akko.miaominal.ui.theme.MiaominalTheme
import org.junit.Rule
import org.junit.Test

/** Fake content only; exercising animation retention also exercises the real saved slot ownership. */
class AnimatedDetailPaneTest {
    @get:Rule val compose = createComposeRule()
    private var selected by mutableStateOf<String?>(null)
    private var wide by mutableStateOf(false)

    private fun showPane() {
        compose.mainClock.autoAdvance = false
        compose.setContent {
            val states = rememberSaveableStateHolder()
            MiaominalTheme(dynamicColor = false) {
                AnimatedDetailPane(selected, wide, "appearance", listContent = {
                    states.SaveableStateProvider("list") {
                        Text("Settings", Modifier.testTag("settings-list"))
                    }
                }, detailContent = { destination ->
                    states.SaveableStateProvider(destination) {
                        var draft by rememberSaveable { mutableStateOf("") }
                        Column(Modifier.testTag("detail-$destination")) {
                            Text(destination)
                            BasicTextField(draft, { draft = it }, Modifier.testTag("draft-$destination"))
                        }
                    }
                })
            }
        }
        compose.waitForIdle()
    }

    private fun settle() {
        compose.mainClock.advanceTimeBy((MiaominalMotion.SystemSurfaceDurationMillis + 100).toLong())
        compose.waitForIdle()
    }

    @Test fun compactEnterAndReturnRetainOutgoingPageUntilTheTransitionEnds() {
        showPane()
        compose.runOnIdle { selected = "appearance" }
        compose.mainClock.advanceTimeBy(80)
        compose.onNodeWithTag("settings-list").assertExists()
        compose.onNodeWithTag("detail-appearance").assertExists()
        settle()
        compose.onNodeWithTag("settings-list").assertDoesNotExist()
        compose.runOnIdle { selected = null }
        compose.mainClock.advanceTimeBy(80)
        compose.onNodeWithTag("settings-list").assertExists()
        compose.onNodeWithTag("detail-appearance").assertExists()
        settle()
        compose.onNodeWithTag("detail-appearance").assertDoesNotExist()
    }

    @Test fun wideDetailSwitchKeepsOneListAndRestoresDraftOnReturn() {
        wide = true
        selected = "appearance"
        showPane()
        compose.onNodeWithTag("draft-appearance").performTextInput("retained draft")
        compose.runOnIdle { selected = "connections" }
        compose.mainClock.advanceTimeBy(80)
        compose.onAllNodesWithTag("settings-list").assertCountEquals(1)
        compose.onNodeWithTag("detail-appearance").assertExists()
        compose.onNodeWithTag("detail-connections").assertExists()
        settle()
        compose.runOnIdle { selected = "appearance" }
        settle()
        compose.onNodeWithTag("draft-appearance").assertTextEquals("retained draft")
    }

    @Test fun resizeDuringEnterDoesNotComposeTheSavedListTwiceOrLoseTheDraft() {
        showPane()
        compose.runOnIdle { selected = "appearance" }
        compose.mainClock.advanceTimeBy(80)
        compose.runOnIdle { wide = true }
        settle()
        compose.onAllNodesWithTag("settings-list").assertCountEquals(1)
        compose.onNodeWithTag("draft-appearance").performTextInput("resize draft")
        compose.runOnIdle { wide = false }
        settle()
        compose.onNodeWithTag("settings-list").assertDoesNotExist()
        compose.onNodeWithTag("draft-appearance").assertTextEquals("resize draft")
    }
}
