package dev.akko.miaominal

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.work.NetworkType
import androidx.work.WorkInfo
import androidx.work.WorkManager
import dev.akko.miaominal.data.SyncScheduler
import dev.akko.miaominal.data.SyncStateModel
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.util.concurrent.TimeUnit

/**
 * acceptance for the periodic automatic-sync job: exactly one unique
 * 15-minute work item with a network constraint, kept on repeated enables and
 * cancelled when the setting is turned off.
 */
@RunWith(AndroidJUnit4::class)
class SyncSchedulerTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    private fun workInfos(): List<WorkInfo> = WorkManager.getInstance(context)
        .getWorkInfosForUniqueWork(SyncScheduler.UNIQUE_WORK_NAME)
        .get(30, TimeUnit.SECONDS)

    @Test
    fun uniquePeriodicWorkIsRegisteredAndCancelled() {
        SyncScheduler.reconcile(context, enabled = true)
        val first = workInfos()
        assertEquals("exactly one periodic job must exist", 1, first.size)
        assertTrue(
            "the job must be waiting for the network: ${first[0].state}",
            first[0].state == WorkInfo.State.ENQUEUED || first[0].state == WorkInfo.State.RUNNING
        )
        assertEquals(
            NetworkType.CONNECTED,
            first[0].constraints.requiredNetworkType
        )

        // Enabling again must not enqueue a second instance.
        SyncScheduler.reconcile(context, enabled = true)
        val second = workInfos()
        assertEquals("the job must stay unique", 1, second.size)

        SyncScheduler.reconcile(context, enabled = false)
        val cancelled = workInfos()
        assertTrue(
            "disabling automatic sync must cancel the job",
            cancelled.all { it.state == WorkInfo.State.CANCELLED }
        )
    }

    @Test
    fun anUnloadedSnapshotNeverCancelsThePersistedJob() {
        SyncScheduler.reconcile(context, enabled = true)
        assertEquals(1, workInfos().size)

        // A cold start observes the default, not-yet-loaded snapshot
        // (`autoSyncEnabled == false`). It must not cancel the persisted job.
        SyncScheduler.reconcile(context, SyncStateModel())
        val infos = workInfos()
        assertEquals(1, infos.size)
        assertTrue(
            "the persisted job must survive an unloaded snapshot: ${infos[0].state}",
            infos[0].state == WorkInfo.State.ENQUEUED || infos[0].state == WorkInfo.State.RUNNING
        )

        SyncScheduler.reconcile(context, enabled = false)
    }
}
