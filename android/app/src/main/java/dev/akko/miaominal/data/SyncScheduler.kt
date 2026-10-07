package dev.akko.miaominal.data

import android.content.Context
import android.util.Log
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import dev.akko.miaominal.MiaominalApplication
import dev.akko.miaominal.nativebridge.BackendException
import java.util.concurrent.TimeUnit

/**
 * Periodic automatic-sync check.
 *
 * The worker is the background half of automatic sync: it runs the same
 * `syncAutoCheck` entry point as the foreground resume check, shares the Rust
 * execution lock, and does nothing when automatic sync is disabled or the
 * vault is locked. It never shows authentication UI.
 */
class SyncWorker(
    appContext: Context,
    params: WorkerParameters
) : CoroutineWorker(appContext, params) {

    override suspend fun doWork(): Result {
        val container = (applicationContext as? MiaominalApplication)?.container
            ?: return Result.success()
        val repository = container.repository
        if (repository.ensureStarted().isFailure) return Result.retry()
        return repository.syncAutoCheck().fold(
            onSuccess = { Result.success() },
            onFailure = { error ->
                if (error.isTransientSyncFailure()) Result.retry() else Result.success()
            }
        )
    }

    private fun Throwable.isTransientSyncFailure(): Boolean =
        this is BackendException.Network || this is BackendException.RuntimeUnavailable
}

/** Registers exactly one unique periodic sync job while automatic sync is on. */
object SyncScheduler {

    const val UNIQUE_WORK_NAME = "miaominal-sync-periodic"
    private const val TAG = "SyncScheduler"

    /**
     * Reconcile the periodic job with a sync snapshot.
     *
     * A snapshot that has never been loaded (`loaded == false`) is ignored: the
     * default state has `autoSyncEnabled == false`, and acting on it during a
     * cold start would cancel the job that just woke the process. Only a real
     * backend snapshot may register or cancel work.
     */
    fun reconcile(context: Context, state: SyncStateModel) {
        if (!state.loaded) return
        reconcile(context, state.config.autoSyncEnabled)
    }

    fun reconcile(context: Context, enabled: Boolean) {
        runCatching {
            val workManager = WorkManager.getInstance(context)
            if (!enabled) {
                workManager.cancelUniqueWork(UNIQUE_WORK_NAME)
                return
            }
            val request = PeriodicWorkRequestBuilder<SyncWorker>(15, TimeUnit.MINUTES)
                .setConstraints(
                    Constraints.Builder()
                        .setRequiredNetworkType(NetworkType.CONNECTED)
                        .build()
                )
                .build()
            workManager.enqueueUniquePeriodicWork(
                UNIQUE_WORK_NAME,
                ExistingPeriodicWorkPolicy.KEEP,
                request
            )
        }.onFailure { error ->
            Log.w(TAG, "failed to reconcile the periodic sync job: ${error.javaClass.simpleName}")
        }
    }

    fun cancel(context: Context) {
        runCatching { WorkManager.getInstance(context).cancelUniqueWork(UNIQUE_WORK_NAME) }
    }
}
