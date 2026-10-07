package dev.akko.miaominal.data.agent

/**
 * Pure state fold for agent events.
 *
 * Rules shared by every adapter:
 * - a complete update replaces the entity with the same stable id;
 * - a delta appends to an existing block, or creates the block when the
 *   adapter streams before the first complete update;
 * - history replay never duplicates messages or blocks;
 * - ending a turn clears every streaming flag.
 */
object AgentReducer {

    fun reduce(state: AgentSessionState, event: AgentEvent): AgentSessionState = when (event) {
        is AgentEvent.ReasoningConfigured -> state.copy(
            reasoningOptions = event.options,
            selectedReasoning = event.selected?.takeIf { id -> event.options.any { it.id == id } }
        )
        is AgentEvent.PhaseChanged -> state.copy(
            phase = event.phase,
            error = event.error ?: if (event.phase == AgentPhase.Ready) null else state.error,
            connectionLost = when (event.phase) {
                AgentPhase.Ready -> false
                AgentPhase.Failed, AgentPhase.Closed -> state.session != null
                else -> state.connectionLost
            }
        )

        is AgentEvent.SessionOpened -> state.copy(
            session = event.session,
            capabilities = event.capabilities,
            error = null
        )

        is AgentEvent.ModelsLoaded -> state.copy(
            models = event.models,
            selectedModel = event.selected
                ?: state.selectedModel?.takeIf { current -> event.models.any { it.key == current.key } }
                ?: event.models.firstOrNull()
        )

        is AgentEvent.HistoryLoaded -> state

        is AgentEvent.TurnStarted -> upsertTurn(
            state,
            AgentTurn(id = event.turnId, status = AgentTurnStatus.Running)
        )

        is AgentEvent.TurnEnded -> endTurn(state, event)

        is AgentEvent.MessageUpserted -> upsertMessage(state, event.message)

        is AgentEvent.MessageRemoved -> state.copy(
            messages = state.messages.filterNot { it.id == event.messageId },
            turns = state.turns.map { turn ->
                if (turn.messageIds.contains(event.messageId)) {
                    turn.copy(messageIds = turn.messageIds - event.messageId)
                } else {
                    turn
                }
            }
        )

        is AgentEvent.BlockUpserted -> upsertBlock(state, event.messageId, event.block)

        is AgentEvent.BlockDelta -> appendDelta(state, event)

        is AgentEvent.BlockRemoved -> removeBlock(state, event.messageId, event.blockId)

        is AgentEvent.ToolOutputDelta -> appendToolOutput(state, event)

        is AgentEvent.InteractionRequested -> upsertInteraction(state, event.request)

        is AgentEvent.InteractionResolved -> state.copy(
            interactions = state.interactions.map { request ->
                if (request.id == event.requestId &&
                    request.state == InteractionState.Pending
                ) {
                    request.copy(state = InteractionState.Answered)
                } else {
                    request
                }
            }
        )

        is AgentEvent.UsageUpdated -> state.copy(usage = event.usage)

        is AgentEvent.NoticeRaised -> state.copy(
            notices = (state.notices.filterNot { it.id == event.notice.id } + event.notice)
                .takeLast(NOTICE_LIMIT)
        )

        is AgentEvent.ErrorRaised -> state.copy(error = event.message)

        is AgentEvent.ConnectionLost -> state.copy(
            connectionLost = true,
            error = event.message,
            phase = AgentPhase.Failed,
            turns = state.turns.map { turn ->
                if (!turn.status.isTerminal) {
                    turn.copy(status = AgentTurnStatus.Failed, error = event.message)
                } else {
                    turn
                }
            },
            messages = state.messages.map { it.copy(streaming = false) }
        )

        AgentEvent.Closed -> state.copy(
            phase = AgentPhase.Closed,
            connectionLost = state.session != null,
            turns = state.turns.map { turn ->
                if (!turn.status.isTerminal) {
                    turn.copy(status = AgentTurnStatus.Cancelled)
                } else {
                    turn
                }
            },
            messages = state.messages.map { it.copy(streaming = false) }
        )
    }

    private fun upsertTurn(state: AgentSessionState, turn: AgentTurn): AgentSessionState {
        val existing = state.turns.indexOfFirst { it.id == turn.id }
        return if (existing < 0) {
            state.copy(turns = state.turns + turn)
        } else {
            state.copy(turns = state.turns.toMutableList().also { it[existing] = turn })
        }
    }

    private fun endTurn(state: AgentSessionState, event: AgentEvent.TurnEnded): AgentSessionState {
        val turns = state.turns.map { turn ->
            if (turn.id == event.turnId) {
                turn.copy(status = event.status, error = event.error)
            } else {
                turn
            }
        }
        return state.copy(
            turns = turns,
            messages = state.messages.map { it.copy(streaming = false) },
            error = event.error ?: if (event.status == AgentTurnStatus.Failed) state.error else null
        )
    }

    private fun upsertMessage(state: AgentSessionState, message: AgentMessage): AgentSessionState {
        val existing = state.messages.indexOfFirst { it.id == message.id }
        val messages = if (existing < 0) {
            state.messages + message
        } else {
            state.messages.toMutableList().also { it[existing] = message }
        }
        // Associate a new message with the active turn so ending the turn can
        // clear its streaming flag even when the adapter does not enumerate ids.
        val activeTurn = state.currentTurn
        val attach = existing < 0 && activeTurn != null &&
            !activeTurn.messageIds.contains(message.id)
        val turns = if (attach && activeTurn != null) {
            state.turns.map { turn ->
                if (turn.id == activeTurn.id) {
                    turn.copy(messageIds = turn.messageIds + message.id)
                } else {
                    turn
                }
            }
        } else {
            state.turns
        }
        return state.copy(messages = messages, turns = turns)
    }

    private fun upsertBlock(
        state: AgentSessionState,
        messageId: String,
        block: ContentBlock
    ): AgentSessionState {
        val message = state.message(messageId)
        if (message == null) {
            // Stream-first protocols may emit a block delta before the message
            // shell; create it as an assistant message to keep ordering stable.
            return upsertMessage(
                state,
                AgentMessage(id = messageId, role = AgentRole.Assistant, blocks = listOf(block))
            )
        }
        val index = message.blocks.indexOfFirst { it.id == block.id }
        val blocks = if (index < 0) {
            message.blocks + block
        } else {
            message.blocks.toMutableList().also { it[index] = block }
        }
        return upsertMessage(state, message.copy(blocks = blocks))
    }

    private fun appendDelta(
        state: AgentSessionState,
        event: AgentEvent.BlockDelta
    ): AgentSessionState {
        val message = state.message(event.messageId)
        val block = message?.blocks?.firstOrNull { it.id == event.blockId }
        val updated: ContentBlock = when (block) {
            is ContentBlock.Text -> block.copy(
                text = block.text + event.text,
                streaming = true
            )

            is ContentBlock.Reasoning -> block.copy(
                text = block.text + event.text,
                streaming = true
            )

            null -> when (event.kind) {
                DeltaKind.Reasoning -> ContentBlock.Reasoning(
                    id = event.blockId,
                    text = event.text,
                    streaming = true
                )

                DeltaKind.Text -> ContentBlock.Text(
                    id = event.blockId,
                    text = event.text,
                    streaming = true
                )
            }

            else -> block
        }
        if (message == null) {
            return upsertMessage(
                state,
                AgentMessage(
                    id = event.messageId,
                    role = AgentRole.Assistant,
                    blocks = listOf(updated),
                    streaming = true
                )
            )
        }
        val index = message.blocks.indexOfFirst { it.id == event.blockId }
        val blocks = if (index < 0) {
            message.blocks + updated
        } else {
            message.blocks.toMutableList().also { it[index] = updated }
        }
        return upsertMessage(state, message.copy(blocks = blocks, streaming = true))
    }

    private fun removeBlock(
        state: AgentSessionState,
        messageId: String,
        blockId: String
    ): AgentSessionState {
        val message = state.message(messageId) ?: return state
        return upsertMessage(
            state,
            message.copy(blocks = message.blocks.filterNot { it.id == blockId })
        )
    }

    private fun appendToolOutput(
        state: AgentSessionState,
        event: AgentEvent.ToolOutputDelta
    ): AgentSessionState {
        val message = state.message(event.messageId)
        val block = message?.blocks?.firstOrNull { it.id == event.blockId }
        if (message == null || block !is ContentBlock.ToolCall) {
            return state
        }
        val updated = block.copy(
            output = (block.output ?: "") + event.text,
            status = if (block.status == ToolStatus.Pending) ToolStatus.Running else block.status,
            streaming = true
        )
        return upsertMessage(
            state,
            message.copy(
                blocks = message.blocks.map { if (it.id == updated.id) updated else it },
                streaming = true
            )
        )
    }

    private fun upsertInteraction(
        state: AgentSessionState,
        request: InteractionRequest
    ): AgentSessionState {
        val index = state.interactions.indexOfFirst { it.id == request.id }
        val interactions = if (index < 0) {
            state.interactions + request
        } else {
            state.interactions.toMutableList().also { it[index] = request }
        }
        return state.copy(interactions = interactions)
    }

    private const val NOTICE_LIMIT = 50
}
