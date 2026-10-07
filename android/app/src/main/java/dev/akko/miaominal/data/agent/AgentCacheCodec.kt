package dev.akko.miaominal.data.agent

/**
 * Versioned cache format retained for reading and migrating older full-session files.
 *
 * The file is written atomically by [AgentSessionStore]; a decode failure or a
 * version mismatch simply drops the cache instead of breaking the feature.
 */
object AgentCacheCodec {
    const val VERSION = 1

    data class Snapshot(val state: AgentSessionState, val draft: String, val files: List<String> = emptyList())

    fun encode(snapshot: Snapshot): String = JsonCodec().stringify(
        mapOf(
            "version" to VERSION,
            "draft" to snapshot.draft,
            "files" to snapshot.files,
            "session" to encodeSession(snapshot.state.session),
            "phase" to snapshot.state.phase.name,
            "capabilities" to encodeCapabilities(snapshot.state.capabilities),
            "turns" to snapshot.state.turns.map(::encodeTurn),
            "messages" to snapshot.state.messages.map(::encodeMessage),
            "interactions" to snapshot.state.interactions.map(::encodeInteraction),
            "models" to snapshot.state.models.map(::encodeModel),
            "selectedModel" to snapshot.state.selectedModel?.let(::encodeModel),
            "reasoningOptions" to snapshot.state.reasoningOptions.map { mapOf("id" to it.id, "label" to it.label) },
            "selectedReasoning" to snapshot.state.selectedReasoning,
            "error" to snapshot.state.error,
            "connectionLost" to snapshot.state.connectionLost,
            "usage" to mapOf(
                "inputTokens" to snapshot.state.usage.inputTokens,
                "outputTokens" to snapshot.state.usage.outputTokens,
                "costUsd" to snapshot.state.usage.costUsd
            ),
            "notices" to snapshot.state.notices.map { encodeBlock(it) }
        )
    )

    fun decode(text: String): Snapshot? {
        val json = JsonCodec().parse(text) ?: return null
        if (json.longAt("version") != VERSION.toLong()) return null
        val phase = runCatching {
            AgentPhase.valueOf(json.stringAt("phase") ?: AgentPhase.Idle.name)
        }.getOrDefault(AgentPhase.Idle)
        val restoredSession = decodeSession(json.mapAt("session"))
        val state = AgentSessionState(
            session = restoredSession,
            phase = if (phase == AgentPhase.Ready || phase == AgentPhase.Connecting) {
                AgentPhase.Closed
            } else {
                phase
            },
            capabilities = decodeCapabilities(json.mapAt("capabilities")),
            turns = json.listAt("turns").mapNotNull { it.asMap()?.let(::decodeTurn) },
            messages = json.listAt("messages").mapNotNull { it.asMap()?.let(::decodeMessage) },
            interactions = json.listAt("interactions")
                .mapNotNull { it.asMap()?.let(::decodeInteraction) },
            models = json.listAt("models").mapNotNull { it.asMap()?.let(::decodeModel) },
            selectedModel = json.mapAt("selectedModel")?.let(::decodeModel),
            reasoningOptions = decodeReasoning(json.listAt("reasoningOptions")),
            selectedReasoning = json.stringAt("selectedReasoning"),
            usage = json.mapAt("usage")?.let { usage ->
                AgentUsage(
                    inputTokens = usage.longAt("inputTokens") ?: 0,
                    outputTokens = usage.longAt("outputTokens") ?: 0,
                    costUsd = usage.doubleAt("costUsd") ?: 0.0
                )
            } ?: AgentUsage(),
            notices = json.listAt("notices").mapNotNull { raw ->
                val block = raw.asMap()?.let(::decodeBlock)
                block as? ContentBlock.Notice
            },
            error = json.stringAt("error"),
            // A cached conversation never has a live SSH/CLI process after a
            // process restart, even when the last persisted phase was Ready.
            connectionLost = restoredSession != null
        )
        return Snapshot(state, json.stringAt("draft").orEmpty(),
            json.listAt("files").filterIsInstance<String>().filter(String::isNotBlank).distinct().take(8))
    }

    private fun encodeSession(session: AgentSessionInfo?): Map<String, Any?>? = session?.let {
        mapOf(
            "cli" to it.cli.id,
            "hostId" to it.hostId,
            "cwd" to it.cwd,
            "remoteSessionId" to it.remoteSessionId,
            "title" to it.title
        )
    }

    private fun decodeSession(map: Map<String, Any?>?): AgentSessionInfo? {
        val cli = map?.stringAt("cli")?.let(AgentCli::fromId) ?: return null
        return AgentSessionInfo(
            cli = cli,
            hostId = map.stringAt("hostId").orEmpty(),
            cwd = map.stringAt("cwd").orEmpty(),
            remoteSessionId = map.stringAt("remoteSessionId"),
            title = map.stringAt("title")
        )
    }

    private fun encodeCapabilities(capabilities: AgentCapabilities): Map<String, Any?> = mapOf(
        "history" to capabilities.history,
        "modelSelection" to capabilities.modelSelection,
        "approvals" to capabilities.approvals,
        "followUpQuestions" to capabilities.followUpQuestions,
        "fileChanges" to capabilities.fileChanges
    )

    private fun decodeCapabilities(map: Map<String, Any?>?): AgentCapabilities =
        AgentCapabilities(
            history = map?.booleanAt("history") ?: true,
            modelSelection = map?.booleanAt("modelSelection") ?: false,
            approvals = map?.booleanAt("approvals") ?: true,
            followUpQuestions = map?.booleanAt("followUpQuestions") ?: false,
            fileChanges = map?.booleanAt("fileChanges") ?: true
        )

    private fun encodeTurn(turn: AgentTurn): Map<String, Any?> = mapOf(
        "id" to turn.id,
        "status" to turn.status.name,
        "messageIds" to turn.messageIds,
        "error" to turn.error
    )

    private fun decodeTurn(map: Map<String, Any?>): AgentTurn = AgentTurn(
        id = map.stringAt("id").orEmpty(),
        status = runCatching {
            AgentTurnStatus.valueOf(map.stringAt("status") ?: "")
        }.getOrDefault(AgentTurnStatus.Completed),
        messageIds = map.listAt("messageIds").mapNotNull { it.asString() },
        error = map.stringAt("error")
    )

    private fun encodeMessage(message: AgentMessage): Map<String, Any?> = mapOf(
        "id" to message.id,
        "role" to message.role.name,
        "streaming" to message.streaming,
        "blocks" to message.blocks.map(::encodeBlock)
    )

    private fun decodeMessage(map: Map<String, Any?>): AgentMessage = AgentMessage(
        id = map.stringAt("id").orEmpty(),
        role = runCatching {
            AgentRole.valueOf(map.stringAt("role") ?: "")
        }.getOrDefault(AgentRole.Assistant),
        blocks = map.listAt("blocks").mapNotNull { raw ->
            raw.asMap()?.let(::decodeBlock)
        },
        streaming = false
    )

    private fun encodeBlock(block: ContentBlock): Map<String, Any?> = when (block) {
        is ContentBlock.Text -> mapOf(
            "type" to "text",
            "id" to block.id,
            "text" to block.text
        )

        is ContentBlock.Reasoning -> mapOf(
            "type" to "reasoning",
            "id" to block.id,
            "text" to block.text
        )

        is ContentBlock.ToolCall -> mapOf(
            "type" to "tool",
            "id" to block.id,
            "callId" to block.callId,
            "name" to block.name,
            "title" to block.title,
            "status" to block.status.name,
            "input" to block.input,
            "output" to block.output,
            "error" to block.error
        )

        is ContentBlock.FileChanges -> mapOf(
            "type" to "files",
            "id" to block.id,
            "status" to block.status.name,
            "changes" to block.changes.map { change ->
                mapOf(
                    "path" to change.path,
                    "kind" to change.kind,
                    "diff" to change.diff
                )
            }
        )

        is ContentBlock.Plan -> mapOf(
            "type" to "plan",
            "id" to block.id,
            "explanation" to block.explanation,
            "steps" to block.steps.map { step ->
                mapOf("step" to step.step, "status" to step.status)
            }
        )

        is ContentBlock.Notice -> mapOf(
            "type" to "notice",
            "id" to block.id,
            "text" to block.text
        )
    }

    private fun decodeBlock(map: Map<String, Any?>): ContentBlock? {
        val id = map.stringAt("id") ?: return null
        return when (map.stringAt("type")) {
            "text" -> ContentBlock.Text(id, map.stringAt("text").orEmpty())

            "reasoning" -> ContentBlock.Reasoning(id, map.stringAt("text").orEmpty())

            "tool" -> ContentBlock.ToolCall(
                id = id,
                callId = map.stringAt("callId"),
                name = map.stringAt("name") ?: "tool",
                title = map.stringAt("title"),
                status = runCatching {
                    ToolStatus.valueOf(map.stringAt("status") ?: "")
                }.getOrDefault(ToolStatus.Completed),
                input = map.stringAt("input"),
                output = map.stringAt("output"),
                error = map.stringAt("error")
            )

            "files" -> ContentBlock.FileChanges(
                id = id,
                changes = map.listAt("changes").mapNotNull { raw ->
                    val change = raw.asMap() ?: return@mapNotNull null
                    val path = change.stringAt("path") ?: return@mapNotNull null
                    FileChange(
                        path = path,
                        kind = change.stringAt("kind") ?: "update",
                        diff = change.stringAt("diff")
                    )
                },
                status = runCatching {
                    ToolStatus.valueOf(map.stringAt("status") ?: "")
                }.getOrDefault(ToolStatus.Completed)
            )

            "plan" -> ContentBlock.Plan(
                id = id,
                explanation = map.stringAt("explanation"),
                steps = map.listAt("steps").mapNotNull { raw ->
                    val step = raw.asMap() ?: return@mapNotNull null
                    val text = step.stringAt("step") ?: return@mapNotNull null
                    PlanStep(text, step.stringAt("status") ?: "pending")
                }
            )

            "notice" -> ContentBlock.Notice(id, map.stringAt("text").orEmpty())

            else -> null
        }
    }

    private fun encodeInteraction(interaction: InteractionRequest): Map<String, Any?> = mapOf(
        "id" to interaction.id,
        "kind" to interaction.kind.name,
        "turnId" to interaction.turnId,
        "toolId" to interaction.toolId,
        "title" to interaction.title,
        "detail" to interaction.detail,
        "options" to interaction.options.map { option ->
            mapOf(
                "id" to option.id,
                "label" to option.label,
                "description" to option.description
            )
        },
        "questions" to interaction.questions.map { question ->
            mapOf(
                "id" to question.id,
                "question" to question.question,
                "multiSelect" to question.multiSelect,
                "freeform" to question.freeform,
                "options" to question.options.map { option ->
                    mapOf("id" to option.id, "label" to option.label)
                }
            )
        },
        "state" to interaction.state.name
    )

    private fun decodeInteraction(map: Map<String, Any?>): InteractionRequest = InteractionRequest(
        id = map.stringAt("id").orEmpty(),
        kind = runCatching {
            InteractionKind.valueOf(map.stringAt("kind") ?: "")
        }.getOrDefault(InteractionKind.Approval),
        turnId = map.stringAt("turnId"),
        toolId = map.stringAt("toolId"),
        title = map.stringAt("title").orEmpty(),
        detail = map.stringAt("detail"),
        options = map.listAt("options").mapNotNull { raw ->
            val option = raw.asMap() ?: return@mapNotNull null
            val id = option.stringAt("id") ?: return@mapNotNull null
            InteractionOption(
                id = id,
                label = option.stringAt("label") ?: id,
                description = option.stringAt("description")
            )
        },
        questions = map.listAt("questions").mapNotNull { raw ->
            val question = raw.asMap() ?: return@mapNotNull null
            val id = question.stringAt("id") ?: return@mapNotNull null
            InteractionQuestion(
                id = id,
                question = question.stringAt("question") ?: id,
                options = question.listAt("options").mapNotNull { optionRaw ->
                    val option = optionRaw.asMap() ?: return@mapNotNull null
                    val optionId = option.stringAt("id") ?: return@mapNotNull null
                    InteractionOption(optionId, option.stringAt("label") ?: optionId)
                },
                multiSelect = question.booleanAt("multiSelect") ?: false,
                freeform = question.booleanAt("freeform") ?: false
            )
        },
        state = runCatching {
            InteractionState.valueOf(map.stringAt("state") ?: "")
        }.getOrDefault(InteractionState.Pending)
    )

    private fun encodeModel(model: AgentModel): Map<String, Any?> = mapOf(
        "providerId" to model.providerId,
        "modelId" to model.modelId,
        "name" to model.name,
        "reasoningOptions" to model.reasoningOptions.map { mapOf("id" to it.id, "label" to it.label) },
        "defaultReasoning" to model.defaultReasoning
    )

    private fun decodeModel(map: Map<String, Any?>): AgentModel = AgentModel(
        providerId = map.stringAt("providerId").orEmpty(),
        modelId = map.stringAt("modelId").orEmpty(),
        name = map.stringAt("name") ?: map.stringAt("modelId").orEmpty(),
        reasoningOptions = decodeReasoning(map.listAt("reasoningOptions")),
        defaultReasoning = map.stringAt("defaultReasoning")
    )

    private fun decodeReasoning(raw: List<Any?>): List<AgentReasoningOption> = raw.mapNotNull {
        val option = it.asMap() ?: return@mapNotNull null
        val id = option.stringAt("id") ?: return@mapNotNull null
        AgentReasoningOption(id, option.stringAt("label") ?: id)
    }
}
