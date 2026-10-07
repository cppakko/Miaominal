package dev.akko.miaominal.ui.model

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive

internal data class TerminalGridSize(val columns: Int, val rows: Int)

/** Layout keeps moving during IME animation; only commit its settled grid. */
internal class TerminalGeometryPolicy {
    private var reported: TerminalGridSize? = null
    private var imeAnimations = 0
    val animating: Boolean get() = imeAnimations > 0
    val needsInitialReport: Boolean get() = reported == null

    fun beginImeAnimation() { imeAnimations++ }
    fun endImeAnimation() { imeAnimations = (imeAnimations - 1).coerceAtLeast(0) }
    fun resetSession() { reported = null }
    fun detach() { resetSession(); imeAnimations = 0 }

    fun takeReport(size: TerminalGridSize, force: Boolean = false): TerminalGridSize? {
        if (size.columns < 2 || size.rows < 1 || (!force && animating)) return null
        if (!force && size == reported) return null
        reported = size
        return size
    }
}

/** One serial consumer per session: slow reflow cannot build a resize backlog. */
internal class TerminalResizeQueue {
    private val requests = Channel<TerminalGridSize>(Channel.CONFLATED)

    fun request(columns: Int, rows: Int) {
        if (columns >= 2 && rows >= 1) requests.trySend(TerminalGridSize(columns, rows))
    }

    suspend fun consume(resize: suspend (columns: Int, rows: Int) -> Unit) {
        var applied: TerminalGridSize? = null
        for (size in requests) {
            currentCoroutineContext().ensureActive()
            if (size == applied) continue
            try {
                resize(size.columns, size.rows)
                applied = size
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                // A disconnect can race the resize. Permit a later retry.
            }
        }
    }

    fun close() { requests.cancel() }
}
