package dev.akko.miaominal

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import dev.akko.miaominal.data.LongRunningTaskCounts
import dev.akko.miaominal.data.MiaominalRepository
import dev.akko.miaominal.data.TaskIdleExit
import dev.akko.miaominal.data.TaskWakeLock
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch

/**
 * Foreground service for background keep-alive.
 *
 * It is only started from a visible user action, reports the number of SSH,
 * forwarding, and transfer tasks in its notification, offers "open app" and
 * "stop tasks" actions, holds a wake lock only while tasks are active, and
 * stops itself when the last task ends. The service type is `specialUse` for
 * interactive SSH/forwarding and `dataSync` when file transfers are running.
 */
class ConnectionService : Service() {

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var observer: Job? = null
    private val wakeLockController by lazy { TaskWakeLock(this) }
    private val idleExit by lazy {
        TaskIdleExit(scope, TaskIdleExit.DEFAULT_IDLE_MILLIS) { stopSelf() }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        ensureChannel(this)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP_TASKS) {
            policy?.stopAllTasks() ?: stopSelf()
            return START_NOT_STICKY
        }
        startInForeground(LongRunningTaskCounts(0, 0, 0))
        observeTasks()
        return START_NOT_STICKY
    }

    override fun onTimeout(startId: Int, fgsType: Int) {
        // The system limits `dataSync` runs; when only transfers were running
        // and the limit hits, stop cleanly instead of being killed.
        if (fgsType == android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC) {
            val counts = currentCounts.value
            if (counts.sshCount + counts.forwardCount + counts.agentCount == 0) {
                stopSelf()
            } else {
                // Keep the interactive part alive under `specialUse`.
                startInForeground(counts)
            }
        }
    }

    override fun onDestroy() {
        observer?.cancel()
        idleExit.cancel()
        wakeLockController.release()
        scope.cancel()
        serviceActive = false
        super.onDestroy()
    }

    private fun observeTasks() {
        if (observer != null) return
        serviceActive = true
        // The policy owns the full count (AI sessions included); the
        // repository-only fallback keeps the service usable in tests.
        val countsFlow = policy?.taskCounts ?: repository?.let { repo ->
            combine(
                repo.sessions,
                repo.forwardSessions,
                repo.sftpSessions
            ) { sessions, forwards, sftpSessions ->
                LongRunningTaskCounts(
                    sshCount = sessions.count { !it.phase.isTerminal },
                    forwardCount = forwards.count { it.phase.isActive },
                    transferCount = sftpSessions.sumOf { it.activeTransferCount }
                )
            }
        } ?: return
        observer = scope.launch {
            countsFlow.collect { counts ->
                currentCounts.value = counts
                if (counts.total == 0) {
                    // The exit is decided by the scheduled job, not by this
                    // emission: the last task ends exactly once.
                    wakeLockController.release()
                } else {
                    wakeLockController.acquire()
                }
                idleExit.update(counts.total)
                startInForeground(counts)
            }
        }
    }

    private fun startInForeground(counts: LongRunningTaskCounts) {
        // AI sessions are interactive work: they must keep `specialUse` so the
        // dataSync time limit cannot stop the service while a chat is running.
        val interactive = counts.sshCount + counts.forwardCount + counts.agentCount > 0
        val type = if (interactive) {
            android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
        } else {
            android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
        }
        val notification = buildNotification(counts)
        try {
            ServiceCompat.startForeground(this, NOTIFICATION_ID, notification, type)
        } catch (error: Throwable) {
            // A rejected foreground start (for example a disallowed type on a
            // newer platform) must not crash the process.
            android.util.Log.w("ConnectionService", "foreground start rejected: $error")
            stopSelf()
        }
    }

    private fun buildNotification(counts: LongRunningTaskCounts): Notification {
        val summary = getString(
            R.string.background_notification_summary,
            counts.sshCount,
            counts.forwardCount,
            counts.transferCount,
            counts.agentCount
        )
        val openIntent = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        val stopIntent = PendingIntent.getService(
            this,
            1,
            Intent(this, ConnectionService::class.java).setAction(ACTION_STOP_TASKS),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_sys_upload_done)
            .setContentTitle(getString(R.string.background_service_title))
            .setContentText(summary)
            .setOngoing(true)
            .setSilent(true)
            .setContentIntent(openIntent)
            .addAction(0, getString(R.string.background_action_open), openIntent)
            .addAction(0, getString(R.string.background_action_stop), stopIntent)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .build()
    }

    companion object {
        const val ACTION_STOP_TASKS = "dev.akko.miaominal.action.STOP_TASKS"
        private const val CHANNEL_ID = "connection-keep-alive"
        private const val NOTIFICATION_ID = 41

        private val currentCounts = MutableStateFlow(LongRunningTaskCounts(0, 0, 0))

        @Volatile
        private var serviceActive = false

        /**
         * Set by the application container so the service can observe and close
         * tasks without owning a second repository instance.
         */
        @Volatile
        var repository: MiaominalRepository? = null

        @Volatile
        var policy: dev.akko.miaominal.data.BackgroundPolicy? = null

        val active: StateFlow<LongRunningTaskCounts> = currentCounts
        val isActive: Boolean get() = serviceActive

        fun start(context: Context) {
            val intent = Intent(context, ConnectionService::class.java)
            // The policy only starts the service while an activity is visible,
            // but a task-count emission can race the last onStop. A background
            // foreground-service start is rejected by the OS and must not
            // crash the app (the grace policy simply applies instead).
            runCatching { context.startForegroundService(intent) }
                .onFailure { error ->
                    android.util.Log.w("ConnectionService", "foreground service start rejected: $error")
                }
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, ConnectionService::class.java))
        }

        fun ensureChannel(context: Context) {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
            val manager = context.getSystemService(NotificationManager::class.java) ?: return
            if (manager.getNotificationChannel(CHANNEL_ID) != null) return
            val channel = NotificationChannel(
                CHANNEL_ID,
                context.getString(R.string.background_service_channel),
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = context.getString(R.string.background_service_channel_desc)
                setShowBadge(false)
            }
            manager.createNotificationChannel(channel)
        }
    }
}
