package dev.akko.miaominal.ui.model

/** Preserve sub-row motion rather than discarding it whenever a row is scrolled. */
internal class TerminalScrollAccumulator {
    private var remainder = 0f

    fun reset() { remainder = 0f }

    fun move(deltaPx: Float, cellHeight: Float): Int {
        if (!deltaPx.isFinite() || !cellHeight.isFinite() || cellHeight <= 0f) return 0
        remainder += deltaPx
        val rows = (remainder / cellHeight).toInt()
        remainder -= rows * cellHeight
        return -rows
    }
}
