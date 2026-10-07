package dev.akko.miaominal.data.agent

import dev.akko.miaominal.data.ShellType
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * OpenCode adapter over the Agent Client Protocol (`opencode acp`).
 *
 * ACP is JSON-RPC 2.0 over stdio, so OpenCode uses the same exec-channel
 * primitive as Codex: no HTTP server, no SSH tunnel, no password and no port
 * retry. Capabilities are negotiated during `initialize`; history listing and
 * session loading are only attempted when the agent advertises them.
 */
class OpenCodeAcpAdapter(
    override val cli: AgentCli = AgentCli.OpenCode,
    private val hostId: String,
    channel: AgentChannel,
    private val scope: CoroutineScope,
    private val clientName: String = "miaominal_android",
    private val clientVersion: String = "1",
    private val json: JsonCodec = JsonCodec(),
    private val requestTimeoutMs: Long = REQUEST_TIMEOUT_MS,
    private val shellType: ShellType = ShellType.Posix,
    private val config: AgentConfigSnapshot? = null
) : AgentAdapter {

    private val channel = AgentProcessChannel(channel, cli, shellType, config)
    private val modelMutex = Mutex()
    private val configuredModels = config?.profile?.models?.map {
        AgentModel(AgentProcessChannel.PROVIDER, "${AgentProcessChannel.PROVIDER}/$it", it)
    }.orEmpty()
    private var configuredModelConfirmed = config == null

    private var advertised = AgentCapabilities(
        history = false,
        modelSelection = false,
        approvals = true,
        followUpQuestions = false,
        fileChanges = true
    )

    override val capabilities: AgentCapabilities get() = advertised

    private val outbox = Channel<AgentEvent>(EVENT_QUEUE_CAPACITY)
    override val events: Flow<AgentEvent> = outbox.receiveAsFlow()

    private val framer = LineFramer()
    private val nextRequestId = AtomicLong(1)
    private val pending = ConcurrentHashMap<Long, CompletableDeferred<Map<String, Any?>>>()
    private val serverRequests = ConcurrentHashMap<String, PendingServerRequest>()

    private var execChannelId: Long = -1
    private var consumer: Job? = null
    private var sessionId: String? = null
    private var cwd: String = ""
    private var listSupported = false
    private var loadSupported = false
    private var agentVersion: String? = null

    private var modelConfigId: String? = null
    private var reasoningConfigId: String? = null
    private var reasoningOptions: List<AgentReasoningOption> = emptyList()
    private var selectedModel: AgentModel? = null

    private var turnCounter = 0L
    private var currentTurnId: String? = null
    private var currentAssistantMessageId: String? = null
    private var currentPrompt: Job? = null

    /** Accumulated tool state: ACP tool updates are partial patches. */
    private val toolStates = ConcurrentHashMap<String, AcpToolState>()

    /** History replay accumulators (session/load streams chunks before replying). */
    private var replaying = false
    private val replayMessages = LinkedHashMap<String, ReplayMessage>()

    private val stderrTail = StringBuilder()

    @Volatile
    private var closed = false

    private data class PendingServerRequest(
        val rpcId: Any,
        val kind: ServerRequestKind
    )

    private enum class ServerRequestKind { Permission, Elicitation }

    private data class ReplayMessage(
        val role: AgentRole,
        val blocks: MutableList<ContentBlock>
    )

    private data class AcpToolState(
        val id: String,
        var title: String?,
        var name: String?,
        var status: ToolStatus,
        var input: String?,
        var output: String?,
        var error: String?,
        val changes: MutableList<FileChange>
    )

    override suspend fun start(cwd: String): AgentProbeResult {
        this.cwd = cwd
        execChannelId = channel.launch(cwd)
        consumer = scope.launch { consumeChannel() }

        val response = try {
            channel.awaitReady(requestTimeoutMs)
            request(
                "initialize",
                mapOf(
                    "protocolVersion" to ACP_PROTOCOL_VERSION,
                    "clientCapabilities" to mapOf(
                        // Tools run on the remote host; the client offers no
                        // local filesystem or terminal execution surface.
                        "fs" to mapOf(
                            "readTextFile" to false,
                            "writeTextFile" to false
                        ),
                        "terminal" to false
                    ),
                    "clientInfo" to mapOf(
                        "name" to clientName,
                        "title" to "Miaominal",
                        "version" to clientVersion
                    )
                )
            )
        } catch (error: Throwable) {
            kotlinx.coroutines.withContext(kotlinx.coroutines.NonCancellable) { close() }
            if (error is kotlinx.coroutines.CancellationException) throw error
            return AgentProbeResult(false, null, redact(describe(error, stderrTail)))
        }

        val agentCapabilities = response.mapAt("agentCapabilities").orEmpty()
        val sessionCapabilities = agentCapabilities.mapAt("sessionCapabilities").orEmpty()
        listSupported = sessionCapabilities["list"] != null
        loadSupported = agentCapabilities.booleanAt("loadSession") == true
        agentVersion = response.mapAt("agentInfo")?.stringAt("version")
            ?: response.stringAt("version")

        advertised = AgentCapabilities(
            history = listSupported,
            // Config options may expose a model selector; the UI only shows a
            // picker once a session returned models.
            modelSelection = true,
            approvals = true,
            followUpQuestions = true,
            fileChanges = true
        )

        emit(
            AgentEvent.SessionOpened(
                session = AgentSessionInfo(cli = cli, hostId = hostId, cwd = cwd),
                capabilities = advertised
            )
        )
        emit(AgentEvent.PhaseChanged(AgentPhase.Ready))
        if (config != null) {
            selectedModel = configuredModels.first { it.name == config.profile.defaultModel }
            emit(AgentEvent.ModelsLoaded(configuredModels, selectedModel))
        }
        return AgentProbeResult(true, agentVersion)
    }

    override suspend fun listSessions(cwd: String?): List<AgentHistoryEntry> {
        if (!listSupported) return emptyList()
        val params = mutableMapOf<String, Any?>()
        cwd?.trim()?.takeIf(String::isNotBlank)?.let { params["cwd"] = it }
        val response = request("session/list", params)
        return response.listAt("sessions").mapNotNull { raw ->
            val session = raw.asMap() ?: return@mapNotNull null
            val id = session.stringAt("sessionId") ?: return@mapNotNull null
            val sessionCwd = session.stringAt("cwd")?.takeIf(String::isNotBlank)
            AgentHistoryEntry(
                remoteId = id,
                title = session.stringAt("title")?.takeIf(String::isNotBlank) ?: id,
                preview = sessionCwd,
                updatedAtMs = session.stringAt("updatedAt")?.let(::parseIsoInstant),
                cwd = sessionCwd
            )
        }
    }

    override suspend fun createSession(cwd: String): String {
        configuredModelConfirmed = config == null
        val response = request(
            "session/new",
            mapOf("cwd" to cwd, "mcpServers" to emptyList<Any>())
        )
        val id = response.stringAt("sessionId")
            ?: error("opencode acp did not return a session id")
        sessionId = id
        this.cwd = cwd
        applyConfigOptions(response.listAt("configOptions"))
        applyConfiguredModel()
        resetTurnState()
        emit(
            AgentEvent.SessionOpened(
                session = AgentSessionInfo(
                    cli = cli,
                    hostId = hostId,
                    cwd = cwd,
                    remoteSessionId = id
                ),
                capabilities = advertised
            )
        )
        return id
    }

    override suspend fun loadSession(remoteId: String, cwd: String?) {
        configuredModelConfirmed = config == null
        val targetCwd = cwd?.trim()?.takeIf(String::isNotBlank) ?: this.cwd
        // ACP requires an absolute cwd on session/load; old history entries
        // without one must not turn into an invalid request.
        require(targetCwd.isNotBlank()) {
            "the conversation did not report a working directory"
        }
        replaying = true
        replayMessages.clear()
        val response = try {
            request(
                "session/load",
                mapOf(
                    "sessionId" to remoteId,
                    "cwd" to targetCwd,
                    "mcpServers" to emptyList<Any>()
                )
            )
        } finally {
            replaying = false
        }
        sessionId = remoteId
        this.cwd = targetCwd
        applyConfigOptions(response.listAt("configOptions"))
        applyConfiguredModel()
        resetTurnState()
        emit(
            AgentEvent.SessionOpened(
                session = AgentSessionInfo(
                    cli = cli,
                    hostId = hostId,
                    cwd = targetCwd,
                    remoteSessionId = remoteId
                ),
                capabilities = advertised
            )
        )
    }

    override suspend fun sendMessage(text: String) = modelMutex.withLock {
        check(configuredModelConfirmed) { "OpenCode has not confirmed the phone configuration's model." }
        val activeSession = sessionId ?: error("no opencode session is active")
        turnCounter += 1
        val turnId = "turn:$turnCounter"
        currentTurnId = turnId
        currentAssistantMessageId = "asst:$turnCounter"
        emit(AgentEvent.TurnStarted(turnId))
        // ACP does not echo a live user message, so render it locally.
        emit(
            AgentEvent.MessageUpserted(
                AgentMessage(
                    id = "user:$turnCounter",
                    role = AgentRole.User,
                    blocks = listOf(ContentBlock.Text("user:$turnCounter", text))
                )
            )
        )

        // Write the prompt on the caller's coroutine: a dead stdin must be
        // reported to the caller (the repository keeps the draft for a retry).
        // Only the response wait — which has no timeout — moves to the
        // background.
        val (requestId, response) = try {
            sendRequest(
                "session/prompt",
                mapOf(
                    "sessionId" to activeSession,
                    "prompt" to listOf(mapOf("type" to "text", "text" to text))
                )
            )
        } catch (error: Throwable) {
            currentTurnId = null
            currentAssistantMessageId = null
            runCatching { emit(AgentEvent.TurnEnded(turnId, AgentTurnStatus.Failed)) }
            throw error
        }

        currentPrompt = scope.launch {
            try {
                val result = response.await()
                val stopReason = result.stringAt("stopReason") ?: "end_turn"
                currentTurnId = null
                currentAssistantMessageId = null
                emit(AgentEvent.TurnEnded(turnId, stopReasonStatus(stopReason)))
            } catch (error: Throwable) {
                if (!closed) {
                    currentTurnId = null
                    currentAssistantMessageId = null
                    emit(AgentEvent.TurnEnded(turnId, AgentTurnStatus.Failed))
                    emit(AgentEvent.ErrorRaised(describe(error, stderrTail)))
                }
            } finally {
                pending.remove(requestId)
            }
        }
    }

    override suspend fun cancelTurn() {
        val activeSession = sessionId ?: return
        cancelPendingPermissions()
        send(
            mapOf(
                "method" to "session/cancel",
                "params" to mapOf("sessionId" to activeSession)
            )
        )
    }

    override suspend fun respondInteraction(requestId: String, answer: AgentInteractionAnswer) {
        val pendingRequest = serverRequests[requestId] ?: return
        when (pendingRequest.kind) {
            ServerRequestKind.Permission -> {
                val outcome = when (answer) {
                    is AgentInteractionAnswer.Option -> mapOf(
                        "outcome" to "selected",
                        "optionId" to answer.optionId
                    )

                    is AgentInteractionAnswer.Answers -> mapOf(
                        "outcome" to "selected",
                        "optionId" to answer.entries.firstOrNull()
                            ?.answers?.firstOrNull().orEmpty()
                    )
                }
                // Send before dropping the pending entry: a failed write keeps
                // the request answerable instead of silently losing it.
                respond(pendingRequest.rpcId, mapOf("outcome" to outcome))
            }

            ServerRequestKind.Elicitation -> {
                val content = when (answer) {
                    is AgentInteractionAnswer.Answers -> answer.entries.associate { entry ->
                        entry.questionId to entry.answers.firstOrNull().orEmpty()
                    }

                    is AgentInteractionAnswer.Option -> mapOf("value" to answer.optionId)
                }
                respond(
                    pendingRequest.rpcId,
                    mapOf("action" to "accept", "content" to content)
                )
            }
        }
        serverRequests.remove(requestId)
        emit(AgentEvent.InteractionResolved(requestId))
    }

    override suspend fun selectModel(model: AgentModel) = modelMutex.withLock {
        if (config != null) require(configuredModels.any { it.modelId == model.modelId })
        val activeSession = sessionId ?: error("No active OpenCode session.")
        val configId = modelConfigId ?: error("This OpenCode version cannot select models. Upgrade the remote CLI.")
        configuredModelConfirmed = config == null
        val response = request("session/set_config_option", mapOf(
            "sessionId" to activeSession, "configId" to configId, "value" to model.modelId
        ))
        val returned = response.listAt("configOptions")
        val option = returned.mapNotNull { it.asMap() }.firstOrNull { it.stringAt("id") == configId }
        check(option?.stringAt("currentValue") == model.modelId) {
            "OpenCode did not confirm the selected model. Check the configuration or upgrade the remote CLI."
        }
        applyConfigOptions(returned)
        selectedModel = model
        configuredModelConfirmed = true
        if (config != null) emit(AgentEvent.ModelsLoaded(configuredModels, model))
    }

    private suspend fun applyConfiguredModel() {
        if (config == null) return
        selectModel(selectedModel ?: configuredModels.first { it.name == config.profile.defaultModel })
    }

    override suspend fun close() {
        if (closed) return
        closed = true
        currentPrompt?.cancel()
        runCatching { channel.close() }
        consumer?.cancel()
        outbox.close()
    }

    override suspend fun selectReasoning(id: String) {
        val activeSession = sessionId ?: error("No active session")
        val configId = reasoningConfigId ?: error("Reasoning selection is unavailable")
        require(reasoningOptions.any { it.id == id }) { "Unsupported reasoning effort: $id" }
        val response = request("session/set_config_option", mapOf(
            "sessionId" to activeSession, "configId" to configId, "value" to id
        ))
        applyConfigOptions(response.listAt("configOptions"))
    }

    // ------------------------------------------------------------------
    // Transport
    // ------------------------------------------------------------------

    private suspend fun consumeChannel() {
        channel.events().collect { event ->
            when (event) {
                is AgentChannelEventModel.Stdout -> {
                    if (closed) return@collect
                    framer.append(event.data).forEach { line -> handleLine(line) }
                }

                is AgentChannelEventModel.Stderr ->
                    appendStderr(String(event.data, Charsets.UTF_8))

                is AgentChannelEventModel.ExecExited -> fail(
                    "opencode acp exited (${describeExit(event.exitCode, event.signal, stderrTail)})"
                )

                is AgentChannelEventModel.ExecFailed -> fail(event.message)

                is AgentChannelEventModel.ChannelClosed,
                is AgentChannelEventModel.Closed -> closeFromRemote()

                is AgentChannelEventModel.Error -> fail(event.message)

                is AgentChannelEventModel.HostKeyPrompt -> {
                    runCatching {
                        channel.respondHostKey(
                            event.requestId,
                            AgentHostKeyDecisionModel.Reject
                        )
                    }
                    fail(
                        "host key for ${event.host}:${event.port} is not trusted yet; " +
                            "open a terminal session to ${event.host} once, then retry"
                    )
                }

                is AgentChannelEventModel.AuthenticationPrompt -> {
                    runCatching {
                        channel.respondKeyboardInteractive(event.requestId, emptyList())
                    }
                    fail(
                        "keyboard-interactive authentication is not available for AI " +
                            "sessions; use a key or saved password"
                    )
                }

                else -> Unit
            }
        }
    }

    private suspend fun handleLine(line: String) {
        val message = json.parse(line) ?: run {
            logTechnical("unparsed output: $line")
            return
        }
        val rawId = message["id"]
        val method = message.stringAt("method")
        when {
            rawId != null && method == null -> completePending(rawId, message)
            rawId != null && method != null ->
                handleServerRequest(rawId, method, message.mapAt("params").orEmpty())

            method != null -> handleNotification(method, message.mapAt("params").orEmpty())
        }
    }

    private fun completePending(rawId: Any, message: Map<String, Any?>) {
        val id = rawId.asLong() ?: return
        val deferred = pending.remove(id) ?: return
        val error = message.mapAt("error")
        if (error != null) {
            deferred.completeExceptionally(
                IllegalStateException(error.stringAt("message") ?: "acp request failed")
            )
            return
        }
        deferred.complete(message.mapAt("result").orEmpty())
    }

    /**
     * Send a request and hand back its id plus the response deferred without
     * awaiting it; the caller decides how a write failure is reported.
     */
    private suspend fun sendRequest(
        method: String,
        params: Map<String, Any?>
    ): Pair<Long, CompletableDeferred<Map<String, Any?>>> {
        val id = nextRequestId.getAndIncrement()
        val deferred = CompletableDeferred<Map<String, Any?>>()
        pending[id] = deferred
        try {
            send(mapOf("id" to id, "method" to method, "params" to params))
        } catch (error: Throwable) {
            pending.remove(id)
            throw error
        }
        return id to deferred
    }

    private suspend fun request(
        method: String,
        params: Map<String, Any?>,
        timeoutMs: Long? = requestTimeoutMs
    ): Map<String, Any?> {
        val (id, deferred) = sendRequest(method, params)
        val result = if (timeoutMs == null) {
            deferred.await()
        } else {
            withTimeoutOrNull(timeoutMs) { deferred.await() }
        }
        pending.remove(id)
        if (result == null) {
            throw IllegalStateException("acp request timed out: $method")
        }
        return result
    }

    private suspend fun send(message: Map<String, Any?>) {
        val line = json.stringify(message) + "\n"
        channel.writeStdin(execChannelId, line.toByteArray(Charsets.UTF_8))
    }

    private suspend fun respond(rpcId: Any, result: Any) {
        send(mapOf("id" to rpcId, "result" to result))
    }

    private suspend fun respondError(rpcId: Any, code: Int, message: String) {
        send(mapOf("id" to rpcId, "error" to mapOf("code" to code, "message" to message)))
    }

    // ------------------------------------------------------------------
    // Server requests
    // ------------------------------------------------------------------

    private suspend fun handleServerRequest(
        rpcId: Any,
        method: String,
        params: Map<String, Any?>
    ) {
        val key = rpcKey(rpcId)
        val interactionId = "opencode:request:$key"
        when (method) {
            "session/request_permission" -> {
                serverRequests[interactionId] = PendingServerRequest(
                    canonicalRpcId(rpcId),
                    ServerRequestKind.Permission
                )
                val toolCall = params.mapAt("toolCall").orEmpty()
                val options = params.listAt("options").mapNotNull { raw ->
                    val option = raw.asMap() ?: return@mapNotNull null
                    val optionId = option.stringAt("optionId") ?: return@mapNotNull null
                    InteractionOption(
                        id = optionId,
                        label = option.stringAt("name") ?: optionId,
                        description = option.stringAt("kind")
                    )
                }
                emit(
                    AgentEvent.InteractionRequested(
                        InteractionRequest(
                            id = interactionId,
                            kind = InteractionKind.Approval,
                            turnId = currentTurnId,
                            toolId = toolCall.stringAt("toolCallId"),
                            title = toolCall.stringAt("title")
                                ?: "OpenCode needs permission",
                            detail = toolCall.stringAt("kind"),
                            options = options
                        )
                    )
                )
            }

            "elicitation/create" -> {
                serverRequests[interactionId] = PendingServerRequest(
                    canonicalRpcId(rpcId),
                    ServerRequestKind.Elicitation
                )
                val questions = params.mapAt("requestedSchema")
                    ?.mapAt("properties")
                    .orEmpty()
                    .map { (field, raw) ->
                        val property = raw.asMap().orEmpty()
                        InteractionQuestion(
                            id = field,
                            question = property.stringAt("title")
                                ?: property.stringAt("description")
                                ?: field,
                            freeform = true
                        )
                    }
                emit(
                    AgentEvent.InteractionRequested(
                        InteractionRequest(
                            id = interactionId,
                            kind = InteractionKind.Question,
                            turnId = currentTurnId,
                            toolId = null,
                            title = params.stringAt("message")
                                ?: "OpenCode needs your input",
                            questions = questions
                        )
                    )
                )
            }

            // The client advertised no filesystem/terminal surface, so an
            // agent must not call these; answer with a protocol error.
            else -> respondError(
                canonicalRpcId(rpcId),
                METHOD_NOT_FOUND,
                "unsupported server request: $method"
            )
        }
    }

    private suspend fun cancelPendingPermissions() {
        val pendingIds = serverRequests.keys.toList()
        for (id in pendingIds) {
            val request = serverRequests.remove(id) ?: continue
            if (request.kind == ServerRequestKind.Permission) {
                respond(
                    request.rpcId,
                    mapOf("outcome" to mapOf("outcome" to "cancelled"))
                )
            } else {
                respond(request.rpcId, mapOf("action" to "cancel"))
            }
            emit(AgentEvent.InteractionResolved(id))
        }
    }

    // ------------------------------------------------------------------
    // Session update notifications
    // ------------------------------------------------------------------

    private suspend fun handleNotification(method: String, params: Map<String, Any?>) {
        if (method != "session/update") {
            logTechnical("notification $method: ${json.stringify(params)}")
            return
        }
        val update = params.mapAt("update") ?: return
        when (update.stringAt("sessionUpdate")) {
            "user_message_chunk" -> handleMessageChunk(update, AgentRole.User)
            "agent_message_chunk" -> handleMessageChunk(update, AgentRole.Assistant)
            "agent_thought_chunk" -> handleThoughtChunk(update)
            "tool_call" -> handleToolCall(update)
            "tool_call_update" -> handleToolCall(update)
            "plan" -> handlePlan(update)
            "usage_update" -> handleUsage(update)
            "session_info_update" -> handleSessionInfo(update)
            "config_option_update" -> applyConfigOptions(update.listAt("configOptions"))
            "available_commands_update" -> Unit
            "current_mode_update" -> logTechnical(
                "mode: " + (update.stringAt("currentModeId") ?: "")
            )

            else -> logTechnical(
                "update: " + (update.stringAt("sessionUpdate") ?: "unknown")
            )
        }
    }

    private suspend fun handleMessageChunk(update: Map<String, Any?>, role: AgentRole) {
        val text = update.mapAt("content")?.stringAt("text") ?: return
        // Live user messages are rendered optimistically when the prompt is
        // sent; only history replay renders the agent's user-message chunks.
        if (role == AgentRole.User && !replaying) return

        if (replaying) {
            val messageId = replayMessageId(role, update)
            appendReplayBlock(
                messageId = messageId,
                role = role,
                placeholder = ContentBlock.Text("$messageId:text", ""),
                text = text
            )
            return
        }
        val messageId = update.stringAt("messageId")
            ?: currentAssistantMessageId
            ?: return
        emit(
            AgentEvent.BlockDelta(
                messageId = messageId,
                blockId = "$messageId:text",
                text = text,
                kind = DeltaKind.Text
            )
        )
    }

    private suspend fun handleThoughtChunk(update: Map<String, Any?>) {
        val text = update.mapAt("content")?.stringAt("text") ?: return
        if (replaying) {
            val messageId = replayMessageId(AgentRole.Assistant, update)
            appendReplayBlock(
                messageId = messageId,
                role = AgentRole.Assistant,
                placeholder = ContentBlock.Reasoning("$messageId:thought", ""),
                text = text
            )
            return
        }
        val messageId = update.stringAt("messageId")
            ?: currentAssistantMessageId
            ?: return
        emit(
            AgentEvent.BlockDelta(
                messageId = messageId,
                blockId = "$messageId:thought",
                text = text,
                kind = DeltaKind.Reasoning
            )
        )
    }

    private suspend fun handleToolCall(update: Map<String, Any?>) {
        val toolCallId = update.stringAt("toolCallId") ?: return
        val state = toolStates.getOrPut(toolCallId) {
            AcpToolState(
                id = toolCallId,
                title = null,
                name = null,
                status = ToolStatus.Pending,
                input = null,
                output = null,
                error = null,
                changes = mutableListOf()
            )
        }
        update.stringAt("name")?.let { state.name = it }
        update.stringAt("title")?.let { state.title = it }
        toolStatusFromAcp(update.stringAt("status"))?.let { state.status = it }
        update["rawInput"]?.let { state.input = json.stringify(it) }
        update["rawOutput"]?.let { state.output = json.stringify(it) }

        val content = update.listAt("content")
        if (content.isNotEmpty()) {
            val texts = mutableListOf<String>()
            val diffs = mutableListOf<FileChange>()
            content.forEach { raw ->
                val entry = raw.asMap() ?: return@forEach
                when (entry.stringAt("type")) {
                    "content" -> entry.mapAt("content")
                        ?.stringAt("text")
                        ?.let(texts::add)

                    "diff" -> {
                        val path = entry.stringAt("path") ?: return@forEach
                        diffs += FileChange(
                            path = path,
                            kind = "diff",
                            diff = simpleDiff(
                                entry.stringAt("oldText"),
                                entry.stringAt("newText").orEmpty()
                            )
                        )
                    }

                    "terminal" -> entry.stringAt("terminalId")
                        ?.let { terminal -> texts += "terminal: $terminal" }
                }
            }
            if (texts.isNotEmpty()) {
                state.output = listOfNotNull(
                    state.output,
                    texts.joinToString("\n")
                ).joinToString("\n")
            }
            if (diffs.isNotEmpty()) {
                state.changes.clear()
                state.changes += diffs
            }
        }

        val blocks = mutableListOf<ContentBlock>(state.toBlock())
        if (state.changes.isNotEmpty()) {
            blocks += ContentBlock.FileChanges(
                id = "$toolCallId:diff",
                changes = state.changes.toList(),
                status = state.status
            )
        }
        emit(
            AgentEvent.MessageUpserted(
                AgentMessage(
                    id = toolCallId,
                    role = AgentRole.Assistant,
                    blocks = blocks,
                    streaming = state.status == ToolStatus.Running ||
                        state.status == ToolStatus.Pending
                )
            )
        )
    }

    private suspend fun handlePlan(update: Map<String, Any?>) {
        val steps = update.listAt("entries").mapNotNull { raw ->
            val entry = raw.asMap() ?: return@mapNotNull null
            val content = entry.stringAt("content") ?: return@mapNotNull null
            PlanStep(content, entry.stringAt("status") ?: "pending")
        }
        if (steps.isEmpty()) return
        val session = sessionId ?: "session"
        emit(
            AgentEvent.MessageUpserted(
                AgentMessage(
                    id = "plan:$session",
                    role = AgentRole.Assistant,
                    blocks = listOf(ContentBlock.Plan("plan", null, steps))
                )
            )
        )
    }

    private suspend fun handleUsage(update: Map<String, Any?>) {
        val used = update.longAt("used") ?: 0
        val cost = update.mapAt("cost")?.doubleAt("amount") ?: 0.0
        emit(AgentEvent.UsageUpdated(AgentUsage(inputTokens = used, costUsd = cost)))
    }

    private suspend fun handleSessionInfo(update: Map<String, Any?>) {
        val title = update.stringAt("title") ?: return
        val session = sessionId ?: return
        emit(
            AgentEvent.SessionOpened(
                session = AgentSessionInfo(
                    cli = cli,
                    hostId = hostId,
                    cwd = cwd,
                    remoteSessionId = session,
                    title = title
                ),
                capabilities = advertised
            )
        )
    }

    // ------------------------------------------------------------------
    // Config options (model selector)
    // ------------------------------------------------------------------

    private suspend fun applyConfigOptions(rawOptions: List<Any?>) {
        val options = rawOptions.mapNotNull { it.asMap() }
        val reasoning = options.firstOrNull { it.stringAt("category") == "thought_level" }
            ?: options.firstOrNull { it.stringAt("id") in listOf("reasoning_effort", "reasoningEffort", "thought_level") }
        reasoningConfigId = reasoning?.stringAt("id")
        reasoningOptions = buildList {
            collectConfigValues(reasoning?.listAt("options").orEmpty()) { value, name ->
                add(AgentReasoningOption(value, name))
            }
        }
        emit(AgentEvent.ReasoningConfigured(reasoningOptions, reasoning?.stringAt("currentValue")))
        val modelOption = options.firstOrNull { it.stringAt("category") == "model" }
            ?: options.firstOrNull { it.stringAt("id") == "model" }
            ?: return
        modelConfigId = modelOption.stringAt("id")
        if (config != null) {
            // Old history may return a different provider. Do not adopt it;
            // create/load will explicitly apply and verify our selected model.
            emit(AgentEvent.ModelsLoaded(configuredModels, selectedModel))
            if (modelOption.stringAt("currentValue") != selectedModel?.modelId) configuredModelConfirmed = false
            return
        }
        val models = mutableListOf<AgentModel>()
        collectConfigValues(modelOption.listAt("options")) { value, name ->
            models += AgentModel(providerId = "opencode", modelId = value, name = name)
        }
        if (models.isEmpty()) return
        val current = modelOption.stringAt("currentValue")
        val selected = models.firstOrNull { it.modelId == current }
            ?: selectedModel?.takeIf { model -> models.any { it.modelId == model.modelId } }
            ?: models.first()
        selectedModel = selected
        emit(AgentEvent.ModelsLoaded(models, selected))
    }

    private fun collectConfigValues(raw: List<Any?>, sink: (String, String) -> Unit) {
        raw.forEach { item ->
            val map = item.asMap() ?: return@forEach
            val value = map.stringAt("value")
            if (value != null) {
                sink(value, map.stringAt("name") ?: value)
                return@forEach
            }
            val nested = map.listAt("options")
            if (nested.isNotEmpty()) {
                collectConfigValues(nested, sink)
            }
        }
    }

    // ------------------------------------------------------------------
    // History replay
    // ------------------------------------------------------------------

    private var currentReplayMessageId: String? = null
    private var currentReplayRole: AgentRole? = null
    private var replayCounter = 0L

    private fun replayMessageId(role: AgentRole, update: Map<String, Any?>): String {
        update.stringAt("messageId")?.let { provided ->
            currentReplayMessageId = provided
            currentReplayRole = role
            return provided
        }
        if (currentReplayMessageId == null || currentReplayRole != role) {
            replayCounter += 1
            currentReplayMessageId = "hist:${role.name.lowercase()}:$replayCounter"
            currentReplayRole = role
        }
        return currentReplayMessageId ?: "hist:${role.name.lowercase()}:$replayCounter"
    }

    private suspend fun appendReplayBlock(
        messageId: String,
        role: AgentRole,
        placeholder: ContentBlock,
        text: String
    ) {
        val message = replayMessages.getOrPut(messageId) { ReplayMessage(role, mutableListOf()) }
        val index = message.blocks.indexOfFirst { it.id == placeholder.id }
        val existing = if (index >= 0) message.blocks[index] else null
        val updated: ContentBlock = when {
            existing is ContentBlock.Text -> existing.copy(text = existing.text + text)
            existing is ContentBlock.Reasoning -> existing.copy(text = existing.text + text)
            placeholder is ContentBlock.Text -> ContentBlock.Text(placeholder.id, text)
            placeholder is ContentBlock.Reasoning -> ContentBlock.Reasoning(placeholder.id, text)
            else -> placeholder
        }
        if (index < 0) message.blocks += updated else message.blocks[index] = updated
        emit(
            AgentEvent.MessageUpserted(
                AgentMessage(
                    id = messageId,
                    role = role,
                    blocks = message.blocks.toList()
                )
            )
        )
    }

    private fun resetTurnState() {
        turnCounter = 0
        currentTurnId = null
        currentAssistantMessageId = null
        toolStates.clear()
        replayMessages.clear()
        currentReplayMessageId = null
        currentReplayRole = null
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    private suspend fun emit(event: AgentEvent) {
        if (closed) return
        outbox.send(config?.redactEvent(event) ?: event)
    }

    /**
     * Protocol noise the user cannot act on goes to logcat instead of the chat;
     * only notices that need a decision stay on screen. `android.util.Log` is a
     * throwing stub in JVM unit tests, so logging must never break a session.
     */
    private fun logTechnical(message: String) {
        runCatching { android.util.Log.d(LOG_TAG, redact(message)) }
    }

    private suspend fun fail(rawMessage: String) {
        val message = redact(rawMessage)
        if (closed) return
        // Terminal events must reach the UI; `emit` drops everything once the
        // closed flag is set, so they are sent first. A concurrent
        // `adapter.close()` may already have closed the outbox.
        runCatching { outbox.send(AgentEvent.ConnectionLost(message)) }
        runCatching { outbox.send(AgentEvent.Closed) }
        closed = true
        failPendingRequests(IllegalStateException(message))
        outbox.close()
    }

    private suspend fun closeFromRemote() {
        if (closed) return
        runCatching { outbox.send(AgentEvent.Closed) }
        closed = true
        failPendingRequests(IllegalStateException("acp session closed"))
        outbox.close()
    }

    /**
     * Complete every in-flight request and server request. The prompt request
     * has no timeout, so without this the turn would wait forever after a
     * dropped connection.
     */
    private fun failPendingRequests(error: Throwable) {
        pending.keys.toList().forEach { id ->
            pending.remove(id)?.completeExceptionally(error)
        }
        serverRequests.clear()
    }

    private fun appendStderr(chunk: String) {
        stderrTail.append(chunk)
        if (stderrTail.length > STDERR_TAIL_LIMIT) {
            stderrTail.delete(0, stderrTail.length - STDERR_TAIL_LIMIT)
        }
    }

    private fun redact(text: String) = config?.redact(text) ?: text

    private fun AcpToolState.toBlock(): ContentBlock.ToolCall = ContentBlock.ToolCall(
        id = id,
        callId = id,
        name = name ?: "tool",
        title = title,
        status = status,
        input = input,
        output = output,
        error = error,
        streaming = status == ToolStatus.Running || status == ToolStatus.Pending
    )

    private companion object {
        const val ACP_COMMAND = "opencode acp"
        const val ACP_PROTOCOL_VERSION = 1
        const val EVENT_QUEUE_CAPACITY = 256
        const val REQUEST_TIMEOUT_MS = 30_000L
        const val STDERR_TAIL_LIMIT = 2_000
        const val METHOD_NOT_FOUND = -32601
        const val LOG_TAG = "OpenCodeAcpAdapter"
    }
}

internal fun toolStatusFromAcp(status: String?): ToolStatus? = when (status) {
    "pending" -> ToolStatus.Pending
    "in_progress" -> ToolStatus.Running
    "completed" -> ToolStatus.Completed
    "failed" -> ToolStatus.Failed
    "cancelled" -> ToolStatus.Declined
    else -> null
}

internal fun stopReasonStatus(stopReason: String): AgentTurnStatus = when (stopReason) {
    "cancelled" -> AgentTurnStatus.Cancelled
    "refusal" -> AgentTurnStatus.Failed
    else -> AgentTurnStatus.Completed
}

internal fun parseIsoInstant(value: String): Long? =
    runCatching { java.time.Instant.parse(value).toEpochMilli() }.getOrNull()

/**
 * Minimal line diff for ACP `diff` tool content: trims the shared prefix and
 * suffix and shows the changed hunk, which is enough for a readable card.
 */
internal fun simpleDiff(oldText: String?, newText: String): String {
    if (oldText == null) return newText
    if (oldText == newText) return ""
    val oldLines = oldText.lines()
    val newLines = newText.lines()
    var prefix = 0
    while (prefix < oldLines.size && prefix < newLines.size &&
        oldLines[prefix] == newLines[prefix]
    ) {
        prefix += 1
    }
    var suffix = 0
    while (suffix < oldLines.size - prefix && suffix < newLines.size - prefix &&
        oldLines[oldLines.size - 1 - suffix] == newLines[newLines.size - 1 - suffix]
    ) {
        suffix += 1
    }
    val output = StringBuilder()
    oldLines.drop(prefix).dropLast(suffix).forEach { output.appendLine("-$it") }
    newLines.drop(prefix).dropLast(suffix).forEach { output.appendLine("+$it") }
    return output.toString().trimEnd()
}
