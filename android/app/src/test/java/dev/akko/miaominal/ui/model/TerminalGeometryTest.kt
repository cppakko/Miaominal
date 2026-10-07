package dev.akko.miaominal.ui.model

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.*
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class TerminalGeometryTest {
    @Test fun keyboardAnimationCommitsOnlyItsFinalGrid() {
        val policy = TerminalGeometryPolicy()
        val initial = TerminalGridSize(80, 40)
        assertEquals(initial, policy.takeReport(initial))
        policy.beginImeAnimation()
        (39 downTo 20).forEach { assertNull(policy.takeReport(TerminalGridSize(80, it))) }
        policy.endImeAnimation()
        val final = TerminalGridSize(80, 20)
        assertEquals(final, policy.takeReport(final))
        assertNull(policy.takeReport(final))
    }

    @Test fun interruptedOrOverlappingAnimationsDoNotCommitAnIntermediateSize() {
        val policy = TerminalGeometryPolicy()
        val initial = TerminalGridSize(80, 40)
        policy.takeReport(initial)
        policy.beginImeAnimation()
        policy.beginImeAnimation()
        policy.endImeAnimation()
        assertNull(policy.takeReport(TerminalGridSize(80, 25)))
        policy.endImeAnimation()
        // Cancelling keyboard opening returns to the original size, without resize.
        assertNull(policy.takeReport(initial))
        assertFalse(policy.animating)
    }

    @Test fun switchingSessionAndReattachingStillReportTheCurrentGrid() {
        val policy = TerminalGeometryPolicy()
        val size = TerminalGridSize(80, 24)
        policy.takeReport(size)
        policy.resetSession()
        assertTrue(policy.needsInitialReport)
        assertEquals(size, policy.takeReport(size))
        policy.beginImeAnimation()
        assertEquals(size, policy.takeReport(size, force = true))
        policy.detach()
        assertFalse(policy.animating)
        assertEquals(size, policy.takeReport(size))
    }

    @Test fun pixelChangesWithinTheSameCellGridDoNotResize() {
        val policy = TerminalGeometryPolicy()
        val size = TerminalGridSize(80, 24)
        policy.takeReport(size)
        repeat(60) { assertNull(policy.takeReport(size)) }
        assertNull(policy.takeReport(TerminalGridSize(0, 0)))
        assertEquals(TerminalGridSize(79, 24), policy.takeReport(TerminalGridSize(79, 24)))
    }

    @Test fun busyResizeDropsIntermediateRequestsAndAppliesTheLatestSerially() = runTest {
        val queue = TerminalResizeQueue()
        val firstCompleted = CompletableDeferred<Unit>()
        val calls = mutableListOf<TerminalGridSize>()
        val worker = launch {
            queue.consume { columns, rows ->
                calls += TerminalGridSize(columns, rows)
                if (calls.size == 1) firstCompleted.await()
            }
        }
        queue.request(80, 40)
        runCurrent()
        (39 downTo 20).forEach { queue.request(80, it) }
        runCurrent()
        assertEquals(listOf(TerminalGridSize(80, 40)), calls)
        firstCompleted.complete(Unit)
        runCurrent()
        assertEquals(listOf(TerminalGridSize(80, 40), TerminalGridSize(80, 20)), calls)
        queue.request(80, 20)
        runCurrent()
        assertEquals(2, calls.size)
        queue.close()
        worker.join()
    }

    @Test fun leavingSessionDropsQueuedResizeAndDoesNotAffectTheNewSession() = runTest {
        val oldQueue = TerminalResizeQueue()
        oldQueue.request(80, 10)
        oldQueue.close()
        oldQueue.request(80, 20)
        val oldCalls = mutableListOf<TerminalGridSize>()
        launch { oldQueue.consume { columns, rows -> oldCalls += TerminalGridSize(columns, rows) } }
        val newQueue = TerminalResizeQueue()
        val newCalls = mutableListOf<TerminalGridSize>()
        val worker = launch { newQueue.consume { columns, rows -> newCalls += TerminalGridSize(columns, rows) } }
        newQueue.request(100, 30)
        runCurrent()
        assertTrue(oldCalls.isEmpty())
        assertEquals(listOf(TerminalGridSize(100, 30)), newCalls)
        newQueue.close()
        worker.join()
    }

    @Test fun failedResizeCanBeRetriedAndInvalidSizesAreIgnored() = runTest {
        val queue = TerminalResizeQueue()
        var attempts = 0
        val worker = launch {
            queue.consume { _, _ ->
                attempts++
                if (attempts == 1) error("disconnected during resize")
            }
        }
        queue.request(0, 0)
        runCurrent()
        assertEquals(0, attempts)
        queue.request(80, 24)
        runCurrent()
        queue.request(80, 24)
        runCurrent()
        assertEquals(2, attempts)
        queue.close()
        worker.join()
    }
}
