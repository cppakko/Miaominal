package dev.akko.miaominal.data

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Delayed "no tasks left" callback for the foreground service.
 *
 * The task count is only observable through StateFlow emissions, so the exit
 * must be scheduled here instead of being decided inside the collector: when
 * the last task ends the flows may never emit again and the service (and its
 * zero-task notification) would stay alive forever. A new task cancels the
 * pending exit.
 */
class TaskIdleExit(
    private val scope: CoroutineScope,
    private val idleMillis: Long = DEFAULT_IDLE_MILLIS,
    private val onIdle: () -> Unit
) {
    private var exitJob: Job? = null

    val isScheduled: Boolean get() = exitJob?.isActive == true

    fun update(activeTasks: Int) {
        if (activeTasks > 0) {
            cancel()
        } else if (exitJob == null) {
            exitJob = scope.launch {
                delay(idleMillis)
                exitJob = null
                onIdle()
            }
        }
    }

    fun cancel() {
        exitJob?.cancel()
        exitJob = null
    }

    companion object {
        const val DEFAULT_IDLE_MILLIS = 5_000L
    }
}
