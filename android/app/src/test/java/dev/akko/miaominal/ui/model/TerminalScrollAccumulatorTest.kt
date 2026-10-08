package dev.akko.miaominal.ui.model

import dev.akko.miaominal.data.FakeTerminalSession
import org.junit.Assert.assertEquals
import org.junit.Test

class TerminalScrollAccumulatorTest {
    @Test fun smallMovesAndLargeMovesKeepTheirFractionalDistance() {
        val scroll = TerminalScrollAccumulator()
        assertEquals(0, scroll.move(9f, 10f))
        assertEquals(1, scroll.move(9f, 10f))
        assertEquals(1, scroll.move(2f, 10f))
        assertEquals(2, scroll.move(29f, 10f))
        assertEquals(1, scroll.move(1f, 10f))
    }

    @Test fun reversingDirectionUsesRemainingMotionWithoutADeadZone() {
        val scroll = TerminalScrollAccumulator()
        assertEquals(1, scroll.move(15f, 10f))
        assertEquals(0, scroll.move(-5f, 10f))
        assertEquals(-1, scroll.move(-10f, 10f))
        assertEquals(-1, scroll.move(-10f, 10f))
    }

    @Test fun aNewGestureDropsOldRemainderAndInvalidGeometryDoesNotPoisonMotion() {
        val scroll = TerminalScrollAccumulator()
        assertEquals(0, scroll.move(9f, 10f))
        scroll.reset()
        assertEquals(0, scroll.move(1f, 10f))
        assertEquals(0, scroll.move(Float.NaN, 10f))
        assertEquals(0, scroll.move(10f, 0f))
        assertEquals(1, scroll.move(9f, 10f))
    }

    @Test fun draggingDownRevealsHistoryAndDraggingUpReturnsToLatestOutput() {
        val scroll = TerminalScrollAccumulator()
        val session = FakeTerminalSession(1, "test")

        session.scroll(scroll.move(30f, 10f))
        assertEquals(3, session.renderFrame(0).displayOffset)

        scroll.reset()
        session.scroll(scroll.move(-10f, 10f))
        assertEquals(2, session.renderFrame(0).displayOffset)

        session.scroll(scroll.move(-30f, 10f))
        assertEquals(0, session.renderFrame(0).displayOffset)
    }
}
