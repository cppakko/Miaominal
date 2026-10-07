package dev.akko.miaominal.data

import android.content.Context
import dev.akko.miaominal.nativebridge.Backend
import dev.akko.miaominal.nativebridge.BackendConfig
import dev.akko.miaominal.nativebridge.BackendEvent
import dev.akko.miaominal.nativebridge.BackendException
import dev.akko.miaominal.nativebridge.ConfigArea
import dev.akko.miaominal.nativebridge.EventBatch
import dev.akko.miaominal.nativebridge.EventSubscription
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.io.File

/**
 * Application-scoped owner of the native backend handle.
 *
 * The singleton outlives every Activity, so recreation reuses the same backend
 * and the same event consumer. Only an explicit [restart] replaces the native
 * handle; the process keeps a single embedded business root either way.
 */
class NativeBackendClient private constructor(context: Context) {

    private val appContext = context.applicationContext
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val lifecycleMutex = Mutex()
    private val _state = MutableStateFlow<NativeBackendState>(NativeBackendState.Stopped)
    val state: StateFlow<NativeBackendState> = _state.asStateFlow()

    private val _configChanges = MutableSharedFlow<ConfigArea>(extraBufferCapacity = 64)
    /** Successful configuration writes, by area, for snapshot refresh. */
    val configChanges: SharedFlow<ConfigArea> = _configChanges.asSharedFlow()

    private val _events = MutableSharedFlow<BackendEvent>(extraBufferCapacity = 128)
    /** Every sequenced backend event, for session state and request handling. */
    val events: SharedFlow<BackendEvent> = _events.asSharedFlow()

    @Volatile
    private var session: Session? = null
    private var nextGeneration = 1L

    private class Session(
        val generation: Long,
        val backend: Backend,
        val subscription: EventSubscription,
        val consumer: Job,
    )

    /** Start in the background; safe to call from `Application.onCreate`. */
    fun start() {
        FirstSwitchMigration.runIfNeeded(appContext)
        scope.launch { ensureStarted() }
    }

    /** Current native handle; `null` while stopped or starting. */
    fun currentBackend(): Backend? = session?.backend

    /**
     * Ensure a backend is running and return the resulting state.
     *
     * All lifecycle work runs as a child of the application scope: cancelling
     * the caller (page teardown, Activity recreation) only stops the caller
     * from waiting, never the startup or cleanup itself.
     */
    suspend fun ensureStarted(): NativeBackendState {
        appScoped { lifecycleMutex.withLock { if (session == null) startLocked() } }.join()
        return _state.value
    }

    /** Replace the native backend handle with a fresh one. */
    suspend fun restart(): NativeBackendState {
        appScoped {
            lifecycleMutex.withLock {
                stopLocked()
                startLocked()
            }
        }.join()
        return _state.value
    }

    /** Stop the backend and its event consumer. Idempotent. */
    suspend fun shutdown() {
        appScoped { lifecycleMutex.withLock { stopLocked() } }.join()
    }

    private fun appScoped(block: suspend () -> Unit): Job = scope.launch { block() }

    private suspend fun startLocked() = withContext(NonCancellable) {
        val generation = nextGeneration
        nextGeneration += 1
        _state.value = NativeBackendState.Starting(generation)
        var backend: Backend? = null
        try {
            val handle = Backend(AndroidPlatformHost(appContext), configFor(appContext))
            backend = handle
            val snapshot = handle.initialize()
            val subscription = handle.subscribeFrom(snapshot.sequence)
            val consumer = scope.launch { consume(generation, subscription) }
            session = Session(generation, handle, subscription, consumer)
            val buildInfo = handle.buildInfo()
            _state.value = NativeBackendState.Running(
                generation = generation,
                sequence = snapshot.sequence,
                abiVersion = buildInfo.abiVersion,
                packageVersion = buildInfo.packageVersion,
                deviceLabel = handle.deviceLabel(),
                dataDir = snapshot.dataDir,
                cacheDir = snapshot.cacheDir,
                recentEvents = listOf("${snapshot.sequence} · ${eventLabel(BackendEvent.Ready)}"),
            )
        } catch (error: Throwable) {
            runCatching { backend?.shutdown() }
            runCatching { backend?.close() }
            _state.value = NativeBackendState.Failed(generation, describe(error))
        }
    }

    private suspend fun stopLocked() = withContext(NonCancellable) {
        val current = session ?: return@withContext
        session = null
        runCatching { current.subscription.cancel() }
        current.consumer.cancelAndJoin()
        runCatching { current.subscription.close() }
        runCatching { current.backend.shutdown() }
        runCatching { current.backend.close() }
        _state.value = NativeBackendState.Stopped
    }

    private suspend fun consume(generation: Long, subscription: EventSubscription) {
        try {
            while (true) {
                val batch = subscription.nextBatch(BATCH_SIZE, POLL_TIMEOUT_MS)
                if (batch.events.isNotEmpty() || batch.resyncRequired) {
                    applyBatch(generation, batch)
                }
            }
        } catch (_: BackendException.Closed) {
            // The backend was shut down or the subscription was cancelled.
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (error: Throwable) {
            _state.update { current ->
                if (current is NativeBackendState.Running && current.generation == generation) {
                    NativeBackendState.Failed(generation, describe(error))
                } else {
                    current
                }
            }
        }
    }

    private fun applyBatch(generation: Long, batch: EventBatch) {
        batch.events.forEach { sequenced ->
            val event = sequenced.event
            if (event is BackendEvent.ConfigChanged) {
                _configChanges.tryEmit(event.area)
            }
            _events.tryEmit(event)
        }
        val descriptions = batch.events.map { event ->
            "${event.sequence} · ${eventLabel(event.event)}"
        }
        val resyncNote = if (batch.resyncRequired) {
            listOf("游标已过期，需要重新获取快照")
        } else {
            emptyList()
        }
        _state.update { current ->
            if (current !is NativeBackendState.Running || current.generation != generation) {
                current
            } else {
                current.copy(
                    sequence = batch.events.lastOrNull()?.sequence ?: current.sequence,
                    recentEvents = (current.recentEvents + descriptions + resyncNote)
                        .takeLast(RECENT_EVENT_LIMIT),
                )
            }
        }
    }

    private fun eventLabel(event: BackendEvent): String = when (event) {
        BackendEvent.Ready -> "就绪"
        BackendEvent.ResyncRequired -> "需要重新同步"
        BackendEvent.Shutdown -> "已关闭"
        is BackendEvent.ConfigChanged -> "配置变更（${event.area.name.lowercase()}）"
        is BackendEvent.SessionPhaseChanged -> "会话状态（${event.phase.name.lowercase()}）"
        is BackendEvent.SessionRequest -> "等待认证（${event.request.hopLabel}）"
        is BackendEvent.SessionRequestExpired -> "认证回答已过期"
        is BackendEvent.SessionExited -> "远程会话已退出"
        is BackendEvent.SftpPhaseChanged -> "SFTP 状态（${event.phase.name.lowercase()}）"
        is BackendEvent.SftpRequest -> "SFTP 认证（${event.request.hopLabel}）"
        is BackendEvent.SftpRequestExpired -> "SFTP 认证回答已过期"
        is BackendEvent.SftpDirectoryChanged -> "SFTP 目录已更新"
        is BackendEvent.SftpTransferChanged -> "SFTP 传输状态已更新"
        is BackendEvent.ForwardPhaseChanged -> "转发状态（${event.phase.name.lowercase()}）"
        is BackendEvent.ForwardRelayFailed -> "转发连接失败（${event.failure.nodeLabel}）"
        is BackendEvent.ForwardRequest -> "转发认证（${event.request.hopLabel}）"
        is BackendEvent.ForwardRequestExpired -> "转发认证回答已过期"
        is BackendEvent.SyncPhaseChanged -> "同步状态（${event.phase.name.lowercase()}）"
        is BackendEvent.SyncConflictDetected -> "同步冲突：需要选择保留哪一侧"
        is BackendEvent.SyncAuthenticationRequired -> "同步已暂停：需要解锁或补充凭据"
        is BackendEvent.SyncCapabilityChanged -> "同步能力检查：${event.state.name.lowercase()}"
    }

    companion object {
        private const val DATA_DIR_NAME = "backend-v2"
        private const val BATCH_SIZE = 64u
        private const val POLL_TIMEOUT_MS = 1_000uL
        private const val RECENT_EVENT_LIMIT = 6

        @Volatile
        private var instance: NativeBackendClient? = null

        fun get(context: Context): NativeBackendClient =
            instance ?: synchronized(this) {
                instance ?: NativeBackendClient(context).also { instance = it }
            }

        /** Business data root: excluded from Android automatic backup. */
        fun dataDir(context: Context): File =
            File(context.applicationContext.noBackupFilesDir, DATA_DIR_NAME)

        /** Temporary root for the embedded runtime. */
        fun cacheDir(context: Context): File =
            File(context.applicationContext.cacheDir, DATA_DIR_NAME)

        fun configFor(context: Context): BackendConfig = BackendConfig(
            dataDir = dataDir(context).absolutePath,
            cacheDir = cacheDir(context).absolutePath,
        )

        private fun describe(error: Throwable): String =
            error.message?.takeIf(String::isNotBlank) ?: error.javaClass.simpleName
    }
}

sealed interface NativeBackendState {
    data object Stopped : NativeBackendState

    data class Starting(val generation: Long) : NativeBackendState

    data class Running(
        val generation: Long,
        val sequence: ULong,
        val abiVersion: String,
        val packageVersion: String,
        val deviceLabel: String,
        val dataDir: String,
        val cacheDir: String,
        val recentEvents: List<String>,
    ) : NativeBackendState

    data class Failed(val generation: Long, val reason: String) : NativeBackendState
}
