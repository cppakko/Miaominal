package dev.akko.miaominal.ui.components

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class WorkspaceBackHandlerTest {
    @Test fun cancelledPreviewDoesNotNavigate() = runBlocking {
        val gesture = BackGestureState()
        var commits = 0
        completeBackGesture(flow { emit(0.7f); throw CancellationException() }, gesture) { commits++ }
        assertEquals(0, commits); assertEquals(0f, gesture.progress)
    }
    @Test fun completedGestureCommitsOnceAndResets() = runBlocking {
        val gesture = BackGestureState()
        var commits = 0
        completeBackGesture(flowOf(0.1f, 0.4f, 0.9f), gesture) { commits++ }
        assertEquals(1, commits); assertEquals(0f, gesture.progress)
    }
}
