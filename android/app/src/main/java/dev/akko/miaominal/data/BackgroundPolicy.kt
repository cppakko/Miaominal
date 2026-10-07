package dev.akko.miaominal.data

import android.app.Activity
import android.app.Application
import android.content.Context
import android.os.Bundle
import dev.akko.miaominal.ConnectionService
import dev.akko.miaominal.MiaominalApplication
import dev.akko.miaominal.data.agent.AgentChatRepository
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

/**
 * Counts shown by the foreground-service notification.
 */
data class LongRunningTaskCounts(
    val sshCount: Int,
    val forwardCount: Int,
    val transferCount: Int,
    /** One while an AI CLI session is running (it is a first-class task). */
    val agentCount: Int = 0
) {
    val total: Int get() = sshCount + forwardCount + transferCount + agentCount
}

/**
 * Process-level background policy.
 *
 * Background keep-alive is on by default: while any task is running, leaving
 * the app keeps the foreground service (and its notification) alive, so
 * SSH/SFTP/forwarding sessions survive backgrounding. When the user turns
 * keep-alive off in settings, leaving the app starts a 30 second grace timer;
 * when it elapses (and no app-owned user flow such as a document picker is
 * open) every SSH/SFTP/forward session is closed and transfers are cancelled.
 */
class BackgroundPolicy(
    private val application: Application,
    private val repository: MiaominalRepository,
    private val scope: CoroutineScope,
    private val agentSessions: AgentChatRepository? = null,
    private val graceMillis: Long = GRACE_MILLIS
) : Application.ActivityLifecycleCallbacks {
    @Volatile
    private var startedActivities = 0

    @Volatile
    private var keepAliveEnabled = false

    @Volatile
    private var userFlowDepth = 0

    /** Depth of launched external apps (system browser) the user is expected to return from. */
    @Volatile
    private var externalFlowDepth = 0

    private var graceJob: Job? = null

    /**
     * Invoked when the app transitions from background to foreground (P6).
     * Automatic sync hooks its resume check here; keeping it separate from the
     * grace logic means the two policies never interleave.
     */
    var onForeground: (() -> Unit)? = null

    val isForeground: Boolean get() = startedActivities > 0
    val isKeepAliveEnabled: Boolean get() = keepAliveEnabled

    /** One task while a CLI session runs; the agent has no separate session list. */
    private val agentTaskCount: Flow<Int> =
        (agentSessions?.active ?: MutableStateFlow(false)).map { if (it) 1 else 0 }

    /**
     * Live task counts for the foreground service. AI sessions are included so
     * an AI-only workload keeps the service, its wake lock and its exit
     * decision alive after the last terminal session is closed.
     */
    val taskCounts: Flow<LongRunningTaskCounts> = combine(
        repository.sessions,
        repository.forwardSessions,
        repository.sftpSessions,
        agentTaskCount
    ) { sessions, forwards, sftpSessions, agentRunning ->
        LongRunningTaskCounts(
            sshCount = sessions.count { !it.phase.isTerminal },
            forwardCount = forwards.count { it.phase.isActive },
            transferCount = sftpSessions.sumOf { it.activeTransferCount },
            agentCount = agentRunning
        )
    }

    init {
        // Keep-alive defaults to on: a new task started from a visible screen
        // brings the foreground service up so backgrounding does not close it.
        scope.launch {
            taskCounts.collect { counts ->
                if (
                    keepAliveEnabled && counts.total > 0 &&
                    isForeground && !ConnectionService.isActive
                ) {
                    android.util.Log.i(
                        "BackgroundPolicy",
                        "keep-alive: starting foreground service for ${counts.total} task(s)"
                    )
                    ConnectionService.start(application)
                }
            }
        }
    }

    fun attach() {
        application.registerActivityLifecycleCallbacks(this)
    }

    /**
     * Enable or disable background keep-alive from a visible screen. Enabling
     * starts the foreground service; disabling stops it and starts the grace
     * timer if the app is already in the background.
     */
    fun setKeepAliveEnabled(enabled: Boolean) {
        keepAliveEnabled = enabled
        if (enabled) {
            graceJob?.cancel()
            ConnectionService.start(application)
        } else {
            ConnectionService.stop(application)
            if (!isForeground && !hasOpenUserFlow()) scheduleGrace()
        }
    }

    /** Called when Android recreated the process and the setting is still on. */
    fun restoreFromPreferences(enabled: Boolean) {
        keepAliveEnabled = enabled
        // The service is never started automatically without a visible action;
        // it will start the next time the user opens a connection screen.
    }

    /**
     * Mark an app-owned user flow that temporarily moves our Activity to the
     * background (document picker, biometric prompt, permission dialog). The
     * grace countdown must not fire while this flow is open.
     */
    fun beginUserFlow() {
        userFlowDepth += 1
        graceJob?.cancel()
    }

    fun endUserFlow() {
        if (userFlowDepth > 0) userFlowDepth -= 1
        if (!isForeground && !hasOpenUserFlow() && !keepAliveEnabled) {
            scheduleGrace()
        }
    }

    /**
     * A launched external app the user is expected to come back from (the
     * system browser for a forwarded port or a terminal search) is an app-owned
     * user flow: the grace countdown must not close the tunnel while the page
     * is on screen. There is no result callback for it, so the flow also ends
     * when our Activity is started again; [endExternalFlow] covers launches
     * that never left the app.
     */
    fun beginExternalFlow() {
        externalFlowDepth += 1
        graceJob?.cancel()
    }

    fun endExternalFlow() {
        if (externalFlowDepth > 0) externalFlowDepth -= 1
        if (!isForeground && !hasOpenUserFlow() && !keepAliveEnabled) {
            scheduleGrace()
        }
    }

    private fun hasOpenUserFlow(): Boolean = userFlowDepth > 0 || externalFlowDepth > 0

    /** Foreground-service "stop tasks" action: close sessions and stop the service. */
    fun stopAllTasks() {
        graceJob?.cancel()
        scope.launch {
            repository.closeAllSessions()
            agentSessions?.closeConnection()
            ConnectionService.stop(application)
        }
    }

    private fun scheduleGrace() {
        graceJob?.cancel()
        graceJob = scope.launch {
            delay(graceMillis)
            if (!isForeground && !hasOpenUserFlow() && !keepAliveEnabled) {
                android.util.Log.i(
                    "BackgroundPolicy",
                    "grace elapsed: closing background sessions (keep-alive off)"
                )
                repository.closeAllSessions()
                // AI sessions keep their rendered history in the local cache,
                // so closing the connection here is recoverable.
                agentSessions?.closeConnection()
            }
        }
    }

    /** Test hook: run the same path as the last Activity stopping. */
    fun simulateBackgroundForTest() {
        startedActivities = 0
        if (!hasOpenUserFlow() && !keepAliveEnabled) {
            scheduleGrace()
        }
    }

    override fun onActivityStarted(activity: Activity) {
        val wasBackground = startedActivities == 0
        startedActivities += 1
        graceJob?.cancel()
        if (wasBackground) {
            onForeground?.invoke()
        }
        // Returning to our own Activity ends a launched browser flow.
        endExternalFlow()
    }

    override fun onActivityStopped(activity: Activity) {
        startedActivities = (startedActivities - 1).coerceAtLeast(0)
        if (!isForeground && !hasOpenUserFlow() && !keepAliveEnabled) {
            scheduleGrace()
        }
    }

    override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) = Unit
    override fun onActivityResumed(activity: Activity) = Unit
    override fun onActivityPaused(activity: Activity) = Unit
    override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) = Unit
    override fun onActivityDestroyed(activity: Activity) = Unit

    companion object {
        /** Grace period before connections are closed after backgrounding. */
        const val GRACE_MILLIS = 30_000L

        /** The process-wide policy, or `null` outside the app process (tests, previews). */
        fun from(context: Context): BackgroundPolicy? =
            (context.applicationContext as? MiaominalApplication)?.container?.backgroundPolicy
    }
}
