package dev.akko.miaominal

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.NativeBackendProbe
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * P0 acceptance: the real native library is loaded from the APK and the
 * Kotlin -> UniFFI -> Rust path works for synchronous calls, asynchronous
 * calls, and platform callbacks.
 */
@RunWith(AndroidJUnit4::class)
class NativeBridgeProbeTest {
    @Test
    fun probeExercisesNativeBridge() = runBlocking {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val result = NativeBackendProbe.run(context)

        assertTrue(result.buildInfo.abiVersion.startsWith("miaominal-android/"))
        assertTrue(result.buildInfo.packageVersion.isNotBlank())
        assertEquals("miaominal-p0", result.echo)
        assertEquals("keystore-roundtrip-ok", result.secret)
        assertTrue(result.deviceLabel.isNotBlank())
    }
}
