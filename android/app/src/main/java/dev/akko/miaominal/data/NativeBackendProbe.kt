package dev.akko.miaominal.data

import android.content.Context
import dev.akko.miaominal.nativebridge.Backend
import dev.akko.miaominal.nativebridge.BackendConfig
import dev.akko.miaominal.nativebridge.BuildInfo
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

/**
 * P0 smoke probe: build info, an asynchronous round trip, and a platform
 * callback that goes through the Android Keystore. It deliberately does not
 * initialize the embedded runtime so the app-level backend keeps the single
 * business root.
 */
object NativeBackendProbe {
    private const val PROBE_KEY = "p0-probe"
    private const val PROBE_VALUE = "keystore-roundtrip-ok"

    suspend fun run(context: Context): NativeProbeResult = withContext(Dispatchers.IO) {
        val host = AndroidPlatformHost(context.applicationContext)
        val probeRoot = File(context.cacheDir, "p0-probe")
        val backend = Backend(
            host,
            BackendConfig(
                dataDir = File(probeRoot, "data").absolutePath,
                cacheDir = File(probeRoot, "cache").absolutePath
            )
        )
        try {
            val buildInfo = backend.buildInfo()
            val echo = backend.echo("miaominal-p0", 80u)
            var secret = backend.probeSecret(PROBE_KEY)
            if (secret == null) {
                host.rememberProbeSecret(PROBE_KEY, PROBE_VALUE)
                secret = backend.probeSecret(PROBE_KEY)
            }
            NativeProbeResult(
                buildInfo = buildInfo,
                echo = echo,
                secret = secret ?: "(unavailable)",
                deviceLabel = backend.deviceLabel(),
            )
        } finally {
            runCatching { backend.shutdown() }
            runCatching { backend.close() }
        }
    }
}

data class NativeProbeResult(
    val buildInfo: BuildInfo,
    val echo: String,
    val secret: String,
    val deviceLabel: String,
)
