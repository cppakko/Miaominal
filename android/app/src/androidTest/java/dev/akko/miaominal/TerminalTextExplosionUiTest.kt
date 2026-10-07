package dev.akko.miaominal

import androidx.compose.foundation.layout.requiredSize
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalViewConfiguration
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.ui.components.ExplosionSelectionPane
import dev.akko.miaominal.ui.components.TerminalExplosionSelection
import dev.akko.miaominal.ui.components.TerminalTextExplosionDialog
import dev.akko.miaominal.ui.theme.MiaominalTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

/** Device execution requires the repository's designated-device consent. */
class TerminalTextExplosionUiTest {
    @get:Rule val compose = createComposeRule()

    private fun control(tag: String): SemanticsNodeInteraction {
        val node = compose.onNodeWithTag(tag)
        if (compose.onAllNodesWithTag("explosion-pinned-controls").fetchSemanticsNodes().isEmpty()) {
            node.performScrollTo()
        }
        return node
    }

    @Test fun longPressDragTogglesMixedWordsOnceEvenWhenRetracingAndUndoesAsOneEdit() {
        val document = TerminalExplosionSelection("one two three")
        document.setTokenSelected(1, true)
        var holdDuration = 0L
        compose.setContent {
            holdDuration = LocalViewConfiguration.current.longPressTimeoutMillis + 100
            MiaominalTheme(dynamicColor = false) {
                ExplosionSelectionPane(document, rememberLazyListState(), Modifier.requiredSize(320.dp, 480.dp))
            }
        }
        val list = compose.onNodeWithTag("explosion-tokens")
        val origin = list.fetchSemanticsNode().boundsInRoot.topLeft
        val first = compose.onNodeWithTag("explosion-token-0").fetchSemanticsNode().boundsInRoot.center - origin
        val second = compose.onNodeWithTag("explosion-token-1").fetchSemanticsNode().boundsInRoot.center - origin
        val third = compose.onNodeWithTag("explosion-token-2").fetchSemanticsNode().boundsInRoot.center - origin
        // Keep the active drag's frame loop from advancing indefinitely between touch batches.
        compose.mainClock.autoAdvance = false
        list.performTouchInput { down(first) }
        compose.mainClock.advanceTimeBy(holdDuration)
        list.performTouchInput {
            advanceEventTime(holdDuration)
            moveTo(second)
            moveTo(third)
            moveTo(first)
            up()
        }
        compose.mainClock.autoAdvance = true
        compose.runOnIdle { assertEquals("one three", document.resultText()) }
        compose.onNodeWithTag("explosion-undo").performClick()
        compose.runOnIdle { assertEquals("two", document.resultText()) }
    }

    @Test fun wordSelectionAndUndoUpdateCopyAvailabilityWithoutPreviewOrSplitControls() {
        val document = TerminalExplosionSelection("git   status --short")
        compose.setContent { MiaominalTheme { TerminalTextExplosionDialog(document, onDismiss = {}) } }
        control("explosion-clear").assertIsNotEnabled()
        compose.onNodeWithTag("explosion-token-0").performScrollTo().assert(
            SemanticsMatcher.expectValue(SemanticsProperties.ToggleableState, ToggleableState.Off))
        compose.onNodeWithTag("explosion-copy-return").assertIsNotEnabled()
        compose.onNodeWithTag("explosion-token-0").performScrollTo().performClick()
        compose.onNodeWithTag("explosion-copy-return").assertIsEnabled()
        assertEquals("git", document.resultText())
        listOf("explosion-result", "explosion-preview-tab", "explosion-select-tab",
            "explosion-dual-pane", "explosion-mode-Characters", "explosion-mode-Words", "explosion-join").forEach {
            compose.onNodeWithTag(it).assertDoesNotExist()
        }
        control("explosion-undo").performClick()
        assertEquals("", document.resultText())
        compose.onNodeWithTag("explosion-copy-return").assertIsNotEnabled()
    }

    @Test fun wholeWordSelectionSurvivesStateRestoration() {
        val document = TerminalExplosionSelection("hello world")
        document.setTokenSelected(0, true)
        val restoration = StateRestorationTester(compose)
        restoration.setContent { MiaominalTheme { TerminalTextExplosionDialog(document, onDismiss = {}) } }
        restoration.emulateSavedInstanceStateRestore()
        compose.onNodeWithTag("explosion-token-0").performScrollTo().assert(
            SemanticsMatcher.expectValue(SemanticsProperties.ToggleableState, ToggleableState.On))
        compose.onNodeWithTag("explosion-token-1").performScrollTo().assert(
            SemanticsMatcher.expectValue(SemanticsProperties.ToggleableState, ToggleableState.Off))
        compose.onNodeWithTag("explosion-copy-return").assertIsEnabled()
        assertEquals("hello", document.resultText())
    }

    @Test fun normalHeightKeepsAllSelectionActionsReachableAfterScrolling() {
        val document = TerminalExplosionSelection((1..5000).joinToString(" ") { "word$it" })
        compose.setContent {
            MiaominalTheme(dynamicColor = false) {
                ExplosionSelectionPane(document, rememberLazyListState(), Modifier.requiredSize(320.dp, 480.dp))
            }
        }
        compose.onNodeWithTag("explosion-pinned-controls").assertIsDisplayed()
        compose.onNodeWithTag("explosion-tokens").performScrollToIndex(50)
        compose.onNodeWithTag("explosion-all").assertIsDisplayed().performClick()
        compose.onNodeWithTag("explosion-clear").assertIsDisplayed().assertIsEnabled()
        compose.onNodeWithTag("explosion-undo").assertIsDisplayed().performClick()
        assertTrue(document.isEmpty)
    }

    @Test fun shortPaneScrollsControlsWithTheTextAndCanReturnToActions() {
        val document = TerminalExplosionSelection((1..5000).joinToString(" ") { "word$it" })
        compose.setContent {
            MiaominalTheme(dynamicColor = false) {
                ExplosionSelectionPane(document, rememberLazyListState(), Modifier.requiredSize(360.dp, 320.dp))
            }
        }
        compose.onNodeWithTag("explosion-pinned-controls").assertDoesNotExist()
        compose.onNodeWithTag("explosion-scrolling-controls").assertExists()
        compose.onNodeWithTag("explosion-tokens").performScrollToIndex(50)
        compose.onNodeWithTag("explosion-all").performScrollTo().assertIsDisplayed().performClick()
        assertTrue(document.allSelected)
        compose.onNodeWithTag("explosion-clear").performScrollTo().performClick()
        assertTrue(document.isEmpty)
    }

    @Test fun longPathWrapsAtLargeFontWithoutChangingItsBoundsWhenSelected() {
        val path = "/home/" + "very-long-directory-name/".repeat(8) + "file.txt"
        val document = TerminalExplosionSelection(path)
        compose.setContent {
            val density = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(density.density, fontScale = 2f)) {
                MiaominalTheme(dynamicColor = false) {
                    ExplosionSelectionPane(document, rememberLazyListState(), Modifier.requiredSize(320.dp, 479.dp))
                }
            }
        }
        val token = compose.onNodeWithTag("explosion-token-0").performScrollTo().assertTextEquals(path)
        val layouts = mutableListOf<TextLayoutResult>()
        token.performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(layouts) }
        assertTrue(layouts.single().lineCount > 1)
        assertEquals(path.length, layouts.single().getLineEnd(layouts.single().lineCount - 1))
        assertTrue(!layouts.single().hasVisualOverflow)
        val before = token.fetchSemanticsNode().boundsInRoot
        token.performClick()
        compose.waitForIdle()
        assertEquals(before, token.fetchSemanticsNode().boundsInRoot)
        assertEquals(path, document.resultText())
    }

    @Test fun thousandsOfWordsComposeOnlyVisibleBatches() {
        val document = TerminalExplosionSelection((1..5000).joinToString(" ") { "word$it" })
        compose.setContent { MiaominalTheme { TerminalTextExplosionDialog(document, onDismiss = {}) } }
        val tokenMatcher = SemanticsMatcher("word block") {
            it.config.getOrNull(SemanticsProperties.TestTag)?.startsWith("explosion-token-") == true
        }
        val composed = compose.onAllNodes(tokenMatcher, useUnmergedTree = true).fetchSemanticsNodes().size
        assertTrue("Expected bounded composition, got $composed blocks", composed in 1..192)
        assertEquals(0, document.selectedTokenCount)
        compose.onNodeWithTag("explosion-copy-return").assertIsNotEnabled()
    }
}
