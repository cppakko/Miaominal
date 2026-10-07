package dev.akko.miaominal.data.agent

import dev.akko.miaominal.data.ShellType
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

class OpenCodeAcpAdapterTest {

    @Test
    fun reasoningUsesAdvertisedConfigIdAndConfirmedValue() = runTest {
        val channel = FakeAgentChannel()
        val server = FakeAcpServer(channel, reasoningSupported = true)
        val adapter = OpenCodeAcpAdapter(hostId = "h", channel = channel, scope = adapterScope())
        val events = mutableListOf<AgentEvent>()
        collectAgentEvents(adapter, events)
        adapter.start("/repo")
        adapter.createSession("/repo")
        assertEquals("medium", fold(events).selectedReasoning)
        adapter.selectReasoning("high")
        val params = server.requests.last { it.stringAt("method") == "session/set_config_option" }.mapAt("params")!!
        assertEquals("thinking", params.stringAt("configId"))
        assertEquals("high", params.stringAt("value"))
        assertEquals("high", fold(events).selectedReasoning)
    }

    private fun fold(events: List<AgentEvent>): AgentSessionState =
        events.fold(AgentSessionState()) { state, event -> AgentReducer.reduce(state, event) }

    @Test
    fun startsOnCmdWithAWindowsWorkingDirectory() = runTest {
        val channel = FakeAgentChannel()
        FakeAcpServer(channel)
        val adapter = OpenCodeAcpAdapter(
            hostId = "windows-host",
            channel = channel,
            scope = adapterScope(),
            shellType = ShellType.Cmd
        )

        assertTrue(adapter.start("C:\\Work & Tools").available)
        assertEquals(
            "cd /d \"C:\\Work & Tools\" && opencode acp",
            channel.execCommands.single()
        )
    }

    @Test
    fun initializes_creates_a_session_and_streams_a_turn() = runTest {
        val channel = FakeAgentChannel()
        val server = FakeAcpServer(channel)
        val adapter = OpenCodeAcpAdapter(
            hostId = "host-1",
            channel = channel,
            scope = adapterScope()
        )
        val events = mutableListOf<AgentEvent>()
        collectAgentEvents(adapter, events)

        val probe = adapter.start("/repo/my project")
        assertTrue(probe.available)
        assertTrue(channel.execCommands.single().contains("cd '/repo/my project'"))
        assertTrue(channel.execCommands.single().contains("opencode acp"))
        assertTrue(adapter.capabilities.history)

        adapter.createSession("/repo/my project")
        advanceUntilIdle()
        val afterCreate = fold(events)
        assertEquals("ses_1", afterCreate.session?.remoteSessionId)
        assertEquals(listOf("m1", "m2"), afterCreate.models.map { it.modelId })
        assertEquals("m1", afterCreate.selectedModel?.modelId)

        adapter.sendMessage("hi")
        server.update(
            mapOf(
                "sessionUpdate" to "agent_message_chunk",
                "content" to mapOf("type" to "text", "text" to "Hel")
            )
        )
        server.update(
            mapOf(
                "sessionUpdate" to "agent_message_chunk",
                "content" to mapOf("type" to "text", "text" to "lo")
            )
        )
        server.update(
            mapOf(
                "sessionUpdate" to "tool_call",
                "toolCallId" to "c1",
                "title" to "Read file",
                "kind" to "read",
                "status" to "pending"
            )
        )
        server.update(
            mapOf(
                "sessionUpdate" to "tool_call_update",
                "toolCallId" to "c1",
                "status" to "completed",
                "content" to listOf(
                    mapOf(
                        "type" to "content",
                        "content" to mapOf("type" to "text", "text" to "done")
                    )
                )
            )
        )
        server.completePrompt("end_turn")
        advanceUntilIdle()

        val state = fold(events)
        val userMessage = state.messages.first { it.role == AgentRole.User }
        assertEquals("hi", (userMessage.blocks.single() as ContentBlock.Text).text)
        val assistantText = state.messages
            .flatMap { it.blocks }
            .filterIsInstance<ContentBlock.Text>()
            .first { it.text == "Hello" }
        assertEquals("Hello", assistantText.text)
        val tool = state.messages.first { it.id == "c1" }
            .blocks.filterIsInstance<ContentBlock.ToolCall>().single()
        assertEquals(ToolStatus.Completed, tool.status)
        assertEquals("done", tool.output)
        assertEquals(AgentTurnStatus.Completed, state.turns.last().status)
        assertFalse(state.isBusy)
    }

    @Test
    fun permission_requests_round_trip_with_acp_option_ids() = runTest {
        val channel = FakeAgentChannel()
        val server = FakeAcpServer(channel)
        val adapter = OpenCodeAcpAdapter(hostId = "h", channel = channel, scope = adapterScope())
        val events = mutableListOf<AgentEvent>()
        collectAgentEvents(adapter, events)
        adapter.start("/repo")
        adapter.createSession("/repo")
        server.serverRequest(
            77,
            "session/request_permission",
            mapOf(
                "sessionId" to "ses_1",
                "toolCall" to mapOf("toolCallId" to "c1", "title" to "Run tests"),
                "options" to listOf(
                    mapOf(
                        "optionId" to "allow-once",
                        "name" to "Allow once",
                        "kind" to "allow_once"
                    ),
                    mapOf(
                        "optionId" to "reject-once",
                        "name" to "Reject",
                        "kind" to "reject_once"
                    )
                )
            )
        )
        advanceUntilIdle()

        val request = fold(events).pendingInteraction
        assertNotNull(request)
        assertEquals("opencode:request:77", request!!.id)
        assertEquals(InteractionKind.Approval, request.kind)
        assertEquals(listOf("allow-once", "reject-once"), request.options.map { it.id })

        adapter.respondInteraction(request.id, AgentInteractionAnswer.Option("allow-once"))
        advanceUntilIdle()
        val response = server.responseTo(77)
        assertEquals(
            mapOf("outcome" to mapOf("outcome" to "selected", "optionId" to "allow-once")),
            response?.get("result")
        )
        assertFalse(fold(events).pendingInteraction != null)
    }

    @Test
    fun session_load_replays_history_without_duplicates() = runTest {
        val channel = FakeAgentChannel()
        FakeAcpServer(channel)
        val adapter = OpenCodeAcpAdapter(hostId = "h", channel = channel, scope = adapterScope())
        val events = mutableListOf<AgentEvent>()
        collectAgentEvents(adapter, events)
        adapter.start("/repo")
        adapter.loadSession("ses_old")
        advanceUntilIdle()

        val state = fold(events)
        assertEquals("ses_old", state.session?.remoteSessionId)
        val user = state.messages.single { it.role == AgentRole.User }
        assertEquals("hi", (user.blocks.single() as ContentBlock.Text).text)
        val assistant = state.messages.single { it.role == AgentRole.Assistant }
        assertEquals(
            "The capital is Paris.",
            (assistant.blocks.single() as ContentBlock.Text).text
        )
    }

    @Test
    fun list_sessions_is_only_called_when_advertised() = runTest {
        val supportedChannel = FakeAgentChannel()
        FakeAcpServer(supportedChannel, listSupported = true)
        val supportedAdapter = OpenCodeAcpAdapter(
            hostId = "h",
            channel = supportedChannel,
            scope = adapterScope()
        )
        collectAgentEvents(supportedAdapter, mutableListOf())
        supportedAdapter.start("/repo")
        assertTrue(supportedAdapter.capabilities.history)
        val entries = supportedAdapter.listSessions()
        assertEquals(1, entries.size)
        assertEquals("ses_old", entries.single().remoteId)
        assertEquals("旧会话", entries.single().title)
        assertEquals("/repo", entries.single().cwd)

        // Without a workspace the ACP agent lists every project, so no filter
        // is sent; a chosen workspace is still forwarded as a filter.
        val global = supportedChannel.writtenMessages.last { it["method"] == "session/list" }
        assertFalse(global.mapAt("params")!!.containsKey("cwd"))
        supportedAdapter.listSessions("/repo")
        val filtered = supportedChannel.writtenMessages.last { it["method"] == "session/list" }
        assertEquals("/repo", filtered.mapAt("params")?.stringAt("cwd"))

        val plainChannel = FakeAgentChannel()
        FakeAcpServer(plainChannel, listSupported = false)
        val plainAdapter = OpenCodeAcpAdapter(
            hostId = "h",
            channel = plainChannel,
            scope = adapterScope()
        )
        collectAgentEvents(plainAdapter, mutableListOf())
        plainAdapter.start("/repo")
        assertFalse(plainAdapter.capabilities.history)
        assertTrue(plainAdapter.listSessions().isEmpty())
        assertFalse(plainChannel.writtenMessages.any { it["method"] == "session/list" })
    }

    @Test
    fun load_session_targets_the_conversation_workspace() = runTest {
        val channel = FakeAgentChannel()
        FakeAcpServer(channel)
        val adapter = OpenCodeAcpAdapter(hostId = "h", channel = channel, scope = adapterScope())
        val events = mutableListOf<AgentEvent>()
        collectAgentEvents(adapter, events)

        // A blank directory boots ACP in the SSH login directory: no cd.
        adapter.start("")
        assertEquals("opencode acp", channel.execCommands.single())

        adapter.loadSession("ses_old", "/srv/other")
        advanceUntilIdle()

        val load = channel.writtenMessages.last { it["method"] == "session/load" }
        assertEquals("/srv/other", load.mapAt("params")?.stringAt("cwd"))
        val state = fold(events)
        assertEquals("/srv/other", state.session?.cwd)
        assertEquals("ses_old", state.session?.remoteSessionId)
    }

    @Test
    fun cancel_maps_the_cancelled_stop_reason() = runTest {
        val channel = FakeAgentChannel()
        FakeAcpServer(channel)
        val adapter = OpenCodeAcpAdapter(hostId = "h", channel = channel, scope = adapterScope())
        val events = mutableListOf<AgentEvent>()
        collectAgentEvents(adapter, events)
        adapter.start("/repo")
        adapter.createSession("/repo")
        adapter.sendMessage("long task")
        advanceUntilIdle()

        adapter.cancelTurn()
        advanceUntilIdle()

        val cancel = channel.writtenMessages.last { it["method"] == "session/cancel" }
        assertEquals("ses_1", cancel.mapAt("params")?.stringAt("sessionId"))
        val state = fold(events)
        assertEquals(AgentTurnStatus.Cancelled, state.turns.last().status)
    }

    @Test
    fun model_selection_sends_set_config_option() = runTest {
        val channel = FakeAgentChannel()
        FakeAcpServer(channel)
        val adapter = OpenCodeAcpAdapter(hostId = "h", channel = channel, scope = adapterScope())
        val events = mutableListOf<AgentEvent>()
        collectAgentEvents(adapter, events)
        adapter.start("/repo")
        adapter.createSession("/repo")
        adapter.selectModel(AgentModel("opencode", "m2", "GPT"))
        advanceUntilIdle()

        val setOption = channel.writtenMessages.last {
            it["method"] == "session/set_config_option"
        }
        assertEquals("model", setOption.mapAt("params")?.stringAt("configId"))
        assertEquals("m2", setOption.mapAt("params")?.stringAt("value"))
        assertEquals("m2", fold(events).selectedModel?.modelId)
        assertEquals("GPT", fold(events).selectedModel?.name)
    }

    @Test
    fun unknown_updates_do_not_reach_the_chat() = runTest {
        val channel = FakeAgentChannel()
        val server = FakeAcpServer(channel)
        val adapter = OpenCodeAcpAdapter(hostId = "h", channel = channel, scope = adapterScope())
        val events = mutableListOf<AgentEvent>()
        collectAgentEvents(adapter, events)
        adapter.start("/repo")
        adapter.createSession("/repo")
        server.update(mapOf("sessionUpdate" to "future_update"))
        advanceUntilIdle()

        // Unknown protocol updates are technical noise: they go to logcat, not
        // into the conversation.
        assertTrue(fold(events).notices.isEmpty())
    }

    @Test
    fun process_exit_completes_the_prompt_and_reports_the_loss() = runTest {
        val channel = FakeAgentChannel()
        FakeAcpServer(channel)
        val adapter = OpenCodeAcpAdapter(hostId = "h", channel = channel, scope = adapterScope())
        val events = mutableListOf<AgentEvent>()
        collectAgentEvents(adapter, events)
        adapter.start("/repo")
        adapter.createSession("/repo")
        // The prompt request has no timeout, so only the exit path can end it.
        adapter.sendMessage("long task")
        advanceUntilIdle()

        channel.emitExit(1)
        advanceUntilIdle()

        val state = fold(events)
        assertTrue(state.connectionLost)
        assertEquals(AgentPhase.Closed, state.phase)
        assertTrue(state.turns.last().status.isTerminal)
    }

    @Test
    fun a_failed_prompt_write_is_reported_to_the_caller() = runTest {
        val channel = FakeAgentChannel()
        FakeAcpServer(channel)
        val adapter = OpenCodeAcpAdapter(hostId = "h", channel = channel, scope = adapterScope())
        collectAgentEvents(adapter, mutableListOf())
        adapter.start("/repo")
        adapter.createSession("/repo")
        advanceUntilIdle()
        channel.failWrites = true

        // The write happens on the caller's coroutine, so the repository sees
        // the failure and keeps the draft instead of clearing it.
        val failure = runCatching { adapter.sendMessage("hi") }.exceptionOrNull()
        assertNotNull(failure)
    }
}

class AcpDiffTest {

    @Test
    fun simple_diff_shows_only_the_changed_hunk() {
        val old = "one\ntwo\nthree\nfour\nfive"
        val new = "one\ntwo\nTHREE\nfour\nfive"
        assertEquals("-three\n+THREE", simpleDiff(old, new))
    }

    @Test
    fun simple_diff_for_a_new_file_is_the_full_content() {
        assertEquals("line one\nline two", simpleDiff(null, "line one\nline two"))
    }

    @Test
    fun identical_content_has_an_empty_diff() {
        assertEquals("", simpleDiff("same", "same"))
    }
}
