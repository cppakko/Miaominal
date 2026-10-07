package dev.akko.miaominal.ui.model

import org.junit.Assert.*
import org.junit.Test

class TerminalViewportTest {
    @Test fun keyboardOpeningDoesNotMoveAFittingPromptBecauseOfTrailingBlankRows() {
        // Recorded regression: the shell greeting fits even after the IME opens.
        (960 downTo 520).forEach { height ->
            val viewport = viewport(height.toFloat(), cursor = 15)
            assertEquals(0f, viewport.offsetPx, 0f)
            assertEquals(0, viewport.firstRow)
            assertEquals(360f, viewport.rowTop(15), 0f)
        }
    }

    @Test fun occludedPromptMovesByPixelsRatherThanWholeRows() {
        val a = viewport(520f)
        val b = viewport(519f)
        assertEquals(a.offsetPx + 1f, b.offsetPx, 0f)
        assertEquals(a.rowTop(39) - 1f, b.rowTop(39), 0f)
        assertEquals(520f, a.rowTop(39) + a.cellHeight, 0f)
        assertEquals(519f, b.rowTop(39) + b.cellHeight, 0f)
        assertEquals(18, a.firstRow)
        assertEquals(40, a.endRowExclusive)
    }

    @Test fun partialRowsUseTheSamePixelOffsetForDrawingAndHitTesting() {
        val viewport = viewport(520f)
        assertEquals(-8f, viewport.rowTop(18), 0f)
        assertEquals(18, viewport.rowAt(0f))
        assertEquals(19, viewport.rowAt(16f))
        assertEquals(39, viewport.rowAt(519f))
        (viewport.firstRow until viewport.endRowExclusive).forEach { row ->
            assertEquals(row, viewport.rowAt(viewport.rowTop(row) + 12f))
        }
    }

    @Test fun resizeHandoffPreservesThePromptPositionAfterRemovingOverflowRows() {
        val before = viewport(480f)
        val after = terminalViewport(20, 24f, 480f, 19, 0, false)
        assertEquals(before.rowTop(39), after.rowTop(19), 0f)
        assertEquals(0f, after.offsetPx, 0f)
    }

    @Test fun keyboardCancellationReversesTheSameMovementWithoutRememberedOffsets() {
        val opening = (960 downTo 520).map { viewport(it.toFloat()).rowTop(39) }
        val closing = (520..960).map { viewport(it.toFloat()).rowTop(39) }
        assertEquals(opening, closing.reversed())
    }

    @Test fun scrollbackAndFullScreenApplicationsKeepTheirTopAnchor() {
        listOf(5 to false, 0 to true).forEach { (scrollback, alternate) ->
            val viewport = terminalViewport(40, 24f, 520f, 39, scrollback, alternate)
            assertEquals(0f, viewport.offsetPx, 0f)
            assertEquals(0, viewport.firstRow)
            assertEquals(22, viewport.endRowExclusive)
        }
    }

    @Test fun emptyOrFullyOccludedViewsHaveNoVisibleRowsAndInvalidCursorsAreClamped() {
        val empty = terminalViewport(0, 24f, 520f, 0, 0, false)
        assertEquals(0, empty.endRowExclusive)
        assertEquals(0, empty.rowAt(10f))
        val hidden = viewport(0f)
        assertEquals(hidden.firstRow, hidden.endRowExclusive)
        assertEquals(0f, viewport(520f, cursor = -1).offsetPx, 0f)
        assertEquals(viewport(520f).offsetPx, viewport(520f, cursor = 100).offsetPx, 0f)
    }

    private fun viewport(height: Float, cursor: Int = 39) =
        terminalViewport(40, 24f, height, cursor, 0, false)
}
