package dev.akko.miaominal

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.AndroidPlatformHost
import dev.akko.miaominal.data.NativeBackendClient
import dev.akko.miaominal.nativebridge.Backend
import dev.akko.miaominal.nativebridge.BackendConfig
import dev.akko.miaominal.nativebridge.BackendEvent
import dev.akko.miaominal.nativebridge.BackendException
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith

/**
 * P1 acceptance: the native backend lifecycle is repeatable, shutdown is
 * idempotent and wakes event waiters, and a fresh handle can restart on the
 * same process business root.
 */
@RunWith(AndroidJUnit4::class)
class NativeBackendLifecycleTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val host = AndroidPlatformHost(context)

    private fun newBackend() = Backend(host, NativeBackendClient.configFor(context))

    @Test
    fun initializeAndShutdownAreRepeatable() = runBlocking {
        val backend = newBackend()
        try {
            assertFalse(backend.snapshot().initialized)

            val first = backend.initialize()
            assertTrue(first.initialized)
            assertFalse(first.closed)

            val second = backend.initialize()
            assertEquals(first.sequence, second.sequence)
            assertEquals(first.dataDir, second.dataDir)
            assertEquals(first.cacheDir, second.cacheDir)

            val subscription = backend.subscribeFrom(first.sequence)
            backend.shutdown()
            backend.shutdown()
            assertTrue(backend.isClosed())

            val batch = subscription.nextBatch(16u, 500uL)
            assertEquals(1, batch.events.size)
            assertEquals(BackendEvent.Shutdown, batch.events.first().event)
            assertEquals(first.sequence + 1uL, batch.events.first().sequence)

            try {
                subscription.nextBatch(16u, 500uL)
                fail("a closed backend must report Closed")
            } catch (_: BackendException.Closed) {
            }
        } finally {
            runCatching { backend.shutdown() }
            runCatching { backend.close() }
        }
    }

    @Test
    fun aFreshHandleRestartsOnTheSameBusinessRoot() = runBlocking {
        val first = newBackend()
        try {
            assertTrue(first.initialize().initialized)
        } finally {
            first.shutdown()
            first.close()
        }

        val second = newBackend()
        try {
            val snapshot = second.initialize()
            assertTrue(snapshot.initialized)
            // A fresh handle owns a fresh event bus, so `Ready` is sequence 1.
            assertEquals(1uL, snapshot.sequence)
        } finally {
            second.shutdown()
            second.close()
        }
    }

    @Test
    fun relativeDirectoriesAreRejectedWithoutFixingTheRoot() = runBlocking {
        val backend = Backend(
            host,
            BackendConfig(dataDir = "relative-data", cacheDir = "relative-cache")
        )
        try {
            backend.initialize()
            fail("relative directories must be rejected")
        } catch (_: BackendException.RuntimeUnavailable) {
        } finally {
            runCatching { backend.shutdown() }
            runCatching { backend.close() }
        }

        val valid = newBackend()
        try {
            assertTrue(valid.initialize().initialized)
        } finally {
            valid.shutdown()
            valid.close()
        }
    }
}
