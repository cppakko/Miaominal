package dev.akko.miaominal.data.agent

import android.content.Context
import dev.akko.miaominal.data.NativeBackendClient
import dev.akko.miaominal.data.ShellType
import dev.akko.miaominal.nativebridge.ShellTypeDto
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.util.concurrent.atomic.AtomicLong

/** Result of preparing a CLI for the launch panel. */
sealed interface AgentLaunchResult {
    data class Ready(
        val probe: AgentProbeResult,
        val history: List<AgentHistoryEntry>,
        val historyError: String? = null
    ) : AgentLaunchResult

    /** The CLI is missing, not logged in, or the server did not start. */
    data class Unavailable(val detail: String) : AgentLaunchResult

    /** The SSH channel or the native backend could not be prepared. */
    data class Failed(val detail: String) : AgentLaunchResult
}

/** UI-facing contract for the AI chat; a fake backs Compose UI tests. */
interface AgentChatRepository {
    val state: StateFlow<AgentSessionState>
    val draft: StateFlow<String>
    val files: StateFlow<List<String>>
    fun addFile(path: String)
    fun removeFile(path: String)

    /** True while a CLI process and its SSH channel are live. */
    val active: StateFlow<Boolean>
    val isActive: Boolean

    fun currentSession(): AgentSessionInfo?

    suspend fun restore()

    /**
     * Connect the CLI and list every conversation it knows about.
     * A blank [cwd] boots the CLI in the SSH login directory; history entries
     * carry their own workspace, so the user is not forced to pick one first.
     */
    suspend fun prepare(hostId: String, cli: AgentCli, cwd: String?): AgentLaunchResult

    suspend fun prepare(hostId: String, cli: AgentCli, cwd: String?, source: AgentConfigSource): AgentLaunchResult =
        if (source == AgentConfigSource.Remote) prepare(hostId, cli, cwd)
        else AgentLaunchResult.Failed("Phone AI configuration is unavailable.")

    fun currentConfigSource(): AgentConfigSource = AgentConfigSource.Remote

    suspend fun startNewSession(): Result<Unit>

    /** Load an existing conversation; [cwd] resumes in that workspace. */
    suspend fun resumeSession(remoteId: String, cwd: String? = null): Result<Unit>

    suspend fun reconnectSession(): Result<Unit> = Result.failure(IllegalStateException("Reconnect is unavailable"))

    suspend fun sendMessage(text: String)

    suspend fun stopTurn()

    suspend fun respondInteraction(requestId: String, answer: AgentInteractionAnswer)

    fun selectModel(model: AgentModel)
    suspend fun selectReasoning(id: String) {}

    fun setDraft(text: String)

    suspend fun closeSession()

    suspend fun closeConnection()
}

/**
 * Application-scoped owner of the AI chat state.
 *
 * One agent session (CLI process + SSH channel) is active at a time; the state
 * survives page switches because the repository outlives every screen. Events
 * from the active adapter fold through [AgentReducer] into [state].
 */
class AgentRepository(
    context: Context,
    private val backendClient: NativeBackendClient,
    private val scope: CoroutineScope,
    private val store: AgentSessionStore = AgentSessionStore(context),
    private val adapterFactory:
        ((AgentCli, String, AgentChannel, CoroutineScope) -> AgentAdapter)? = null,
    private val configs: AgentConfigRepository? = null
) : AgentChatRepository {
    private val _state = MutableStateFlow(AgentSessionState())
    override val state: StateFlow<AgentSessionState> = _state.asStateFlow()

    private val _draft = MutableStateFlow("")
    override val draft: StateFlow<String> = _draft.asStateFlow()
    private val _files = MutableStateFlow<List<String>>(emptyList())
    override val files: StateFlow<List<String>> = _files.asStateFlow()
    override fun addFile(path: String) {
        if (path.isNotBlank()) _files.update { (it + path).distinct().take(8) }
        persist()
    }
    override fun removeFile(path: String) { _files.update { it - path }; persist() }

    private var current: Active? = null
    private var sessionConfig: AgentConfigSnapshot? = null
    private var pendingModelChange: Job? = null
    private var modelSelectionError: String? = null
    private val reconnectMutex = Mutex()
    private val interactionMutex = Mutex()
    private val eventLock = Any()
    @Volatile private var reconnectBuffer: AgentSessionState? = null

    /** Serializes cache writes and clears; all of them share one temp file. */
    private val persistMutex = Mutex()

    /** Bumped by every persist/clear so stale snapshots cannot win. */
    private val persistGeneration = AtomicLong(0)

    private data class Active(
        val hostId: String,
        val cli: AgentCli,
        val cwd: String,
        val channel: AgentChannel,
        val adapter: AgentAdapter,
        val eventsJob: Job
    )

    private val _active = MutableStateFlow(false)
    override val active: StateFlow<Boolean> = _active.asStateFlow()

    override val isActive: Boolean get() = current != null

    override fun currentSession(): AgentSessionInfo? = _state.value.session
    override fun currentConfigSource(): AgentConfigSource = sessionConfig?.profile?.id
        ?.let(AgentConfigSource::Local) ?: AgentConfigSource.Remote

    /** A cold process start keeps only the unsent draft, never the old chat. */
    override suspend fun restore() {
        withContext(Dispatchers.IO) {
            reconnectMutex.withLock {
                persistMutex.withLock restoreCache@ {
                    if (current != null) return@restoreCache
                    persistGeneration.incrementAndGet()
                    store.retainDraftOnly()?.let { savedDraft ->
                        if (_draft.value.isEmpty()) _draft.value = savedDraft
                    }
                }
            }
        }
    }

    /**
     * Start (or restart) the CLI and list its existing conversations.
     *
     * The launch panel calls this before the user chooses between a new
     * conversation and one from the list, so the history is authoritative.
     * Without a [cwd] the CLI starts in the SSH login directory and history is
     * listed across every workspace, each entry carrying its own directory.
     */
    override suspend fun prepare(hostId: String, cli: AgentCli, cwd: String?): AgentLaunchResult =
        prepare(hostId, cli, cwd, AgentConfigSource.Remote)

    override suspend fun prepare(hostId: String, cli: AgentCli, cwd: String?, source: AgentConfigSource): AgentLaunchResult =
        reconnectMutex.withLock {
            val config = try {
                if (source == AgentConfigSource.Remote) null
                else requireNotNull(configs) { "Phone AI configuration is unavailable." }.resolve(source, cli).getOrThrow()
            } catch (error: Exception) {
                if (error is CancellationException) throw error
                return@withLock AgentLaunchResult.Failed(describe(error))
            }
            prepareInternal(hostId, cli, cwd, config)
        }

    private suspend fun prepareInternal(hostId: String, cli: AgentCli, cwd: String?, config: AgentConfigSnapshot? = null): AgentLaunchResult =
        // Native session preparation polls Rust on the caller's dispatcher, so
        // it must never run on the Compose main thread.
        withContext(Dispatchers.IO) {
            closeActive()
            sessionConfig = config
            modelSelectionError = null
            val backend = backendClient.currentBackend()
                ?: return@withContext AgentLaunchResult.Failed("native backend is not running")
            val nativeSession = try {
                backend.openAgentSession(hostId)
            } catch (error: Throwable) {
                if (error is CancellationException) throw error
                return@withContext AgentLaunchResult.Failed(describe(error))
            }
            val channel = NativeAgentChannel(nativeSession, scope) { connectionId ->
                // Release the native registry entry; without this every prepare
                // leaks the previous session and its event queue.
                runCatching { backend.closeAgentSession(connectionId.toULong()) }
            }
            val shellType = runCatching {
                backend.hostsSnapshot().firstOrNull { it.id == hostId }?.shellType?.toModel()
            }.getOrNull() ?: ShellType.Posix
            val workingDirectory = cwd?.trim()?.takeIf(String::isNotBlank)
                ?.let { AgentCommands.fromSftpPath(it, shellType) }
                .orEmpty()
            val adapter = adapterFactory?.invoke(cli, hostId, channel, scope)
                ?: defaultAdapter(cli, hostId, channel, shellType, config)
            val eventsJob = scope.launch {
                adapter.events.collect { event -> handleEvent(event) }
            }
            current = Active(hostId, cli, workingDirectory, channel, adapter, eventsJob)
            _active.value = true
            if (reconnectBuffer == null) _state.value = AgentSessionState(phase = AgentPhase.Connecting)

            val probe = try {
                adapter.start(workingDirectory)
            } catch (error: Throwable) {
                withContext(kotlinx.coroutines.NonCancellable) { closeActive() }
                if (error is CancellationException) throw error
                return@withContext AgentLaunchResult.Failed(describe(error))
            }
            if (!probe.available) {
                closeActive()
                return@withContext AgentLaunchResult.Unavailable(
                    probe.detail ?: "${cli.displayName} is not available on this host"
                )
            }
            var historyError: String? = null
            val history = try {
                adapter.listSessions(workingDirectory.takeIf(String::isNotBlank))
            } catch (error: Throwable) {
                if (error is CancellationException) throw error
                historyError = describe(error)
                emptyList()
            }
            AgentLaunchResult.Ready(probe, history, historyError)
        }

    /** Re-open exactly the current process's remote conversation; never create a replacement. */
    override suspend fun reconnectSession(): Result<Unit> = withContext(Dispatchers.IO) {
        if (!reconnectMutex.tryLock()) return@withContext Result.failure(IllegalStateException("Reconnect in progress"))
        val previous = _state.value
        try {
            val session = previous.session ?: error("No conversation to reconnect")
            val remoteId = session.remoteSessionId ?: error("No remote conversation id")
            synchronized(eventLock) {
                reconnectBuffer = AgentSessionState()
                _state.value = previous.copy(phase = AgentPhase.Connecting, connectionLost = true, error = null)
            }
            val pinnedConfig = sessionConfig
            prepareAndResumeConversation(session, { host, cli, cwd -> prepareInternal(host, cli, cwd, pinnedConfig) }) { id ->
                val active = current ?: error("Connection ended during reconnect")
                active.adapter.loadSession(id, session.cwd)
                // A transport restart must not silently choose another model
                // for the next prompt when the previous choice is still offered.
                val previousModel = reconnectBuffer?.models?.firstOrNull {
                    it.key == previous.selectedModel?.key
                }
                previousModel?.let { model ->
                    active.adapter.selectModel(model)
                    synchronized(eventLock) {
                        reconnectBuffer = reconnectBuffer?.copy(selectedModel = model)
                    }
                }
                val reasoning = previous.selectedReasoning
                val supportedReasoning = previousModel?.reasoningOptions
                    ?.takeIf { it.isNotEmpty() } ?: reconnectBuffer?.reasoningOptions.orEmpty()
                if (reasoning != null && supportedReasoning.any { it.id == reasoning }) {
                    active.adapter.selectReasoning(reasoning)
                }
            }
            synchronized(eventLock) {
                val restored = reconnectBuffer ?: AgentSessionState()
                check(restored.phase != AgentPhase.Failed && restored.phase != AgentPhase.Closed) {
                    restored.error ?: "Connection ended during reconnect"
                }
                _state.value = restored.copy(
                    session = restored.session?.takeIf { it.remoteSessionId == remoteId }
                        ?.let { it.copy(title = it.title ?: session.title) } ?: session,
                    messages = restored.messages.ifEmpty { previous.messages },
                    phase = AgentPhase.Ready, connectionLost = false, error = null
                )
                reconnectBuffer = null
            }
            Result.success(Unit)
        } catch (error: Throwable) {
            closeActive()
            synchronized(eventLock) {
                _state.value = previous.copy(phase = AgentPhase.Failed, connectionLost = true, error = describe(error))
                reconnectBuffer = null
            }
            if (error is CancellationException) throw error
            Result.failure(IllegalStateException(describe(error)))
        } finally {
            reconnectBuffer = null
            persist()
            reconnectMutex.unlock()
        }
    }

    /** Create a new conversation on the prepared CLI. */
    override suspend fun startNewSession(): Result<Unit> = withContext(Dispatchers.IO) {
        val active = current ?: return@withContext Result.failure(
            IllegalStateException("no CLI is prepared")
        )
        if (active.cwd.isBlank()) return@withContext Result.failure(
            IllegalStateException("choose a working directory before starting a conversation")
        )
        resetConversation()
        try {
            active.adapter.createSession(active.cwd)
            Result.success(Unit)
        } catch (error: Throwable) {
            if (error is CancellationException) throw error
            Result.failure(IllegalStateException(describe(error)))
        }
    }

    /**
     * Load an existing conversation on the prepared CLI. The entry's own
     * workspace wins over the process directory, so history can be resumed
     * without choosing a cwd first.
     */
    override suspend fun resumeSession(remoteId: String, cwd: String?): Result<Unit> =
        withContext(Dispatchers.IO) {
            val active = current ?: return@withContext Result.failure(
                IllegalStateException("no CLI is prepared")
            )
            val targetCwd = cwd?.trim()?.takeIf(String::isNotBlank) ?: active.cwd
            resetConversation()
            try {
                active.adapter.loadSession(remoteId, targetCwd)
                if (targetCwd.isNotBlank() && targetCwd != active.cwd) {
                    current = active.copy(cwd = targetCwd)
                }
                Result.success(Unit)
            } catch (error: Throwable) {
                if (error is CancellationException) throw error
                Result.failure(IllegalStateException(describe(error)))
            }
        }

    override suspend fun sendMessage(text: String) = withContext(Dispatchers.IO) {
        val active = current ?: run {
            handleEvent(AgentEvent.ConnectionLost("AI connection is no longer active"))
            return@withContext
        }
        try {
            val sentFiles = _files.value
            pendingModelChange?.join()
            check(current === active) { "AI connection changed before sending." }
            modelSelectionError?.let { error(it) }
            active.adapter.sendMessage(AgentFileReferences.prompt(text, sentFiles))
            // The draft is only cleared once the turn was actually accepted, so
            // a failed send keeps the text for a retry.
            _draft.value = ""
            _files.update { it - sentFiles.toSet() }
            persist()
        } catch (error: Throwable) {
            surfaceError(error)
        }
    }

    override suspend fun stopTurn() {
        withContext(Dispatchers.IO) {
            val active = current ?: return@withContext
            runCatching { active.adapter.cancelTurn() }
                .onFailure { surfaceError(it) }
        }
    }

    override suspend fun respondInteraction(
        requestId: String,
        answer: AgentInteractionAnswer
    ) {
        withContext(Dispatchers.IO) {
            if (!interactionMutex.tryLock()) return@withContext
            try {
                val active = current ?: error("AI connection is no longer active")
                if (_state.value.pendingInteraction?.id != requestId) return@withContext
                _state.update { it.copy(interactionSubmitting = true, error = null) }
                active.adapter.respondInteraction(requestId, answer)
                handleEvent(AgentEvent.InteractionResolved(requestId))
            } catch (error: Throwable) {
                if (error is CancellationException) throw error
                surfaceError(error)
            } finally {
                _state.update { it.copy(interactionSubmitting = false) }
                interactionMutex.unlock()
            }
        }
    }

    override fun selectModel(model: AgentModel) {
        val active = current ?: return
        if (_state.value.isBusy) return
        val previous = pendingModelChange
        pendingModelChange = scope.launch(Dispatchers.IO) {
            previous?.join()
            if (current !== active) return@launch
            try {
                active.adapter.selectModel(model)
                if (current !== active) return@launch
                modelSelectionError = null
                _state.update { it.copy(selectedModel = model) }
                persist()
            } catch (error: Throwable) {
                if (error is CancellationException) throw error
                modelSelectionError = describe(error)
                surfaceError(error)
            }
        }
    }

    override fun setDraft(text: String) {
        _draft.value = text
        persist()
    }

    override suspend fun selectReasoning(id: String) = withContext(Dispatchers.IO) {
        if (_state.value.isBusy || _state.value.reasoningOptions.none { it.id == id }) return@withContext
        runCatching { current?.adapter?.selectReasoning(id) }
            .onFailure { surfaceError(it) }
    }


    /** Close the live session and forget the rendered conversation. */
    override suspend fun closeSession() = withContext(Dispatchers.IO) {
        reconnectMutex.withLock {
            // Invalidate queued saves before clearing: the snapshot is taken at
            // write time, so any save still waiting on the mutex either fails the
            // generation check or sees the already-reset state.
            persistGeneration.incrementAndGet()
            closeActive()
            sessionConfig = null
            modelSelectionError = null
            _state.value = AgentSessionState()
            _draft.value = ""
            _files.value = emptyList()
            persistMutex.withLock { store.clear() }
        }
    }

    /** Close the SSH channel but keep the rendered state (process exit). */
    override suspend fun closeConnection() = withContext(Dispatchers.IO) {
        reconnectMutex.withLock {
            persist()
            closeActive()
            _state.update { state ->
                state.copy(phase = AgentPhase.Closed, connectionLost = true)
            }
        }
    }

    private fun defaultAdapter(
        cli: AgentCli,
        hostId: String,
        channel: AgentChannel,
        shellType: ShellType,
        config: AgentConfigSnapshot?
    ): AgentAdapter =
        when (cli) {
            AgentCli.Codex -> CodexAdapter(
                hostId = hostId,
                channel = channel,
                scope = scope,
                shellType = shellType,
                config = config
            )

            AgentCli.OpenCode -> OpenCodeAcpAdapter(
                hostId = hostId,
                channel = channel,
                scope = scope,
                shellType = shellType,
                config = config
            )
        }

    private fun ShellTypeDto.toModel(): ShellType = when (this) {
        ShellTypeDto.POSIX -> ShellType.Posix
        ShellTypeDto.FISH -> ShellType.Fish
        ShellTypeDto.POWER_SHELL -> ShellType.PowerShell
        ShellTypeDto.CMD -> ShellType.Cmd
    }

    private suspend fun handleEvent(event: AgentEvent) {
        synchronized(eventLock) {
            reconnectBuffer?.let { buffered ->
                reconnectBuffer = reduceEvent(buffered, event)
                return
            }
            _state.update { state -> reduceEvent(state, event) }
        }
        when (event) {
            is AgentEvent.ReasoningConfigured,
            is AgentEvent.TurnEnded,
            is AgentEvent.InteractionRequested,
            is AgentEvent.InteractionResolved,
            is AgentEvent.ConnectionLost,
            is AgentEvent.Closed -> persist()

            else -> Unit
        }
        if (event is AgentEvent.ConnectionLost || event is AgentEvent.Closed) {
            releaseTerminatedSession()
        }
    }

    private fun reduceEvent(state: AgentSessionState, event: AgentEvent): AgentSessionState {
        val snapshot = sessionConfig
        val reduced = AgentReducer.reduce(state, snapshot?.redactEvent(event) ?: event)
        return snapshot?.redactDeltaState(reduced, event) ?: reduced
    }

    /**
     * The CLI process ended or the channel dropped. The repository must stop
     * reporting the session as active (background task count, AI entry points
     * would otherwise reopen a dead chat) and reclaim the native resources.
     *
     * Runs off the event collector's job: that job may be the coroutine which
     * just ended, so it cannot await its own cleanup.
     */
    private fun releaseTerminatedSession() {
        val active = current ?: return
        current = null
        _active.value = false
        active.eventsJob.cancel()
        scope.launch(Dispatchers.IO) {
            runCatching { active.adapter.close() }
            runCatching { active.channel.close() }
        }
    }

    private fun resetConversation() {
        val previous = _state.value
        _state.value = AgentSessionState(
            phase = previous.phase,
            capabilities = previous.capabilities,
            models = previous.models,
            selectedModel = previous.selectedModel,
            reasoningOptions = previous.reasoningOptions,
            selectedReasoning = previous.selectedReasoning
        )
    }

    private suspend fun closeActive() {
        pendingModelChange?.cancel()
        pendingModelChange = null
        val active = current ?: return
        current = null
        _active.value = false
        active.eventsJob.cancel()
        runCatching { active.adapter.close() }
        runCatching { active.channel.close() }
    }

    /**
     * Persist the current state. Saves are serialized and tagged with a
     * generation so only the newest queued save writes; the snapshot is taken
     * inside the same critical section as the write, so a concurrent clear can
     * never be overwritten by a stale snapshot.
     */
    private fun persist() {
        val generation = persistGeneration.incrementAndGet()
        scope.launch(Dispatchers.IO) {
            persistMutex.withLock {
                if (generation != persistGeneration.get()) return@withLock
                store.saveDraft(_draft.value)
            }
        }
    }

    /** Surface a failed operation in the chat instead of crashing the caller. */
    private suspend fun surfaceError(error: Throwable) {
        handleEvent(AgentEvent.ErrorRaised(describe(error)))
    }

    private fun describe(error: Throwable): String =
        (error.message?.takeIf(String::isNotBlank) ?: error.javaClass.simpleName).let {
            sessionConfig?.redact(it) ?: it
        }
}
