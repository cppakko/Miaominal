package dev.akko.miaominal.data.agent

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AgentCacheCodecTest {

    private fun sampleState(): AgentSessionState {
        val model = AgentModel("openai", "gpt-5", "GPT-5")
        return AgentSessionState(
            session = AgentSessionInfo(
                cli = AgentCli.Codex,
                hostId = "host-1",
                cwd = "/home/me/repo",
                remoteSessionId = "thr_1",
                title = "Title"
            ),
            phase = AgentPhase.Ready,
            capabilities = AgentCapabilities(
                history = true,
                modelSelection = true,
                approvals = true,
                followUpQuestions = false,
                fileChanges = true
            ),
            turns = listOf(AgentTurn("t1", AgentTurnStatus.Completed, listOf("m1"))),
            messages = listOf(
                AgentMessage(
                    id = "m1",
                    role = AgentRole.Assistant,
                    blocks = listOf(
                        ContentBlock.Text("b1", "hello"),
                        ContentBlock.Reasoning("b2", "because"),
                        ContentBlock.ToolCall(
                            id = "b3",
                            callId = "call_1",
                            name = "command",
                            title = "ls",
                            status = ToolStatus.Completed,
                            input = "cwd",
                            output = "a.txt",
                            error = null
                        ),
                        ContentBlock.FileChanges(
                            id = "b4",
                            changes = listOf(FileChange("/a", "update", "@@ diff")),
                            status = ToolStatus.Completed
                        ),
                        ContentBlock.Plan(
                            id = "b5",
                            explanation = "why",
                            steps = listOf(PlanStep("one", "completed"))
                        )
                    )
                )
            ),
            interactions = listOf(
                InteractionRequest(
                    id = "req1",
                    kind = InteractionKind.Approval,
                    turnId = "t1",
                    toolId = "b3",
                    title = "Run?",
                    detail = null,
                    options = listOf(InteractionOption("accept", "Allow")),
                    state = InteractionState.Answered
                )
            ),
            models = listOf(model),
            selectedModel = model,
            usage = AgentUsage(11, 22, 0.25),
            notices = listOf(ContentBlock.Notice("n1", "notice text"))
        )
    }

    @Test
    fun round_trips_the_rendered_conversation() {
        val state = sampleState()
        val encoded = AgentCacheCodec.encode(AgentCacheCodec.Snapshot(state, "draft text"))
        val decoded = AgentCacheCodec.decode(encoded)
        assertEquals("draft text", decoded?.draft)
        val restored = decoded!!.state
        assertEquals(state.session, restored.session)
        assertEquals(state.capabilities, restored.capabilities)
        assertEquals(state.turns, restored.turns)
        assertEquals(state.messages, restored.messages)
        assertEquals(state.interactions, restored.interactions)
        assertEquals(state.models, restored.models)
        assertEquals(state.selectedModel, restored.selectedModel)
        assertEquals(state.usage, restored.usage)
        assertEquals(state.notices, restored.notices)
        // A restored session has no live connection.
        assertEquals(AgentPhase.Closed, restored.phase)
        assertTrue(restored.connectionLost)
    }

    @Test
    fun rejects_a_different_version() {
        val encoded = AgentCacheCodec.encode(AgentCacheCodec.Snapshot(sampleState(), ""))
        val tampered = encoded.replace("\"version\":1", "\"version\":99")
        assertNull(AgentCacheCodec.decode(tampered))
    }

    @Test
    fun rejects_corrupt_input_without_throwing() {
        assertNull(AgentCacheCodec.decode("not json"))
        assertNull(AgentCacheCodec.decode(""))
        val partial = AgentCacheCodec.decode("{\"version\":1}")
        assertTrue(partial?.state?.messages?.isEmpty() ?: true)
    }
}
