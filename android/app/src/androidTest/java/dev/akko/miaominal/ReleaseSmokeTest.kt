package dev.akko.miaominal

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.AndroidPlatformHost
import dev.akko.miaominal.nativebridge.AuthMethodDto
import dev.akko.miaominal.nativebridge.Backend
import dev.akko.miaominal.nativebridge.BackendConfig
import dev.akko.miaominal.nativebridge.BackendEvent
import dev.akko.miaominal.nativebridge.ConfigArea
import dev.akko.miaominal.nativebridge.CredentialAction
import dev.akko.miaominal.nativebridge.CredentialHealth
import dev.akko.miaominal.nativebridge.CredentialKindDto
import dev.akko.miaominal.nativebridge.HostUpdate
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * Release smoke coverage for the optimized native bridge.
 *
 * Runs inside the R8-optimized release APK and proves the whole JNA/UniFFI
 * surface survived optimization:
 *   initialize -> subscribe -> async FFI write -> typed event -> Keystore
 *   platform callback -> async read -> shutdown.
 */
@RunWith(AndroidJUnit4::class)
class ReleaseSmokeTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val host = AndroidPlatformHost(context)

    /** Same embedded root the app singleton uses, without touching its Kotlin API. */
    private val config = BackendConfig(
        dataDir = File(context.noBackupFilesDir, "backend-v2").absolutePath,
        cacheDir = File(context.cacheDir, "backend-v2").absolutePath
    )

    @Test
    fun releaseBackendInitializesEventsPlatformCallbacksAndAsyncCalls() = runBlocking {
        val backend = Backend(host, config)
        val profileId = "p7-release-smoke"
        try {
            // 1. Initialize (loads libmiaominal_android.so through JNA).
            val snapshot = backend.initialize()
            assertTrue(snapshot.initialized)
            assertTrue(backend.buildInfo().abiVersion.isNotBlank())

            // 2. Subscribe before writing so the event cannot be missed.
            val subscription = backend.subscribeFrom(snapshot.sequence)

            // 3. Async FFI write returning a typed record.
            val saved = backend.saveHost(
                HostUpdate(
                    id = profileId,
                    name = "P7 release smoke",
                    group = null,
                    tags = null,
                    kind = null,
                    host = "127.0.0.1",
                    port = 22.toUShort(),
                    username = "smoke",
                    authMethod = AuthMethodDto.PASSWORD,
                    privateKeyPath = null,
                    managedKeyId = null,
                    certificatePath = null,
                    startupCommand = null,
                    environmentVariables = null,
                    shellType = null,
                    isFavorite = null,
                    lastConnectedAtMs = null
                )
            )
            assertEquals(profileId, saved.id)

            // 4. Platform callback: Rust -> Kotlin Keystore -> Rust.
            assertEquals(
                CredentialHealth.PRESENT,
                backend.setHostCredential(
                    profileId,
                    CredentialKindDto.PASSWORD,
                    CredentialAction.Set("release-smoke-not-a-real-secret")
                )
            )
            assertEquals(
                CredentialHealth.PRESENT,
                backend.hostCredentialHealth(profileId, CredentialKindDto.PASSWORD)
            )
            assertEquals(
                CredentialHealth.MISSING,
                backend.setHostCredential(
                    profileId,
                    CredentialKindDto.PASSWORD,
                    CredentialAction.Clear
                )
            )

            // 5. The write above must arrive as a typed event.
            val deadline = System.currentTimeMillis() + 20_000
            var sawConfigChanged = false
            while (!sawConfigChanged && System.currentTimeMillis() < deadline) {
                val batch = subscription.nextBatch(16u, 500uL)
                sawConfigChanged = batch.events.any { sequenced ->
                    val event = sequenced.event
                    event is BackendEvent.ConfigChanged && event.area == ConfigArea.HOSTS
                }
            }
            assertTrue("saveHost must publish ConfigChanged(HOSTS)", sawConfigChanged)

            // 6. Async read-back and the async echo round trip.
            assertTrue(backend.hostsSnapshot().any { it.id == profileId })
            assertEquals("p7-release-smoke", backend.echo("p7-release-smoke", 1u))
        } finally {
            runCatching { backend.deleteHost(profileId) }
            runCatching {
                backend.setHostCredential(
                    profileId,
                    CredentialKindDto.PASSWORD,
                    CredentialAction.Clear
                )
            }
            runCatching { backend.shutdown() }
            runCatching { backend.close() }
        }
    }
}
