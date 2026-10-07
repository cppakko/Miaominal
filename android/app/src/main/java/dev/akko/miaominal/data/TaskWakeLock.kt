package dev.akko.miaominal.data

import android.content.Context
import android.os.PowerManager

/**
 * Partial wake lock held for exactly as long as active tasks exist.
 *
 * Deliberately acquired without a timeout: the previous 30 minute timeout
 * lapsed while a stable forwarding session never re-emitted its state, so the
 * lock was never renewed. The service releases it when the task count reaches
 * zero and in `onDestroy`, so the hold is tied to the task and service
 * lifecycle instead of a timer.
 */
class TaskWakeLock(context: Context) {

    private val powerManager = context.applicationContext
        .getSystemService(Context.POWER_SERVICE) as? PowerManager

    private var lock: PowerManager.WakeLock? = null

    val isHeld: Boolean get() = lock?.isHeld == true

    /** Idempotent: repeated calls while held do not add a reference. */
    fun acquire() {
        if (lock?.isHeld == true) return
        val manager = powerManager ?: return
        lock = manager.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, TAG).apply {
            setReferenceCounted(false)
            acquire()
        }
    }

    /** Idempotent: releasing an unheld lock is a no-op. */
    fun release() {
        val current = lock
        lock = null
        if (current != null && current.isHeld) {
            runCatching { current.release() }
        }
    }

    companion object {
        const val TAG = "Miaominal::ConnectionService"
    }
}
