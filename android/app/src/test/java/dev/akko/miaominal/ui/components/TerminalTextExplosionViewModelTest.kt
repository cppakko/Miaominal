package dev.akko.miaominal.ui.components

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.Assert.*
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class TerminalTextExplosionViewModelTest {
    @Test fun closingDuringParsingCannotReopenTheDocumentOrKeepItsText() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        Dispatchers.setMain(dispatcher)
        try {
            val model = TerminalTextExplosionViewModel(dispatcher)
            model.open("private terminal output")
            assertNotNull(model.source)
            model.close()
            advanceUntilIdle()
            assertNull(model.source)
            assertNull(model.document)
        } finally { Dispatchers.resetMain() }
    }

    @Test fun aNewRequestReplacesOldParsingAndKeepsSelectionUntilExplicitClose() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        Dispatchers.setMain(dispatcher)
        try {
            val model = TerminalTextExplosionViewModel(dispatcher)
            model.open("old document")
            model.open("new document")
            advanceUntilIdle()
            assertEquals("new document", model.document?.source)
            val document = requireNotNull(model.document)
            assertTrue(document.isEmpty)
            document.setTokenSelected(0, true)
            assertSame(document, model.document)
            assertEquals("new", model.document?.resultText())
            model.close()
            assertNull(model.source)
            assertNull(model.document)
        } finally { Dispatchers.resetMain() }
    }
}
