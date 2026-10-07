package dev.akko.miaominal.data.agent

import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.receiveAsFlow

/** In-memory channel used by adapter unit tests; never touches the native lib. */
class FakeAgentChannel : AgentChannel {
    override val connectionId: Long = 1

    private val events = Channel<AgentChannelEventModel>(Channel.UNLIMITED)
    private val json = JsonCodec()

    val execCommands = mutableListOf<String>()
    val rawWrites = mutableListOf<ByteArray>()
    val closedChannels = mutableListOf<Long>()
    var acknowledgeBootstrap = false
    var onRawWrite: ((ByteArray) -> Unit)? = null
    val writtenMessages = mutableListOf<Map<String, Any?>>()

    /** Called for every JSON-RPC message written to stdin. */
    var onLine: ((Map<String, Any?>) -> Unit)? = null

    /** When true every stdin write fails, simulating a dead channel. */
    var failWrites: Boolean = false

    override fun events(): Flow<AgentChannelEventModel> = events.receiveAsFlow()

    override suspend fun openExec(
        command: String,
        environment: List<Pair<String, String>>
    ): Long {
        execCommands += command
        events.send(AgentChannelEventModel.ExecOpened(1))
        return 1
    }

    override suspend fun writeStdin(channelId: Long, data: ByteArray) {
        if (failWrites) throw IllegalStateException("stdin write failed")
        rawWrites += data.copyOf()
        onRawWrite?.invoke(data)
        if (acknowledgeBootstrap && rawWrites.size == 1) {
            val command = execCommands.last()
            val script = if ("-EncodedCommand " in command) String(
                java.util.Base64.getDecoder().decode(command.substringAfter("-EncodedCommand ")), Charsets.UTF_16LE
            ) else command
            val marker = Regex("MIAOMINAL_READY_[a-fA-F0-9]+").find(script)!!.value
            emitStdout(marker.take(10))
            emitStdout(marker.drop(10) + "\n")
            return
        }
        val text = String(data, Charsets.UTF_8)
        text.lineSequence().filter(String::isNotBlank).forEach { line ->
            val parsed = json.parse(line) ?: return@forEach
            writtenMessages += parsed
            onLine?.invoke(parsed)
        }
    }

    override suspend fun signal(channelId: Long, signal: String) = Unit

    override suspend fun closeChannel(channelId: Long) { closedChannels += channelId }

    override suspend fun openTunnel(remoteHost: String, remotePort: Int): Long {
        events.send(AgentChannelEventModel.TunnelReady(1, 12345))
        return 1
    }

    override suspend fun closeTunnel(tunnelId: Long) = Unit

    override fun respondHostKey(requestId: Long, decision: AgentHostKeyDecisionModel) = Unit

    override fun respondKeyboardInteractive(requestId: Long, answers: List<String>) = Unit

    override suspend fun close() {
        events.close()
    }

    fun emit(event: AgentChannelEventModel) {
        events.trySend(event)
    }

    fun emitStdout(text: String) {
        events.trySend(AgentChannelEventModel.Stdout(1, text.toByteArray(Charsets.UTF_8)))
    }

    fun emitStderr(text: String) {
        events.trySend(AgentChannelEventModel.Stderr(1, text.toByteArray(Charsets.UTF_8)))
    }

    fun emitExit(code: Int) {
        events.trySend(AgentChannelEventModel.ExecExited(1, code, null))
    }
}

/** Minimal scripted `codex app-server` for unit tests. */
class FakeCodexServer(private val channel: FakeAgentChannel) {
    private val json = JsonCodec()
    val requests = mutableListOf<Map<String, Any?>>()
    var confirmConfiguredProvider = true

    init {
        channel.onLine = ::handle
    }

    private fun handle(message: Map<String, Any?>) {
        requests += message
        if (message["method"] == "initialized") return
        val id = (message["id"] as? Number)?.toLong() ?: return
        when (message["method"]) {
            "initialize" -> respond(id, mapOf("userAgent" to "codex-test/1"))
            "model/list" -> respond(
                id,
                mapOf(
                    "data" to listOf(
                        mapOf("id" to "gpt-5", "displayName" to "GPT-5",
                            "defaultReasoningEffort" to "medium",
                            "supportedReasoningEfforts" to listOf(
                                mapOf("reasoningEffort" to "medium", "description" to "Balanced"),
                                mapOf("reasoningEffort" to "high", "description" to "Thorough")
                            )),
                        mapOf("id" to "gpt-5-mini", "displayName" to "GPT-5 mini")
                    )
                )
            )

            "thread/start" -> respond(id, mapOf("thread" to mapOf("id" to "thr_1")) + configuredResponse(message))
            "thread/resume" -> respond(
                id,
                mapOf(
                    "thread" to mapOf(
                        "id" to "thr_old",
                        "name" to "Old thread",
                        "cwd" to (message.mapAt("params")?.stringAt("cwd") ?: "/repo")
                    )
                ) + configuredResponse(message)
            )

            "thread/read" -> respond(id, threadReadResponse)
            "thread/list" -> respond(
                id,
                mapOf(
                    "data" to listOf(
                        mapOf(
                            "id" to "thr_old",
                            "name" to "Old thread",
                            "preview" to "preview text",
                            "cwd" to "/repo",
                            "updatedAt" to 1_730_831_111L
                        )
                    )
                )
            )

            "turn/start" -> respond(id, mapOf("turn" to mapOf("id" to "turn_1")))
            "turn/interrupt" -> respond(id, emptyMap<String, Any?>())
            else -> respond(id, emptyMap<String, Any?>())
        }
    }

    private fun configuredResponse(message: Map<String, Any?>): Map<String, Any?> =
        if (!confirmConfiguredProvider) emptyMap() else mapOf(
            "modelProvider" to message.mapAt("params")?.stringAt("modelProvider"),
            "model" to message.mapAt("params")?.stringAt("model")
        )

    var threadReadResponse: Map<String, Any?> = mapOf(
        "thread" to mapOf(
            "id" to "thr_old",
            "name" to "Old thread",
            "turns" to listOf(
                mapOf(
                    "id" to "turn_old",
                    "status" to "completed",
                    "items" to listOf(
                        mapOf(
                            "id" to "umsg_1",
                            "type" to "userMessage",
                            "content" to listOf(mapOf("type" to "text", "text" to "hi"))
                        ),
                        mapOf(
                            "id" to "amsg_1",
                            "type" to "agentMessage",
                            "text" to "hello from history"
                        )
                    )
                )
            )
        )
    )

    fun respond(id: Long, result: Any?) {
        channel.emitStdout(json.stringify(mapOf("id" to id, "result" to result)) + "\n")
    }

    fun notify(method: String, params: Any?) {
        channel.emitStdout(json.stringify(mapOf("method" to method, "params" to params)) + "\n")
    }

    fun serverRequest(id: Long, method: String, params: Any?) {
        channel.emitStdout(
            json.stringify(mapOf("id" to id, "method" to method, "params" to params)) + "\n"
        )
    }

    fun responseTo(id: Long): Map<String, Any?>? =
        channel.writtenMessages.lastOrNull { (it["id"] as? Number)?.toLong() == id }
}

/** Minimal scripted `opencode acp` agent for unit tests. */
class FakeAcpServer(
    private val channel: FakeAgentChannel,
    private val listSupported: Boolean = true,
    private val loadSupported: Boolean = true,
    private val reasoningSupported: Boolean = false
) {
    private val json = JsonCodec()
    val requests = mutableListOf<Map<String, Any?>>()

    private var pendingPromptId: Long? = null
    private var effort = "medium"

    init {
        channel.onLine = ::handle
    }

    private fun handle(message: Map<String, Any?>) {
        requests += message
        val method = message["method"] as? String ?: return
        val id = (message["id"] as? Number)?.toLong()
        if (id == null) {
            if (method == "session/cancel") {
                pendingPromptId?.let { respond(it, mapOf("stopReason" to "cancelled")) }
                pendingPromptId = null
            }
            return
        }
        when (method) {
            "initialize" -> respond(
                id,
                mapOf(
                    "protocolVersion" to 1,
                    "agentCapabilities" to buildMap<String, Any?> {
                        put("loadSession", loadSupported)
                        put(
                            "sessionCapabilities",
                            buildMap { if (listSupported) put("list", emptyMap<String, Any>()) }
                        )
                    },
                    "agentInfo" to mapOf("name" to "opencode", "version" to "acp-test/1")
                )
            )

            "session/new" -> respond(
                id,
                mapOf("sessionId" to "ses_1", "configOptions" to configOptions())
            )

            "session/load" -> {
                emitReplay()
                respond(
                    id,
                    mapOf("sessionId" to "ses_old", "configOptions" to configOptions())
                )
            }

            "session/list" -> respond(
                id,
                mapOf(
                    "sessions" to listOf(
                        mapOf(
                            "sessionId" to "ses_old",
                            "cwd" to "/repo",
                            "title" to "旧会话",
                            "updatedAt" to "2026-09-19T04:00:00Z"
                        )
                    )
                )
            )

            "session/set_config_option" -> {
                val value = message.mapAt("params")?.stringAt("value") ?: "m1"
                if (message.mapAt("params")?.stringAt("configId") == "thinking") effort = value
                respond(id, mapOf("configOptions" to configOptions(current = if (value in listOf("m1", "m2")) value else "m1")))
            }

            "session/prompt" -> pendingPromptId = id
            else -> respond(id, emptyMap<String, Any?>())
        }
    }

    private fun emitReplay() {
        update(
            mapOf(
                "sessionUpdate" to "user_message_chunk",
                "messageId" to "msg_u1",
                "content" to mapOf("type" to "text", "text" to "hi")
            )
        )
        update(
            mapOf(
                "sessionUpdate" to "agent_message_chunk",
                "messageId" to "msg_a1",
                "content" to mapOf("type" to "text", "text" to "The capital ")
            )
        )
        update(
            mapOf(
                "sessionUpdate" to "agent_message_chunk",
                "messageId" to "msg_a1",
                "content" to mapOf("type" to "text", "text" to "is Paris.")
            )
        )
    }

    fun completePrompt(stopReason: String = "end_turn") {
        pendingPromptId?.let { respond(it, mapOf("stopReason" to stopReason)) }
        pendingPromptId = null
    }

    fun update(update: Map<String, Any?>) {
        channel.emitStdout(
            json.stringify(
                mapOf(
                    "method" to "session/update",
                    "params" to mapOf("sessionId" to "ses_1", "update" to update)
                )
            ) + "\n"
        )
    }

    fun serverRequest(id: Long, method: String, params: Map<String, Any?>) {
        channel.emitStdout(
            json.stringify(mapOf("id" to id, "method" to method, "params" to params)) + "\n"
        )
    }

    fun respond(id: Long, result: Any?) {
        channel.emitStdout(json.stringify(mapOf("id" to id, "result" to result)) + "\n")
    }

    fun responseTo(id: Long): Map<String, Any?>? =
        channel.writtenMessages.lastOrNull { (it["id"] as? Number)?.toLong() == id }

    private fun configOptions(current: String = "m1"): List<Map<String, Any?>> = listOf(
        mapOf(
            "id" to "model",
            "name" to "Model",
            "category" to "model",
            "type" to "select",
            "currentValue" to current,
            "options" to listOf(
                mapOf("value" to "m1", "name" to "Claude"),
                mapOf(
                    "group" to "other",
                    "name" to "Other",
                    "options" to listOf(mapOf("value" to "m2", "name" to "GPT"))
                )
            )
        )
    ) + if (reasoningSupported) listOf(mapOf(
        "id" to "thinking", "category" to "thought_level", "type" to "select",
        "currentValue" to effort, "options" to listOf(
            mapOf("value" to "medium", "name" to "Medium"),
            mapOf("value" to "high", "name" to "High")
        )
    )) else emptyList()
}
