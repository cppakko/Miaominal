package dev.akko.miaominal

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.SessionProfile
import dev.akko.miaominal.ui.components.JumpChainEditor
import dev.akko.miaominal.ui.theme.MiaominalTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class JumpChainEditorTest {
    @get:Rule val compose = createComposeRule()
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private var chain by mutableStateOf(listOf("a", "b"))
    private val hosts = listOf(
        SessionProfile(id = "a", name = "Bastion A", host = "a.example", username = "root"),
        SessionProfile(id = "b", name = "Bastion B", host = "b.example", username = "root"),
        SessionProfile(id = "target", name = "Destination", host = "target.example", username = "root")
    )

    private fun openEditor() {
        compose.setContent {
            MiaominalTheme {
                val listState = rememberLazyListState()
                var viewport by remember { mutableStateOf(Rect.Zero) }
                LazyColumn(state = listState, modifier = Modifier.fillMaxSize()
                    .onGloballyPositioned { viewport = it.boundsInRoot() }) {
                    item {
                        JumpChainEditor(hosts, "target", "Destination", "target.example", chain,
                            { chain = it }, listState, viewport, enabled = true)
                    }
                }
            }
        }
    }

    @Test fun arrowsReorderAndRemoveOnlyTheChosenHop() {
        openEditor()
        compose.onNodeWithContentDescription(context.getString(R.string.jump_move_up, "Bastion A"))
            .assertIsNotEnabled()
        compose.onNodeWithContentDescription(context.getString(R.string.jump_move_down, "Bastion A"))
            .performClick()
        compose.runOnIdle { assertEquals(listOf("b", "a"), chain) }
        compose.onNodeWithContentDescription(context.getString(R.string.jump_remove, "Bastion B"))
            .performClick()
        compose.runOnIdle { assertEquals(listOf("a"), chain) }
    }

    @Test fun draggingWithoutLongPressChangesTheRouteOrder() {
        openEditor()
        val next = compose.onNodeWithTag("jump-hop-b").fetchSemanticsNode().boundsInRoot
        val handle = compose.onNodeWithTag("jump-drag-a")
        val bounds = handle.fetchSemanticsNode().boundsInRoot
        handle.performTouchInput {
            down(center)
            moveTo(center + Offset(0f, next.center.y - bounds.center.y + 24f), delayMillis = 300)
            advanceEventTime(100)
            up()
        }
        compose.runOnIdle { assertEquals(listOf("b", "a"), chain) }
    }

    @Test fun pickerExcludesDestinationAndAddsHostsOnlyOnConfirmation() {
        chain = emptyList()
        openEditor()
        compose.onNodeWithText(context.getString(R.string.forward_jump_add)).performScrollTo().performClick()
        compose.onNodeWithText("Bastion B").performClick()
        compose.onNodeWithText("Bastion A").performClick()
        compose.runOnIdle { assertEquals(emptyList<String>(), chain) }
        compose.onNodeWithTag("jump-candidate-target").assertDoesNotExist()
        compose.onNodeWithText(context.getString(R.string.jump_add_selected, 2)).performClick()
        compose.runOnIdle { assertEquals(listOf("b", "a"), chain) }
    }
}
