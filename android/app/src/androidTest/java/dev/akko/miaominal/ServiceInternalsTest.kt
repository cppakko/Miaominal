package dev.akko.miaominal

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.TaskIdleExit
import dev.akko.miaominal.data.TaskWakeLock
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * service internals: the delayed service exit fires without another state
 * emission, and the wake lock is held for exactly the task lifetime (no timer
 * to lapse while a stable session never re-emits).
 */
@RunWith(AndroidJUnit4::class)
class ServiceInternalsTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    @Test
    fun idleExitFiresAfterTheDelayWithoutAnotherStateEmission() = runBlocking {
        var fired = 0
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        try {
            val exit = TaskIdleExit(scope, idleMillis = 120) { fired += 1 }

            exit.update(0)
            assertTrue("an idle count must schedule the exit", exit.isScheduled)

            delay(400)
            assertEquals("the scheduled exit must fire on its own", 1, fired)
            assertFalse(exit.isScheduled)
        } finally {
            scope.cancel()
        }
    }

    @Test
    fun newTasksCancelThePendingExit() = runBlocking {
        var fired = 0
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        try {
            val exit = TaskIdleExit(scope, idleMillis = 200) { fired += 1 }

            exit.update(0)
            exit.update(2)
            assertFalse(exit.isScheduled)

            delay(400)
            assertEquals("a new task must cancel the pending exit", 0, fired)

            exit.update(0)
            delay(400)
            assertEquals(1, fired)
        } finally {
            scope.cancel()
        }
    }

    @Test
    fun wakeLockIsHeldForTheTaskLifetimeAndReleasesIdempotently() {
        val wakeLock = TaskWakeLock(context)
        try {
            assertFalse(wakeLock.isHeld)
            wakeLock.acquire()
            assertTrue(wakeLock.isHeld)
            // Repeated acquires (state re-emissions) must not double-reference.
            wakeLock.acquire()
            assertTrue(wakeLock.isHeld)

            wakeLock.release()
            assertFalse(wakeLock.isHeld)
            wakeLock.release()
            assertFalse(wakeLock.isHeld)

            // The lock can be taken again for the next batch of tasks.
            wakeLock.acquire()
            assertTrue(wakeLock.isHeld)
        } finally {
            wakeLock.release()
        }
    }
}
