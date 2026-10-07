package dev.akko.miaominal.data.agent

import dev.akko.miaominal.data.ShellType
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction
import java.nio.charset.Charset
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

/**
 * Codex adapter: speaks the `codex app-server` JSON-RPC 2.0 protocol
 * (newline-delimited JSON on stdin/stdout) over one exec channel.
 *
 * The payloads follow the app-server v2 schema (initialize, thread, turn and
 * item methods); notifications are mapped to the common model, and
 * unknown item types degrade into notice cards instead of breaking the chat.
 * The protocol is versioned by the remote CLI, so the adapter parses
 * defensively: every optional field is validated before use.
 */
class CodexAdapter(
    override val cli: AgentCli = AgentCli.Codex,
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

    override val capabilities: AgentCapabilities = AgentCapabilities(
        history = true,
        modelSelection = true,
        approvals = true,
        followUpQuestions = true,
        fileChanges = true
    )

    private val outbox = Channel<AgentEvent>(EVENT_QUEUE_CAPACITY)
    override val events: Flow<AgentEvent> = outbox.receiveAsFlow()

    private val framer = LineFramer()
    private val nextRequestId = AtomicLong(1)
    private val pending = ConcurrentHashMap<Long, CompletableDeferred<Map<String, Any?>>>()

    /** interaction id -> server request, for requests that expect an answer. */
    private val pendingInteractions = ConcurrentHashMap<String, PendingInteraction>()

    /** rpc id key -> interaction id, for `serverRequest/resolved`. */
    private val rpcInteractions = ConcurrentHashMap<String, String>()

    private var execChannelId: Long = -1
    private var threadId: String? = null
    private var currentTurnId: String? = null
    private var selectedModel: AgentModel? = null
    private var selectedReasoning: String? = null
    private var configuredModelConfirmed = config == null
    private var cwd: String = ""
    private var consumer: Job? = null
    private val stderrTail = AgentStderrTail(shellType)

    @Volatile
    private var closed = false

    private data class PendingInteraction(
        val rpcId: Any,
        val id: String,
        /** Approvals reply with `decision`, user-input requests with `answers`. */
        val kind: InteractionKind
    )
    override suspend fun start(cwd: String): AgentProbeResult {
        this.cwd = cwd
        execChannelId = channel.launch(cwd)
        consumer = scope.launch { consumeChannel() }
        val initialized = try {
            channel.awaitReady(requestTimeoutMs)
            request(
                "initialize",
                mapOf(
                    "clientInfo" to mapOf(
                        "name" to clientName,
                        "title" to "Miaominal",
                        "version" to clientVersion
                    ),
                    // Approvals and user-input requests stay on the stable
                    // surface; experimental items still degrade to notices.
                    "capabilities" to mapOf("experimentalApi" to true)
                )
            )
        } catch (error: Throwable) {
            withContext(NonCancellable) { close() }
            if (error is CancellationException) throw error
            return AgentProbeResult(false, null, redact(describe(error, stderrTail.text())))
        }
        send(mapOf("method" to "initialized", "params" to emptyMap<String, Any?>()))
        emit(
            AgentEvent.SessionOpened(
                session = AgentSessionInfo(
                    cli = cli,
                    hostId = hostId,
                    cwd = cwd,
                    remoteSessionId = null
                ),
                capabilities = capabilities
            )
        )
        emit(AgentEvent.PhaseChanged(AgentPhase.Ready))
        val models = if (config == null) loadModels() else config.profile.models.map {
            AgentModel(AgentProcessChannel.PROVIDER, it, it)
        }
        if (models.isNotEmpty()) {
            selectedModel = models.firstOrNull { it.modelId == config?.profile?.defaultModel } ?: models.firstOrNull()
            emit(AgentEvent.ModelsLoaded(models, selected = selectedModel))
            configureReasoning(selectedModel)
        }
        val version = initialized.stringAt("userAgent") ?: initialized.stringAt("version")
        return AgentProbeResult(true, version)
    }

    override suspend fun listSessions(cwd: String?): List<AgentHistoryEntry> {
        val params = mutableMapOf<String, Any?>(
            "limit" to 40,
            "sortKey" to "updated_at"
        )
        // A phone provider must not hide threads recorded under the host's old provider.
        if (config != null) params["modelProviders"] = emptyList<String>()
        cwd?.trim()?.takeIf(String::isNotBlank)?.let { params["cwd"] = it }
        val response = request("thread/list", params)
        return response.listAt("data").mapNotNull { raw ->
            val entry = raw.asMap() ?: return@mapNotNull null
            val id = entry.stringAt("id") ?: return@mapNotNull null
            val name = entry.stringAt("name")
            val preview = entry.stringAt("preview")
            AgentHistoryEntry(
                remoteId = id,
                title = name?.takeIf(String::isNotBlank)
                    ?: preview?.takeIf(String::isNotBlank)
                    ?: id,
                preview = preview,
                updatedAtMs = entry.longAt("updatedAt")?.let(::secondsToMillis),
                cwd = entry.stringAt("cwd")?.takeIf(String::isNotBlank)
            )
        }
    }

    override suspend fun createSession(cwd: String): String {
        configuredModelConfirmed = config == null
        val params = mutableMapOf<String, Any?>("cwd" to cwd)
        config?.let {
            params["modelProvider"] = AgentProcessChannel.PROVIDER
            params["model"] = selectedModel?.modelId ?: it.profile.defaultModel
        }
        val response = request("thread/start", params)
        confirmConfiguredSession(response)
        val id = response.mapAt("thread")?.stringAt("id")
            ?: response.stringAt("threadId")
            ?: error("codex did not return a thread id")
        threadId = id
        currentTurnId = null
        emit(
            AgentEvent.SessionOpened(
                session = AgentSessionInfo(
                    cli = cli,
                    hostId = hostId,
                    cwd = cwd,
                    remoteSessionId = id
                ),
                capabilities = capabilities
            )
        )
        return id
    }

    override suspend fun loadSession(remoteId: String, cwd: String?) {
        configuredModelConfirmed = config == null
        val targetCwd = cwd?.trim()?.takeIf(String::isNotBlank)
        val params = mutableMapOf<String, Any?>("threadId" to remoteId)
        config?.let {
            params["modelProvider"] = AgentProcessChannel.PROVIDER
            params["model"] = selectedModel?.modelId ?: it.profile.defaultModel
        }
        // `thread/resume` accepts an optional cwd override; without it the
        // thread's persisted directory is used.
        if (targetCwd != null) params["cwd"] = targetCwd
        val resumed = request("thread/resume", params)
        confirmConfiguredSession(resumed)
        val thread = resumed.mapAt("thread")
        threadId = thread?.stringAt("id") ?: remoteId
        val resumedCwd = thread?.stringAt("cwd")?.takeIf(String::isNotBlank)
            ?: targetCwd
            ?: this.cwd
        this.cwd = resumedCwd
        emit(
            AgentEvent.SessionOpened(
                session = AgentSessionInfo(
                    cli = cli,
                    hostId = hostId,
                    cwd = resumedCwd,
                    remoteSessionId = threadId,
                    title = thread?.stringAt("name")
                ),
                capabilities = capabilities
            )
        )
        val read = try {
            request("thread/read", mapOf("threadId" to remoteId, "includeTurns" to true))
        } catch (_: Throwable) {
            return
        }
        replayHistory(read.mapAt("thread"))
    }

    override suspend fun sendMessage(text: String) {
        check(configuredModelConfirmed) { "Codex has not confirmed the phone configuration's provider and model." }
        val activeThread = threadId ?: error("no codex thread is active")
        val params = mutableMapOf<String, Any?>(
            "threadId" to activeThread,
            "input" to listOf(mapOf("type" to "text", "text" to text))
        )
        selectedModel?.let { params["model"] = it.modelId }
        selectedReasoning?.let { params["effort"] = it }
        val response = request("turn/start", params)
        val turnId = response.mapAt("turn")?.stringAt("id")
            ?: response.stringAt("turnId")
        if (turnId != null) {
            currentTurnId = turnId
            emit(AgentEvent.TurnStarted(turnId))
        }
    }

    private fun confirmConfiguredSession(response: Map<String, Any?>) {
        val snapshot = config ?: return
        check(response.stringAt("modelProvider") == AgentProcessChannel.PROVIDER &&
            response.stringAt("model") == (selectedModel?.modelId ?: snapshot.profile.defaultModel)) {
            "Codex did not confirm the phone provider and model. Upgrade the remote CLI or check its configuration."
        }
        configuredModelConfirmed = true
    }

    override suspend fun cancelTurn() {
        val turnId = currentTurnId ?: return
        val activeThread = threadId ?: return
        try {
            request(
                "turn/interrupt",
                mapOf("threadId" to activeThread, "turnId" to turnId)
            )
        } catch (_: Throwable) {
            // The turn may have completed between the tap and the request.
        }
    }

    override suspend fun respondInteraction(requestId: String, answer: AgentInteractionAnswer) {
        val pendingInteraction = pendingInteractions[requestId] ?: return
        // The app-server expects a shape per request kind: approvals take
        // `{decision: "..."}`, user input takes `{answers: {questionId: {answers: [...]}}}`.
        val result: Any = when (pendingInteraction.kind) {
            InteractionKind.Approval -> mapOf(
                "decision" to when (answer) {
                    is AgentInteractionAnswer.Option -> answer.optionId
                    is AgentInteractionAnswer.Answers ->
                        answer.entries.firstOrNull()?.answers?.firstOrNull().orEmpty()
                }
            )

            InteractionKind.Question -> mapOf(
                "answers" to when (answer) {
                    is AgentInteractionAnswer.Answers -> answer.entries.associate { entry ->
                        entry.questionId to mapOf("answers" to entry.answers)
                    }

                    is AgentInteractionAnswer.Option -> emptyMap<String, Any?>()
                }
            )
        }
        // Send before dropping the pending entry: a failed write keeps the
        // request answerable instead of silently losing it.
        respond(pendingInteraction.rpcId, result)
        pendingInteractions.remove(requestId)
        rpcInteractions.remove(rpcKey(pendingInteraction.rpcId))
        emit(AgentEvent.InteractionResolved(requestId))
    }

    override suspend fun selectModel(model: AgentModel) {
        config?.let { require(model.providerId == AgentProcessChannel.PROVIDER && model.modelId in it.profile.models) }
        selectedModel = model
        selectedReasoning = model.defaultReasoning?.takeIf { id -> model.reasoningOptions.any { it.id == id } }
        emit(AgentEvent.ReasoningConfigured(selectedModel?.reasoningOptions.orEmpty(), selectedReasoning))
    }

    private suspend fun configureReasoning(model: AgentModel?) {
        selectedReasoning = model?.defaultReasoning?.takeIf { id -> model.reasoningOptions.any { it.id == id } }
        emit(AgentEvent.ReasoningConfigured(model?.reasoningOptions.orEmpty(), selectedReasoning))
    }

    override suspend fun selectReasoning(id: String) {
        val options = selectedModel?.reasoningOptions.orEmpty()
        require(options.any { it.id == id }) { "Unsupported reasoning effort: $id" }
        selectedReasoning = id
        emit(AgentEvent.ReasoningConfigured(options, id))
    }

    override suspend fun close() {
        if (closed) return
        closed = true
        runCatching { channel.close() }
        consumer?.cancel()
        outbox.close()
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

                is AgentChannelEventModel.Stderr -> {
                    stderrTail.append(event.data)
                }

                is AgentChannelEventModel.ExecExited -> {
                    val detail = describeExit(event.exitCode, event.signal, stderrTail.text())
                    fail("Codex app-server exited ($detail)")
                }

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

                is AgentChannelEventModel.RequestExpired -> Unit

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
            rawId != null && method != null -> handleServerRequest(rawId, method, message.mapAt("params").orEmpty())
            method != null -> handleNotification(method, message.mapAt("params").orEmpty())
        }
    }

    private suspend fun completePending(rawId: Any, message: Map<String, Any?>) {
        val id = rawId.asLong() ?: return
        val deferred = pending.remove(id) ?: return
        val error = message.mapAt("error")
        if (error != null) {
            deferred.completeExceptionally(
                IllegalStateException(error.stringAt("message") ?: "codex request failed")
            )
            return
        }
        deferred.complete(message.mapAt("result").orEmpty())
    }

    private suspend fun request(
        method: String,
        params: Map<String, Any?>
    ): Map<String, Any?> {
        val id = nextRequestId.getAndIncrement()
        val deferred = CompletableDeferred<Map<String, Any?>>()
        pending[id] = deferred
        send(mapOf("id" to id, "method" to method, "params" to params))
        val result = withTimeoutOrNull(requestTimeoutMs) { deferred.await() }
        pending.remove(id)
        if (result == null) {
            throw IllegalStateException("codex request timed out: $method")
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
    // Server requests (approvals, user input)
    // ------------------------------------------------------------------

    private fun recordPendingInteraction(
        key: String,
        rpcId: Any,
        interactionId: String,
        kind: InteractionKind
    ) {
        pendingInteractions[interactionId] = PendingInteraction(rpcId, interactionId, kind)
        rpcInteractions[key] = interactionId
    }

    private suspend fun handleServerRequest(
        rpcId: Any,
        method: String,
        params: Map<String, Any?>
    ) {
        val key = rpcKey(rpcId)
        val canonicalId = canonicalRpcId(rpcId)
        val interactionId = "codex:req:$key"
        when (method) {
            "item/commandExecution/requestApproval",
            "item/fileChange/requestApproval" -> {
                recordPendingInteraction(key, canonicalId, interactionId, InteractionKind.Approval)
                val isCommand = method.startsWith("item/commandExecution")
                val reason = params.stringAt("reason")
                val command = params.stringAt("command")
                emit(
                    AgentEvent.InteractionRequested(
                        InteractionRequest(
                            id = interactionId,
                            kind = InteractionKind.Approval,
                            turnId = params.stringAt("turnId"),
                            toolId = params.stringAt("itemId"),
                            title = if (isCommand) "Run this command?" else "Apply file changes?",
                            detail = reason ?: command,
                            options = listOf(
                                InteractionOption("accept", "Allow"),
                                InteractionOption(
                                    "acceptForSession",
                                    "Allow for this session"
                                ),
                                InteractionOption("decline", "Deny"),
                                InteractionOption("cancel", "Cancel task")
                            )
                        )
                    )
                )
            }

            "item/tool/requestUserInput" -> {
                recordPendingInteraction(key, canonicalId, interactionId, InteractionKind.Question)
                val questions = params.listAt("questions").mapNotNull { raw ->
                    val question = raw.asMap() ?: return@mapNotNull null
                    val id = question.stringAt("id") ?: return@mapNotNull null
                    InteractionQuestion(
                        id = id,
                        question = question.stringAt("question")
                            ?: question.stringAt("header")
                            ?: id,
                        // The app-server options carry only label/description;
                        // the label is what goes back in the answer.
                        options = question.listAt("options").mapNotNull { option ->
                            val optionMap = option.asMap() ?: return@mapNotNull null
                            val label = optionMap.stringAt("label") ?: return@mapNotNull null
                            InteractionOption(
                                id = label,
                                label = label,
                                description = optionMap.stringAt("description")
                            )
                        },
                        multiSelect = question.booleanAt("multiSelect") ?: false,
                        freeform = question.booleanAt("isOther") ?: false
                    )
                }
                emit(
                    AgentEvent.InteractionRequested(
                        InteractionRequest(
                            id = interactionId,
                            kind = InteractionKind.Question,
                            turnId = params.stringAt("turnId"),
                            toolId = params.stringAt("itemId"),
                            title = "Codex needs your input",
                            questions = questions
                        )
                    )
                )
            }

            else -> respondError(canonicalId, METHOD_NOT_FOUND, "unsupported request: $method")
        }
    }

    // ------------------------------------------------------------------
    // Notifications
    // ------------------------------------------------------------------

    private suspend fun handleNotification(method: String, params: Map<String, Any?>) {
        when (method) {
            "turn/started" -> {
                val turnId = params.mapAt("turn")?.stringAt("id") ?: return
                currentTurnId = turnId
                emit(AgentEvent.TurnStarted(turnId))
            }

            "turn/completed" -> {
                val turn = params.mapAt("turn") ?: return
                val turnId = turn.stringAt("id") ?: currentTurnId ?: return
                val status = turnStatus(turn.stringAt("status"))
                val error = turn.mapAt("error")?.stringAt("message")
                currentTurnId = null
                emit(AgentEvent.TurnEnded(turnId, status, error))
                if (error != null) {
                    emit(AgentEvent.ErrorRaised(error))
                }
            }

            "item/started" -> handleItem(params, completed = false)

            "item/completed" -> handleItem(params, completed = true)

            "item/agentMessage/delta" -> {
                val itemId = params.stringAt("itemId") ?: return
                val delta = params.stringAt("delta") ?: return
                emit(AgentEvent.BlockDelta(itemId, itemId, delta, DeltaKind.Text))
            }

            "item/reasoning/summaryTextDelta",
            "item/reasoning/textDelta" -> {
                val itemId = params.stringAt("itemId") ?: return
                val delta = params.stringAt("delta") ?: return
                emit(AgentEvent.BlockDelta(itemId, itemId, delta, DeltaKind.Reasoning))
            }

            "item/commandExecution/outputDelta",
            "item/fileChange/outputDelta" -> {
                val itemId = params.stringAt("itemId") ?: return
                val delta = params.stringAt("delta") ?: return
                emit(AgentEvent.ToolOutputDelta(itemId, itemId, delta))
            }

            "turn/plan/updated" -> handlePlanUpdated(params)

            "turn/diff/updated" -> handleDiffUpdated(params)

            "thread/tokenUsage/updated" -> handleUsageUpdated(params)

            "error" -> {
                val message = params.mapAt("error")?.stringAt("message")
                    ?: params.stringAt("message")
                    ?: "codex reported an error"
                emit(AgentEvent.ErrorRaised(message))
            }

            "serverRequest/resolved" -> {
                val key = params["requestId"]?.let(::rpcKey) ?: return
                val interactionId = rpcInteractions.remove(key) ?: return
                pendingInteractions.remove(interactionId)
                emit(AgentEvent.InteractionResolved(interactionId))
            }

            "thread/started",
            "thread/status/changed",
            "thread/name/updated",
            "thread/closed" -> Unit

            else -> logTechnical("notification $method: ${json.stringify(params)}")
        }
    }

    private suspend fun handleItem(params: Map<String, Any?>, completed: Boolean) {
        val item = params.mapAt("item") ?: return
        val itemId = item.stringAt("id") ?: return
        val type = item.stringAt("type") ?: return
        when (type) {
            "userMessage" -> {
                val blocks = item.listAt("content").mapIndexedNotNull { index, raw ->
                    val part = raw.asMap() ?: return@mapIndexedNotNull null
                    when (part.stringAt("type")) {
                        "text" -> ContentBlock.Text(
                            id = "$itemId:content:$index",
                            text = part.stringAt("text").orEmpty()
                        )

                        "image", "localImage" -> ContentBlock.Notice(
                            id = "$itemId:content:$index",
                            text = "Image attachment"
                        )

                        else -> null
                    }
                }
                emit(
                    AgentEvent.MessageUpserted(
                        AgentMessage(itemId, AgentRole.User, blocks)
                    )
                )
            }

            "agentMessage" -> {
                val text = item.stringAt("text").orEmpty()
                emit(
                    AgentEvent.MessageUpserted(
                        AgentMessage(
                            id = itemId,
                            role = AgentRole.Assistant,
                            blocks = listOf(
                                ContentBlock.Text(
                                    id = itemId,
                                    text = text,
                                    streaming = !completed
                                )
                            ),
                            streaming = !completed
                        )
                    )
                )
            }

            "reasoning" -> {
                val summary = item.listAt("summary")
                    .mapNotNull { it.asString() }
                    .joinToString("\n\n")
                val content = item.listAt("content")
                    .mapNotNull { it.asString() }
                    .joinToString("\n\n")
                val text = summary.ifBlank { content }
                emit(
                    AgentEvent.MessageUpserted(
                        AgentMessage(
                            id = itemId,
                            role = AgentRole.Assistant,
                            blocks = listOf(
                                ContentBlock.Reasoning(
                                    id = itemId,
                                    text = text,
                                    streaming = !completed
                                )
                            ),
                            streaming = !completed
                        )
                    )
                )
            }

            "commandExecution" -> {
                val command = item.stringAt("command").orEmpty()
                val output = item.stringAt("aggregatedOutput")
                emit(
                    AgentEvent.MessageUpserted(
                        AgentMessage(
                            id = itemId,
                            role = AgentRole.Assistant,
                            blocks = listOf(
                                ContentBlock.ToolCall(
                                    id = itemId,
                                    callId = item.stringAt("callId") ?: itemId,
                                    name = "command",
                                    title = command,
                                    status = toolStatus(item.stringAt("status"), completed),
                                    input = item.stringAt("cwd"),
                                    output = output,
                                    error = exitError(item),
                                    streaming = !completed
                                )
                            ),
                            streaming = !completed
                        )
                    )
                )
            }

            "fileChange" -> {
                val changes = item.listAt("changes").mapNotNull { raw ->
                    val change = raw.asMap() ?: return@mapNotNull null
                    val path = change.stringAt("path") ?: return@mapNotNull null
                    FileChange(
                        path = path,
                        kind = change.stringAt("kind") ?: "update",
                        diff = change.stringAt("diff")
                    )
                }
                emit(
                    AgentEvent.MessageUpserted(
                        AgentMessage(
                            id = itemId,
                            role = AgentRole.Assistant,
                            blocks = listOf(
                                ContentBlock.FileChanges(
                                    id = itemId,
                                    changes = changes,
                                    status = toolStatus(item.stringAt("status"), completed)
                                )
                            ),
                            streaming = !completed
                        )
                    )
                )
            }

            "mcpToolCall", "dynamicToolCall" -> {
                val server = item.stringAt("server")
                val tool = item.stringAt("tool").orEmpty()
                val name = if (server.isNullOrBlank()) tool else "$server.$tool"
                emit(
                    AgentEvent.MessageUpserted(
                        AgentMessage(
                            id = itemId,
                            role = AgentRole.Assistant,
                            blocks = listOf(
                                ContentBlock.ToolCall(
                                    id = itemId,
                                    callId = itemId,
                                    name = name,
                                    title = name,
                                    status = toolStatus(item.stringAt("status"), completed),
                                    input = item["arguments"]?.let(json::stringify),
                                    output = item["result"]?.let(json::stringify),
                                    error = item.mapAt("error")?.stringAt("message"),
                                    streaming = !completed
                                )
                            ),
                            streaming = !completed
                        )
                    )
                )
            }

            "webSearch" -> {
                val query = item.stringAt("query").orEmpty()
                emit(
                    AgentEvent.MessageUpserted(
                        AgentMessage(
                            id = itemId,
                            role = AgentRole.Assistant,
                            blocks = listOf(
                                ContentBlock.ToolCall(
                                    id = itemId,
                                    callId = itemId,
                                    name = "web_search",
                                    title = query,
                                    status = if (completed) ToolStatus.Completed else ToolStatus.Running,
                                    streaming = !completed
                                )
                            ),
                            streaming = !completed
                        )
                    )
                )
            }

            "plan", "enteredReviewMode", "exitedReviewMode" -> {
                val text = item.stringAt("text")
                    ?: item.stringAt("review")
                    ?: item.stringAt("plan")
                    ?: json.stringify(item)
                emit(
                    AgentEvent.MessageUpserted(
                        AgentMessage(
                            id = itemId,
                            role = AgentRole.Assistant,
                            blocks = listOf(ContentBlock.Text(itemId, text, streaming = !completed)),
                            streaming = !completed
                        )
                    )
                )
            }

            "contextCompaction" -> emit(
                AgentEvent.MessageUpserted(
                    AgentMessage(
                        id = itemId,
                        role = AgentRole.System,
                        blocks = listOf(
                            ContentBlock.Notice(itemId, "Context compacted")
                        )
                    )
                )
            )

            "imageView" -> emit(AgentEvent.MessageUpserted(
                AgentMessage(itemId, AgentRole.Assistant, listOf(ContentBlock.Notice(
                    itemId, "Image\n" + (item.stringAt("path") ?: json.stringify(item))
                )))
            ))

            else -> logTechnical("item $type: ${json.stringify(item)}")
        }
    }

    private suspend fun handlePlanUpdated(params: Map<String, Any?>) {
        val turnId = params.stringAt("turnId") ?: "turn"
        val steps = params.listAt("plan").mapNotNull { raw ->
            val step = raw.asMap() ?: return@mapNotNull null
            val text = step.stringAt("step") ?: return@mapNotNull null
            PlanStep(text, step.stringAt("status") ?: "pending")
        }
        emit(
            AgentEvent.MessageUpserted(
                AgentMessage(
                    id = "plan:$turnId",
                    role = AgentRole.Assistant,
                    blocks = listOf(
                        ContentBlock.Plan(
                            id = "plan",
                            explanation = params.stringAt("explanation"),
                            steps = steps
                        )
                    )
                )
            )
        )
    }

    private suspend fun handleDiffUpdated(params: Map<String, Any?>) {
        val turnId = params.stringAt("turnId") ?: "turn"
        val diff = params.stringAt("diff") ?: return
        emit(
            AgentEvent.MessageUpserted(
                AgentMessage(
                    id = "diff:$turnId",
                    role = AgentRole.Assistant,
                    blocks = listOf(
                        ContentBlock.FileChanges(
                            id = "diff",
                            changes = listOf(FileChange("Turn changes", "update", diff)),
                            status = ToolStatus.Completed
                        )
                    )
                )
            )
        )
    }

    private suspend fun handleUsageUpdated(params: Map<String, Any?>) {
        val usage = params.mapAt("usage")
            ?: params.mapAt("tokenUsage")
            ?: params
        val input = usage.longAt("inputTokens")
            ?: usage.longAt("input_tokens")
            ?: usage.longAt("totalInputTokens")
            ?: 0
        val output = usage.longAt("outputTokens")
            ?: usage.longAt("output_tokens")
            ?: usage.longAt("totalOutputTokens")
            ?: 0
        val cost = usage.doubleAt("costUsd") ?: usage.doubleAt("cost") ?: 0.0
        emit(AgentEvent.UsageUpdated(AgentUsage(input, output, cost)))
    }

    private suspend fun loadModels(): List<AgentModel> = try {
        val response = request("model/list", emptyMap())
        response.listAt("data").mapNotNull { raw ->
            val model = raw.asMap() ?: return@mapNotNull null
            val modelId = model.stringAt("model")
                ?: model.stringAt("id")
                ?: return@mapNotNull null
            val providerId = model.stringAt("providerId")
                ?: model.stringAt("provider")
                ?: "openai"
            AgentModel(
                providerId = providerId,
                modelId = modelId,
                name = model.stringAt("displayName") ?: modelId,
                reasoningOptions = model.listAt("supportedReasoningEfforts").mapNotNull { rawOption ->
                    val option = rawOption.asMap() ?: return@mapNotNull null
                    val id = option.stringAt("reasoningEffort") ?: return@mapNotNull null
                    AgentReasoningOption(id, id)
                },
                defaultReasoning = model.stringAt("defaultReasoningEffort")
            )
        }
    } catch (_: Throwable) {
        emptyList()
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    private suspend fun replayHistory(thread: Map<String, Any?>?) {
        val turns = thread?.listAt("turns").orEmpty()
        for (rawTurn in turns) {
            val turn = rawTurn.asMap() ?: continue
            val turnId = turn.stringAt("id") ?: continue
            emit(AgentEvent.TurnStarted(turnId))
            for (rawItem in turn.listAt("items")) {
                handleItem(
                    mapOf("item" to rawItem, "turnId" to turnId),
                    completed = true
                )
            }
            val status = turn.stringAt("status")
            if (status != null && status != "inProgress") {
                emit(AgentEvent.TurnEnded(turnId, turnStatus(status)))
            }
        }
    }

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

    private suspend fun emitNotice(title: String, detail: String) {
        emit(
            AgentEvent.NoticeRaised(
                ContentBlock.Notice(
                    id = "codex:notice:${nextRequestId.getAndIncrement()}",
                    text = if (detail.isBlank() || detail.length > NOTICE_DETAIL_LIMIT) {
                        title
                    } else {
                        "$title\n$detail"
                    }
                )
            )
        )
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
        failPendingRequests(IllegalStateException("codex session closed"))
        outbox.close()
    }

    /** Complete every in-flight request and interaction; nothing can answer them. */
    private fun failPendingRequests(error: Throwable) {
        pending.keys.toList().forEach { id ->
            pending.remove(id)?.completeExceptionally(error)
        }
        pendingInteractions.clear()
        rpcInteractions.clear()
    }

    private fun redact(text: String) = config?.redact(text) ?: text

    private companion object {
        const val CODEX_COMMAND = "codex app-server"
        const val EVENT_QUEUE_CAPACITY = 256
        const val REQUEST_TIMEOUT_MS = 30_000L
        const val NOTICE_DETAIL_LIMIT = 400
        const val METHOD_NOT_FOUND = -32601
        const val LOG_TAG = "CodexAdapter"
    }
}

internal fun secondsToMillis(value: Long): Long =
    if (value in 1..9_999_999_999L) value * 1000 else value

internal fun toolStatus(status: String?, completed: Boolean): ToolStatus = when (status) {
    "pending" -> ToolStatus.Pending
    "inProgress", "running" -> ToolStatus.Running
    "completed" -> ToolStatus.Completed
    "failed" -> ToolStatus.Failed
    "declined" -> ToolStatus.Declined
    else -> if (completed) ToolStatus.Completed else ToolStatus.Running
}

internal fun turnStatus(status: String?): AgentTurnStatus = when (status) {
    "interrupted" -> AgentTurnStatus.Cancelled
    "failed" -> AgentTurnStatus.Failed
    "completed" -> AgentTurnStatus.Completed
    else -> AgentTurnStatus.Completed
}

internal fun exitError(item: Map<String, Any?>): String? {
    val code = item.longAt("exitCode") ?: return null
    return if (code == 0L) null else "exit code $code"
}

/** Keep stderr bytes intact: SSH may split a character across multiple events. */
internal class AgentStderrTail(private val shellType: ShellType) {
    private val bytes = ByteArrayOutputStream()

    @Synchronized
    fun append(chunk: ByteArray) {
        bytes.write(chunk)
        if (bytes.size() > MAX_BYTES) {
            val tail = bytes.toByteArray().takeLast(MAX_BYTES).toByteArray()
            bytes.reset()
            bytes.write(tail)
        }
    }

    @Synchronized
    fun text(): String {
        val data = bytes.toByteArray()
        val utf8 = runCatching {
            Charsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(data)).toString()
        }.getOrNull()
        if (utf8 != null) return utf8
        // Windows shell diagnostics use the machine's legacy code page on
        // some hosts. GB18030 covers the Chinese Windows output observed here.
        if (shellType == ShellType.Cmd || shellType == ShellType.PowerShell) {
            return String(data, Charset.forName("GB18030"))
        }
        return String(data, Charsets.UTF_8)
    }

    private companion object {
        const val MAX_BYTES = 8_192
    }
}

internal fun describeExit(exitCode: Int?, signal: String?, stderrTail: CharSequence): String {
    val code = when {
        exitCode != null -> "exit code $exitCode"
        signal != null -> "signal $signal"
        else -> "no exit status"
    }
    val tail = stderrTail.toString().trim()
    return if (tail.isEmpty()) code else "$code: ${tail.takeLast(300)}"
}

internal fun describe(error: Throwable, stderrTail: CharSequence): String {
    val message = error.message?.takeIf(String::isNotBlank) ?: error.javaClass.simpleName
    val tail = stderrTail.toString().trim()
    return if (tail.isEmpty()) message else "$message: ${tail.takeLast(300)}"
}

/** Stable string key for a JSON-RPC id (Moshi parses integer ids as doubles). */
internal fun rpcKey(rawId: Any?): String = when (rawId) {
    is Number -> rawId.toLong().toString()
    else -> rawId?.toString().orEmpty()
}

/** Echo the id back in its integral form so the server can match the request. */
internal fun canonicalRpcId(rawId: Any): Any = if (rawId is Number) rawId.toLong() else rawId
