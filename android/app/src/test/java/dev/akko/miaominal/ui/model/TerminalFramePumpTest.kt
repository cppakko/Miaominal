package dev.akko.miaominal.ui.model

import dev.akko.miaominal.data.FakeTerminalSession
import dev.akko.miaominal.data.SessionPhase
import dev.akko.miaominal.data.SessionSnapshotModel
import dev.akko.miaominal.data.TerminalFrameModel
import dev.akko.miaominal.data.TerminalSessionHandle
import dev.akko.miaominal.data.TerminalUpdateModel
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.*
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class TerminalFramePumpTest {
    @Test fun stateIsReadBeforeTheGridAndWaitUsesSessionRevisionInsteadOfFrameGeneration() = runTest {
        val fake = FakeTerminalSession(1, "test", phase = SessionPhase.Ready).apply { revision = 10 }
        val events = mutableListOf<String>()
        var waitedRevision = -1L
        val session = object : TerminalSessionHandle by fake {
            override fun snapshot(): SessionSnapshotModel {
                events += "state"
                return fake.snapshot()
            }
            override fun renderFrame(sinceFrameVersion: Long): TerminalFrameModel {
                events += "grid"
                val frame = fake.renderFrame(sinceFrameVersion).copy(revision = 500)
                fake.revision = 11 // Arrives while converting the grid.
                return frame
            }
            override suspend fun awaitUpdate(sinceRevision: Long, timeoutMs: Long): TerminalUpdateModel {
                waitedRevision = sinceRevision
                awaitCancellation()
            }
        }
        val worker = launch {
            pumpTerminalFrames(session, {}, {}, {}, StandardTestDispatcher(testScheduler))
        }
        runCurrent()
        assertEquals(listOf("state", "grid"), events)
        assertEquals(10L, waitedRevision)
        worker.cancel()
    }

    @Test fun idleTimeoutsDoNotPullOrRedrawAndCancellationStopsWaiting() = runTest {
        val fake = FakeTerminalSession(1, "test", phase = SessionPhase.Ready)
        var waits = 0
        var draws = 0
        val session = object : TerminalSessionHandle by fake {
            override suspend fun awaitUpdate(sinceRevision: Long, timeoutMs: Long): TerminalUpdateModel {
                waits++
                delay(timeoutMs)
                return TerminalUpdateModel(sinceRevision, SessionPhase.Ready, idle = true, closed = false)
            }
        }
        val worker = launch {
            pumpTerminalFrames(session, {}, { draws++ }, {}, StandardTestDispatcher(testScheduler))
        }
        runCurrent()
        advanceTimeBy(1200)
        runCurrent()
        assertEquals(4, waits)
        assertEquals(1L, fake.renderCalls)
        assertEquals(1, draws)
        worker.cancel()
        runCurrent()
        advanceTimeBy(1200)
        assertEquals(4, waits)
    }

    @Test fun pressureIsCoalescedIntoOneLatestPullPerDisplayTick() = runTest {
        val fake = FakeTerminalSession(1, "test", phase = SessionPhase.Ready)
        val ticks = Channel<Unit>(Channel.RENDEZVOUS)
        val revisions = mutableListOf<Long>()
        val session = object : TerminalSessionHandle by fake {
            override suspend fun awaitUpdate(sinceRevision: Long, timeoutMs: Long) =
                TerminalUpdateModel(sinceRevision + 1, SessionPhase.Ready, idle = false, closed = false)
        }
        val worker = launch {
            pumpTerminalFrames(session, { ticks.receive() }, { revisions += it.revision }, {},
                StandardTestDispatcher(testScheduler))
        }
        runCurrent()
        assertEquals(0L, fake.renderCalls)
        ticks.send(Unit)
        runCurrent()
        repeat(100) { fake.revision++ }
        runCurrent()
        assertEquals(1L, fake.renderCalls)
        ticks.send(Unit)
        runCurrent()
        assertEquals(listOf(1L, 101L), revisions)
        assertEquals(2L, fake.renderCalls)
        worker.cancel()
        ticks.cancel()
    }

    @Test fun terminalStateDeliversTheLastFrameThenStopsWithoutPolling() = runTest {
        val fake = FakeTerminalSession(1, "test", phase = SessionPhase.Closed)
        var draws = 0
        var phase: SessionPhase? = null
        val worker = launch {
            pumpTerminalFrames(fake, {}, { draws++ }, { phase = it.phase }, StandardTestDispatcher(testScheduler))
        }
        runCurrent()
        assertTrue(worker.isCompleted)
        assertEquals(1, draws)
        assertEquals(SessionPhase.Closed, phase)
    }

    @Test fun renderingFailureStillDeliversTheConnectionFailureAndStops() = runTest {
        val fake = FakeTerminalSession(1, "test", phase = SessionPhase.Failed).apply { failure = "network lost" }
        val session = object : TerminalSessionHandle by fake {
            override fun renderFrame(sinceFrameVersion: Long): TerminalFrameModel = error("renderer closed")
        }
        var state: SessionSnapshotModel? = null
        var draws = 0
        val worker = launch {
            pumpTerminalFrames(session, {}, { draws++ }, { state = it }, StandardTestDispatcher(testScheduler))
        }
        runCurrent()
        assertTrue(worker.isCompleted)
        assertEquals("network lost", state?.failure)
        assertEquals(0, draws)
    }
}
