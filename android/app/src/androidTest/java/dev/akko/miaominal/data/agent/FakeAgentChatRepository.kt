package dev.akko.miaominal.data.agent

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Deterministic [AgentChatRepository] for Compose UI tests: no native backend,
 * no SSH, no HTTP. Actions are recorded so the test can assert the exact
 * protocol answer the screen produced.
 */
class FakeAgentChatRepository : AgentChatRepository {

    private val _state = MutableStateFlow(AgentSessionState())
    override val state: StateFlow<AgentSessionState> = _state.asStateFlow()

    private val _draft = MutableStateFlow("")
    override val draft: StateFlow<String> = _draft.asStateFlow()
    private val _files = MutableStateFlow<List<String>>(emptyList())
    override val files: StateFlow<List<String>> = _files.asStateFlow()
    override fun addFile(path: String) { _files.value = (_files.value + path).distinct() }
    override fun removeFile(path: String) { _files.value -= path }

    private val _active = MutableStateFlow(false)
    override val active: StateFlow<Boolean> = _active.asStateFlow()

    override var isActive: Boolean = false

    var prepareResult: AgentLaunchResult = AgentLaunchResult.Ready(
        probe = AgentProbeResult(available = true, version = "test-1"),
        history = emptyList()
    )

    var preparedWith: Triple<String, AgentCli, String?>? = null
    val preparedSources = mutableListOf<AgentConfigSource>()
    var startedNew: Boolean = false
    var resumedId: String? = null
    var resumedCwd: String? = null
    val sentMessages = mutableListOf<String>()
    val responses = mutableListOf<Pair<String, AgentInteractionAnswer>>()
    var selectedModel: AgentModel? = null
    var stopped: Boolean = false
    var closedSession: Boolean = false

    fun setState(state: AgentSessionState) {
        _state.value = state
    }

    override fun currentSession(): AgentSessionInfo? = _state.value.session

    override suspend fun restore() = Unit

    override suspend fun prepare(
        hostId: String,
        cli: AgentCli,
        cwd: String?
    ): AgentLaunchResult {
        preparedWith = Triple(hostId, cli, cwd)
        isActive = true
        _active.value = true
        return prepareResult
    }

    override suspend fun prepare(hostId: String, cli: AgentCli, cwd: String?, source: AgentConfigSource): AgentLaunchResult {
        preparedSources += source
        return prepare(hostId, cli, cwd)
    }

    override fun currentConfigSource(): AgentConfigSource = preparedSources.lastOrNull() ?: AgentConfigSource.Remote

    override suspend fun startNewSession(): Result<Unit> {
        startedNew = true
        return Result.success(Unit)
    }

    override suspend fun resumeSession(remoteId: String, cwd: String?): Result<Unit> {
        resumedId = remoteId
        resumedCwd = cwd
        return Result.success(Unit)
    }

    override suspend fun sendMessage(text: String) {
        sentMessages += text
        _draft.value = ""
    }

    override suspend fun stopTurn() {
        stopped = true
    }

    override suspend fun respondInteraction(
        requestId: String,
        answer: AgentInteractionAnswer
    ) {
        responses += requestId to answer
    }

    override fun selectModel(model: AgentModel) {
        selectedModel = model
    }

    override fun setDraft(text: String) {
        _draft.value = text
    }

    override suspend fun closeSession() {
        closedSession = true
        isActive = false
        _active.value = false
    }

    override suspend fun closeConnection() {
        isActive = false
        _active.value = false
    }
}
