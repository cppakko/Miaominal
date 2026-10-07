package dev.akko.miaominal

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.RenderNode
import android.view.View
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.FakeTerminalSession
import dev.akko.miaominal.data.TerminalFrameModel
import dev.akko.miaominal.ui.components.TerminalSurfaceView
import org.junit.Assert.*
import org.junit.Test

/** Compile-only until a designated test device is approved. */
class TerminalSurfaceRenderingTest {
    @Test fun fittingPromptStaysVisibleAndStationaryDuringKeyboardHeightChanges() = onMain {
        val view = surface()
        val original = FakeTerminalSession(1, "test", columns = 20, lines = 44).apply {
            repeat(16) { grid.add("greeting line $it") }
        }.renderFrame(0)
        view.applyFrame(original)
        record(view)
        val top = view.accessibleRowBounds(0).top
        val promptTop = view.accessibleRowBounds(15).top
        val firstNode = nodes(view)[0]
        (800 downTo 400 step 5).forEach { height ->
            view.layout(0, 0, 480, height)
            record(view)
            assertTrue(view.accessibleRows().contains(0))
            assertTrue(view.accessibleRows().contains(15))
            assertEquals(top, view.accessibleRowBounds(0).top)
            assertEquals(promptTop, view.accessibleRowBounds(15).top)
            assertSame(firstNode, nodes(view)[0])
        }
    }

    @Test fun heightChangesReuseRowsWhileSparseFramesReplaceOnlyChangedRows() = onMain {
        val view = surface()
        val original = frame()
        view.applyFrame(original)
        record(view)
        val before = nodes(view)
        assertNotNull(before[0])
        assertNotNull(before[1])

        view.layout(0, 0, 480, 600)
        record(view)
        assertSame(before[0], nodes(view)[0])
        assertSame(before[1], nodes(view)[1])

        val changed = original.rows[0].copy(cells = original.rows[0].cells.mapIndexed { index, cell ->
            if (index == 0) cell.copy(text = "B", fg = 0xFFFF0000.toInt()) else cell
        })
        view.applyFrame(original.copy(full = false, rows = listOf(changed)))
        record(view)
        assertNotSame(before[0], nodes(view)[0])
        assertSame(before[1], nodes(view)[1])
    }

    @Test fun fontBackgroundAndSessionChangesInvalidateRecordingsButIdenticalFullFramesReuseThem() = onMain {
        val view = surface()
        val original = frame()
        view.applyFrame(original)
        record(view)
        var before = nodes(view)[0]
        view.setTextSizePx(20f)
        record(view)
        assertNotSame(before, nodes(view)[0])

        before = nodes(view)[0]
        view.applyFrame(original.copy(full = false, rows = emptyList(), defaultBg = 0xFF304050.toInt()))
        record(view)
        assertNotSame(before, nodes(view)[0])

        before = nodes(view)[0]
        val changedBackground = original.copy(defaultBg = 0xFF304050.toInt())
        view.applyFrame(changedBackground)
        record(view)
        assertSame(before, nodes(view)[0])
        view.session = FakeTerminalSession(99, "new-session")
        assertTrue(nodes(view).isEmpty())
        assertTrue(view.accessibleRows().isEmpty())
    }

    @Test fun scrollingAndOutputReuseMovedRowRecordingsAcrossSparseAndFullFrames() = onMain {
        val view = surface()
        val original = frame()
        view.applyFrame(original)
        record(view)
        val first = nodes(view)[0]
        val second = nodes(view)[1]
        val moved = original.rows.toMutableList().apply {
            set(0, original.rows[1].copy(row = 0))
            set(1, original.rows[0].copy(row = 1))
        }
        view.applyFrame(original.copy(full = false, rows = moved, displayOffset = 1))
        record(view)
        assertSame(second, nodes(view)[0])
        assertSame(first, nodes(view)[1])
        view.applyFrame(original.copy(full = true, displayOffset = 0))
        record(view)
        assertSame(first, nodes(view)[0])
        assertSame(second, nodes(view)[1])
    }

    @Test fun discardedRowRecordingIsRestoredWithoutAnotherTerminalFrame() = onMain {
        val view = surface()
        view.applyFrame(frame())
        record(view)
        val before = nodes(view)
        val text = view.accessibleRows().associateWith(view::accessibleRowText)
        before[0]!!.discardDisplayList()
        assertFalse(before[0]!!.hasDisplayList())

        record(view)

        assertNotSame(before[0], nodes(view)[0])
        assertTrue(nodes(view)[0]!!.hasDisplayList())
        assertSame(before[1], nodes(view)[1])
        assertEquals(text, view.accessibleRows().associateWith(view::accessibleRowText))
    }

    @Test fun fullFrameDoesNotReuseDiscardedRecordings() = onMain {
        val view = surface()
        val original = frame()
        view.applyFrame(original)
        record(view)
        val before = nodes(view)
        before[0]!!.discardDisplayList()

        view.applyFrame(original)

        assertNull(nodes(view)[0])
        assertSame(before[1], nodes(view)[1])
        record(view)
        assertTrue(nodes(view)[0]!!.hasDisplayList())
    }

    @Test fun returningToVisibleWindowRestoresIdleGridWithoutAnotherTerminalFrame() = onMain {
        val view = surface()
        view.applyFrame(frame())
        record(view)
        val before = nodes(view)
        val text = view.accessibleRows().associateWith(view::accessibleRowText)

        view.dispatchWindowVisibilityChanged(View.INVISIBLE)
        assertSame(before[0], nodes(view)[0])
        view.dispatchWindowVisibilityChanged(View.VISIBLE)
        assertTrue(nodes(view).all { it == null })
        assertEquals(text, view.accessibleRows().associateWith(view::accessibleRowText))

        record(view)

        view.accessibleRows().forEach { row ->
            assertNotSame(before[row], nodes(view)[row])
            assertTrue(nodes(view)[row]!!.hasDisplayList())
        }
        assertEquals(text, view.accessibleRows().associateWith(view::accessibleRowText))
    }

    @Test fun softwareCanvasStillDrawsBackgroundsAndSparseUpdates() = onMain {
        val view = surface()
        val original = frame()
        view.applyFrame(original)
        val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        try {
            view.draw(Canvas(bitmap))
            assertEquals(original.defaultBg, bitmap.getPixel(0, 0))
            view.applyFrame(original.copy(full = false, rows = emptyList(), defaultBg = 0xFF304050.toInt()))
            view.draw(Canvas(bitmap))
            assertEquals(0xFF304050.toInt(), bitmap.getPixel(0, 0))
        } finally { bitmap.recycle() }
    }

    private fun surface() = TerminalSurfaceView(InstrumentationRegistry.getInstrumentation().targetContext).apply {
        setTextSizePx(14f)
        layout(0, 0, 480, 800)
    }

    private fun frame(): TerminalFrameModel = FakeTerminalSession(1, "test", columns = 20, lines = 4).apply {
        grid.add("A wide 中 text")
        grid.add("second row")
    }.renderFrame(0)

    private fun record(view: TerminalSurfaceView) {
        val target = RenderNode("terminal-test")
        val canvas = target.beginRecording(view.width, view.height)
        try {
            assertTrue(canvas.isHardwareAccelerated)
            view.draw(canvas)
        } finally {
            target.endRecording()
            target.discardDisplayList()
        }
    }

    @Suppress("UNCHECKED_CAST")
    private fun nodes(view: TerminalSurfaceView): List<RenderNode?> =
        (TerminalSurfaceView::class.java.getDeclaredField("rowNodes").apply { isAccessible = true }
            .get(view) as List<RenderNode?>).toList()

    private fun onMain(block: () -> Unit) {
        InstrumentationRegistry.getInstrumentation().runOnMainSync(block)
    }
}
