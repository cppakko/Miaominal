package dev.akko.miaominal

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Text
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import dev.akko.miaominal.ui.screens.SftpDirectoryContent
import dev.akko.miaominal.ui.theme.MiaominalTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class SftpDirectoryLoadingUiTest {
    @get:Rule val compose = createComposeRule()
    private val loading = mutableStateOf(false)
    private val hasEntries = mutableStateOf(true)
    private val error = mutableStateOf<String?>(null)

    private fun open() {
        compose.setContent {
            MiaominalTheme(dynamicColor = false) {
                SftpDirectoryContent(loading.value, hasEntries.value, error.value, Modifier.fillMaxSize()) {
                    Text(error.value ?: if (hasEntries.value) "Files" else "Empty", Modifier.testTag("directory-result"))
                }
            }
        }
        compose.mainClock.autoAdvance = false
    }

    @Test fun fastDirectoryRequestsNeverFlashALoadingOrEmptyState() {
        open()
        compose.runOnIdle { loading.value = true; hasEntries.value = false }
        compose.mainClock.advanceTimeBy(96)
        compose.onNodeWithTag("sftp-browser-loading").assertDoesNotExist()
        compose.onNodeWithText("Empty").assertDoesNotExist()
        compose.runOnIdle { loading.value = false; hasEntries.value = true }
        compose.mainClock.advanceTimeBy(1_000)
        compose.onNodeWithText("Files").assertIsDisplayed()
        compose.onNodeWithTag("sftp-browser-loading").assertDoesNotExist()
    }

    @Test fun visibleDirectoryLoadingCompletesItsEntranceBeforeShowingFiles() {
        open()
        compose.runOnIdle { loading.value = true; hasEntries.value = false }
        compose.mainClock.advanceTimeBy(320)
        compose.onNodeWithTag("sftp-browser-loading").assertIsDisplayed()
        compose.runOnIdle { loading.value = false; hasEntries.value = true }
        compose.mainClock.advanceTimeBy(160)
        compose.onNodeWithTag("sftp-browser-loading").assertIsDisplayed()
        compose.onNodeWithText("Files").assertDoesNotExist()
        compose.mainClock.advanceTimeBy(1_000)
        compose.onNodeWithText("Files").assertIsDisplayed()
        compose.onNodeWithTag("sftp-browser-loading").assertDoesNotExist()
    }

    @Test fun refreshingKeepsFilePositionsStableAsTheIndicatorAppearsAndLeaves() {
        open()
        val originalTop = compose.onNodeWithTag("directory-result").fetchSemanticsNode().boundsInRoot.top
        compose.runOnIdle { loading.value = true }
        compose.mainClock.advanceTimeBy(400)
        compose.onNodeWithTag("sftp-directory-refresh-progress").assertIsDisplayed()
        compose.onNodeWithText("Files").assertIsDisplayed()
        assertEquals(originalTop, compose.onNodeWithTag("directory-result").fetchSemanticsNode().boundsInRoot.top, 0.5f)
        compose.runOnIdle { loading.value = false }
        compose.mainClock.advanceTimeBy(96)
        compose.onNodeWithTag("sftp-directory-refresh-progress").assertIsDisplayed()
        compose.mainClock.advanceTimeBy(1_000)
        compose.onNodeWithTag("sftp-directory-refresh-progress").assertDoesNotExist()
        assertEquals(originalTop, compose.onNodeWithTag("directory-result").fetchSemanticsNode().boundsInRoot.top, 0.5f)
    }

    @Test fun aNewRequestCancelsThePendingExitAndErrorsBypassTheMinimumHold() {
        open()
        compose.runOnIdle { loading.value = true; hasEntries.value = false }
        compose.mainClock.advanceTimeBy(320)
        compose.runOnIdle { loading.value = false; hasEntries.value = true }
        compose.mainClock.advanceTimeBy(96)
        compose.runOnIdle { loading.value = true; hasEntries.value = false }
        compose.mainClock.advanceTimeBy(96)
        compose.onNodeWithTag("sftp-browser-loading").assertIsDisplayed()
        compose.onNodeWithText("Empty").assertDoesNotExist()
        compose.runOnIdle { loading.value = false; error.value = "Cannot open folder" }
        compose.mainClock.advanceTimeBy(48)
        compose.onNodeWithText("Cannot open folder").assertExists()
        // AnimatedContent retains the faded-out node until the incoming content finishes animating.
        compose.mainClock.advanceTimeBy(450)
        compose.onNodeWithText("Cannot open folder").assertIsDisplayed()
        compose.onNodeWithTag("sftp-browser-loading").assertDoesNotExist()
    }
}
