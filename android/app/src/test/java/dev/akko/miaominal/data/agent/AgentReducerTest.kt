package dev.akko.miaominal.data.agent

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AgentReducerTest {

    private fun message(id: String, text: String, role: AgentRole = AgentRole.Assistant) =
        AgentEvent.MessageUpserted(
            AgentMessage(
                id = id,
                role = role,
                blocks = listOf(ContentBlock.Text(id = id, text = text))
            )
        )

    @Test
    fun completeUpdatesReplaceInsteadOfDuplicating() {
        var state = AgentSessionState()
        state = AgentReducer.reduce(state, message("m1", "hello"))
        state = AgentReducer.reduce(state, message("m1", "hello world"))
        assertEquals(1, state.messages.size)
        assertEquals(
            "hello world",
            (state.messages.single().blocks.single() as ContentBlock.Text).text
        )
    }

    @Test
    fun deltasCreateThenAppendBlocks() {
        var state = AgentSessionState()
        state = AgentReducer.reduce(
            state,
            AgentEvent.BlockDelta("m1", "m1", "Hel", DeltaKind.Text)
        )
        state = AgentReducer.reduce(
            state,
            AgentEvent.BlockDelta("m1", "m1", "lo", DeltaKind.Text)
        )
        val block = state.messages.single().blocks.single() as ContentBlock.Text
        assertEquals("Hello", block.text)
        assertTrue(block.streaming)
    }

    @Test
    fun historyReplayIsIdempotent() {
        var state = AgentSessionState()
        state = AgentReducer.reduce(state, AgentEvent.TurnStarted("t1"))
        state = AgentReducer.reduce(state, message("m1", "one", AgentRole.User))
        state = AgentReducer.reduce(state, message("m2", "answer"))
        state = AgentReducer.reduce(
            state,
            AgentEvent.TurnEnded("t1", AgentTurnStatus.Completed)
        )
        val replayed = state
        state = AgentReducer.reduce(state, AgentEvent.TurnStarted("t1"))
        state = AgentReducer.reduce(state, message("m1", "one", AgentRole.User))
        state = AgentReducer.reduce(state, message("m2", "answer"))
        state = AgentReducer.reduce(
            state,
            AgentEvent.TurnEnded("t1", AgentTurnStatus.Completed)
        )
        assertEquals(replayed.messages, state.messages)
        assertEquals(1, state.turns.size)
    }

    @Test
    fun endingATurnClearsStreaming() {
        var state = AgentSessionState()
        state = AgentReducer.reduce(state, AgentEvent.TurnStarted("t1"))
        state = AgentReducer.reduce(
            state,
            AgentEvent.BlockDelta("m1", "m1", "partial", DeltaKind.Text)
        )
        assertTrue(state.messages.single().streaming)
        state = AgentReducer.reduce(
            state,
            AgentEvent.TurnEnded("t1", AgentTurnStatus.Completed)
        )
        assertFalse(state.messages.single().streaming)
        assertNull(state.currentTurn)
        assertEquals(AgentTurnStatus.Completed, state.turns.single().status)
    }

    @Test
    fun toolOutputDeltasAppendAndPromoteStatus() {
        var state = AgentSessionState()
        state = AgentReducer.reduce(
            state,
            AgentEvent.MessageUpserted(
                AgentMessage(
                    id = "tool1",
                    role = AgentRole.Assistant,
                    blocks = listOf(
                        ContentBlock.ToolCall(
                            id = "tool1",
                            callId = "call-1",
                            name = "command",
                            title = "ls",
                            status = ToolStatus.Pending
                        )
                    )
                )
            )
        )
        state = AgentReducer.reduce(
            state,
            AgentEvent.ToolOutputDelta("tool1", "tool1", "a\n")
        )
        state = AgentReducer.reduce(
            state,
            AgentEvent.ToolOutputDelta("tool1", "tool1", "b\n")
        )
        val tool = state.messages.single().blocks.single() as ContentBlock.ToolCall
        assertEquals("a\nb\n", tool.output)
        assertEquals(ToolStatus.Running, tool.status)
    }

    @Test
    fun interactionsUpsertResolveAndExpire() {
        var state = AgentSessionState()
        val request = InteractionRequest(
            id = "req1",
            kind = InteractionKind.Approval,
            turnId = "t1",
            toolId = "tool1",
            title = "Run?",
            options = listOf(InteractionOption("accept", "Allow"))
        )
        state = AgentReducer.reduce(state, AgentEvent.InteractionRequested(request))
        assertEquals(1, state.interactions.size)
        assertEquals("req1", state.pendingInteraction?.id)

        state = AgentReducer.reduce(state, AgentEvent.InteractionResolved("req1"))
        assertNull(state.pendingInteraction)
        assertEquals(InteractionState.Answered, state.interactions.single().state)

        // A late duplicate of an old request must not reopen an answered one.
        state = AgentReducer.reduce(state, AgentEvent.InteractionRequested(request))
        assertEquals(InteractionState.Pending, state.interactions.single().state)
    }

    @Test
    fun connectionLossTerminatesTurnsAndStreams() {
        var state = AgentSessionState(phase = AgentPhase.Ready)
        state = AgentReducer.reduce(state, AgentEvent.TurnStarted("t1"))
        state = AgentReducer.reduce(
            state,
            AgentEvent.BlockDelta("m1", "m1", "text", DeltaKind.Text)
        )
        state = AgentReducer.reduce(
            state,
            AgentEvent.ConnectionLost("ssh closed")
        )
        assertEquals(AgentPhase.Failed, state.phase)
        assertTrue(state.connectionLost)
        assertEquals(AgentTurnStatus.Failed, state.turns.single().status)
        assertFalse(state.messages.single().streaming)
    }

    @Test
    fun remoteCloseIsShownAsConnectionLossWhenSessionExists() {
        var state = AgentSessionState(
            session = AgentSessionInfo(AgentCli.Codex, "host-1", "/repo", "thread-1"),
            phase = AgentPhase.Ready
        )
        state = AgentReducer.reduce(state, AgentEvent.Closed)
        assertEquals(AgentPhase.Closed, state.phase)
        assertTrue(state.connectionLost)
    }

    @Test
    fun modelsKeepASelectionOnlyWhilePresent() {
        val first = AgentModel("openai", "a", "A")
        val second = AgentModel("openai", "b", "B")
        var state = AgentSessionState()
        state = AgentReducer.reduce(state, AgentEvent.ModelsLoaded(listOf(first, second), first))
        assertEquals(first, state.selectedModel)
        state = AgentReducer.reduce(state, AgentEvent.ModelsLoaded(listOf(second), null))
        assertEquals(second, state.selectedModel)
    }

    @Test
    fun noticesAreDeduplicatedAndBounded() {
        var state = AgentSessionState()
        state = AgentReducer.reduce(
            state,
            AgentEvent.NoticeRaised(ContentBlock.Notice("n1", "first"))
        )
        state = AgentReducer.reduce(
            state,
            AgentEvent.NoticeRaised(ContentBlock.Notice("n1", "updated"))
        )
        assertEquals(1, state.notices.size)
        assertEquals("updated", state.notices.single().text)
        repeat(80) { index ->
            state = AgentReducer.reduce(
                state,
                AgentEvent.NoticeRaised(ContentBlock.Notice("n$index", "notice $index"))
            )
        }
        assertTrue(state.notices.size <= 50)
    }
}
