package dev.akko.miaominal.ui.model

import kotlin.math.ceil
import kotlin.math.floor

/** A pixel viewport over the last terminal frame while its grid awaits resize. */
internal data class TerminalViewport(
    val firstRow: Int,
    val endRowExclusive: Int,
    val offsetPx: Float,
    val cellHeight: Float,
    val gridRows: Int
) {
    fun rowTop(row: Int): Float = row * cellHeight - offsetPx
    fun rowAt(y: Float): Int = floor((y + offsetPx) / cellHeight).toInt()
        .coerceIn(0, (gridRows - 1).coerceAtLeast(0))
}

internal fun terminalViewport(
    gridRows: Int,
    cellHeight: Float,
    viewportHeight: Float,
    cursorRow: Int,
    displayOffset: Int,
    alternateScreen: Boolean
): TerminalViewport {
    val rows = gridRows.coerceAtLeast(0)
    val cell = cellHeight.coerceAtLeast(1f)
    val height = viewportHeight.coerceAtLeast(0f)
    // Trailing blank rows are not an anchor. Keep content still if the prompt fits.
    // Scrollback and full-screen applications retain their top until grid redraw.
    val offset = if (rows > 0 && displayOffset == 0 && !alternateScreen) {
        ((cursorRow.coerceIn(0, rows - 1) + 1) * cell - height).coerceAtLeast(0f)
    } else 0f
    val first = floor(offset / cell).toInt().coerceIn(0, rows)
    val end = if (height == 0f) first else ceil((offset + height) / cell).toInt().coerceIn(first, rows)
    return TerminalViewport(first, end, offset, cell, rows)
}
