package dev.akko.miaominal.ui.model

import dev.akko.miaominal.data.SessionSnapshotModel
import dev.akko.miaominal.data.TerminalFrameModel
import dev.akko.miaominal.data.TerminalSessionHandle
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.withContext

/** Pull once per display tick under load; idle sessions only wait for updates. */
internal suspend fun pumpTerminalFrames(
    session: TerminalSessionHandle,
    awaitDisplayFrame: suspend () -> Unit,
    onFrame: (TerminalFrameModel) -> Unit,
    onSnapshot: (SessionSnapshotModel) -> Unit,
    dispatcher: CoroutineDispatcher = Dispatchers.Default
) {
    var frameVersion = 0L
    while (currentCoroutineContext().isActive) {
        awaitDisplayFrame()
        val reading = withContext(dispatcher) {
            // Session revisions and terminal generations are separate counters.
            // Read state BEFORE the grid so later output cannot be acknowledged
            // by the snapshot without being included in this render pull.
            val state = readTerminal { session.snapshot() }
            state to readTerminal { session.renderFrame(frameVersion) }
        }
        val (state, frame) = reading
        if (frame != null) {
            frameVersion = frame.frameVersion
            onFrame(frame)
        }
        if (state != null) onSnapshot(state)
        // A render failure must not hide the connection failure/recovery controls.
        if (state?.phase?.isTerminal == true && (frame == null || frame.closed)) return
        if (state == null || frame == null) {
            delay(100)
            continue
        }

        while (currentCoroutineContext().isActive) {
            val update = try {
                session.awaitUpdate(state.revision, 400)
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                delay(100)
                break
            }
            if (!update.idle || update.closed || update.revision != state.revision) break
        }
    }
}

private inline fun <T> readTerminal(read: () -> T): T? = try {
    read()
} catch (cancelled: CancellationException) {
    throw cancelled
} catch (_: Exception) {
    null
}
