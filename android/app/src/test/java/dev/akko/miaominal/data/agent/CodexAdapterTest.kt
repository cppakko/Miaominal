package dev.akko.miaominal.data.agent

import dev.akko.miaominal.data.ShellType
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CodexAdapterTest {

    @Test
    fun windowsStderrKeepsSplitUtf8AndDecodesLegacyChineseErrors() {
        val utf8 = AgentStderrTail(ShellType.Cmd)
        val utf8Bytes = "系统找不到指定的路径".toByteArray(Charsets.UTF_8)
        utf8.append(utf8Bytes.copyOfRange(0, 2))
        utf8.append(utf8Bytes.copyOfRange(2, utf8Bytes.size))
        assertEquals("系统找不到指定的路径", utf8.text())

        val legacy = AgentStderrTail(ShellType.Cmd)
        val legacyBytes = "系统找不到指定的路径".toByteArray(java.nio.charset.Charset.forName("GB18030"))
        legacy.append(legacyBytes.copyOfRange(0, 3))
        legacy.append(legacyBytes.copyOfRange(3, legacyBytes.size))
        assertEquals("系统找不到指定的路径", legacy.text())
    }

    @Test
    fun reasoningSelectionIsSentAndClearedForUnsupportedModels() = runTest {
        val channel = FakeAgentChannel()
        val server = FakeCodexServer(channel)
        val adapter = CodexAdapter(hostId = "h", channel = channel, scope = adapterScope())
        val events = mutableListOf<AgentEvent>()
        collectAgentEvents(adapter, events)
        adapter.start("/repo")
        adapter.createSession("/repo")
        assertEquals("medium", fold(events).selectedReasoning)
        adapter.selectReasoning("high")
        adapter.sendMessage("review")
        assertEquals("high", server.requests.last { it.stringAt("method") == "turn/start" }.mapAt("params")?.stringAt("effort"))
        val unsupported = fold(events).models.last()
        adapter.selectModel(unsupported)
        advanceUntilIdle()
        adapter.sendMessage("hello")
        assertFalse(server.requests.last { it.stringAt("method") == "turn/start" }.mapAt("params")!!.containsKey("effort"))
        assertTrue(fold(events).reasoningOptions.isEmpty())
    }

    private fun fold(events: List<AgentEvent>): AgentSessionState =
        events.fold(AgentSessionState()) { state, event -> AgentReducer.reduce(state, event) }

    @Test
    fun startsOnPowerShellWithAWindowsWorkingDirectory() = runTest {
        val channel = FakeAgentChannel()
        FakeCodexServer(channel)
        val adapter = CodexAdapter(
            hostId = "windows-host",
            channel = channel,
            scope = adapterScope(),
            shellType = ShellType.PowerShell
        )

        assertTrue(adapter.start("C:\\Users\\tester\\My Project").available)
        assertEquals(
            "Set-Location -LiteralPath 'C:\\Users\\tester\\My Project'; " +
                "if (\$?) { codex app-server }",
            channel.execCommands.single()
        )
    }

    @Test
    fun initializes_and_maps_a_full_turn() = runTest {
        val channel = FakeAgentChannel()
        val server = FakeCodexServer(channel)
        val adapter = CodexAdapter(
            hostId = "host-1",
            channel = channel,
            scope = adapterScope()
        )
        val events = mutableListOf<AgentEvent>()
        collectAgentEvents(adapter, events)

        val probe = adapter.start("/home/me/my project")
        assertTrue(probe.available)
        assertTrue(channel.execCommands.single().contains("cd '/home/me/my project'"))
        assertTrue(channel.execCommands.single().contains("codex app-server"))

        val following = server.requests.mapNotNull { it["method"] as? String }
        assertTrue(following.contains("initialize"))
        assertTrue(following.contains("initialized"))

        assertTrue(events.any { it is AgentEvent.ModelsLoaded })
        val afterStart = fold(events)
        assertEquals(AgentPhase.Ready, afterStart.phase)
        assertEquals(2, afterStart.models.size)

        adapter.createSession("/home/me/my project")
        assertTrue(
            events.any {
                it is AgentEvent.SessionOpened && it.session.remoteSessionId == "thr_1"
            }
        )

        adapter.sendMessage("hello")
        server.notify("turn/started", mapOf("turn" to mapOf("id" to "turn_1")))
        server.notify(
            "item/started",
            mapOf(
                "item" to mapOf("id" to "msg_1", "type" to "agentMessage", "text" to "")
            )
        )
        server.notify(
            "item/agentMessage/delta",
            mapOf("itemId" to "msg_1", "delta" to "Hello ")
        )
        server.notify(
            "item/agentMessage/delta",
            mapOf("itemId" to "msg_1", "delta" to "world")
        )
        server.notify(
            "item/completed",
            mapOf(
                "item" to mapOf("id" to "msg_1", "type" to "agentMessage", "text" to "Hello world")
            )
        )
        server.notify(
            "turn/completed",
            mapOf("turn" to mapOf("id" to "turn_1", "status" to "completed"))
        )
        advanceUntilIdle()

        val state = fold(events)
        val text = state.messages
            .flatMap { it.blocks }
            .filterIsInstance<ContentBlock.Text>()
            .last()
        assertEquals("Hello world", text.text)
        assertEquals(AgentTurnStatus.Completed, state.turns.last().status)
        assertFalse(state.messages.last().streaming)
    }

    @Test
    fun command_approval_round_trips_to_the_server_request() = runTest {
        val channel = FakeAgentChannel()
        val server = FakeCodexServer(channel)
        val adapter = CodexAdapter(hostId = "h", channel = channel, scope = adapterScope())
        val events = mutableListOf<AgentEvent>()
        collectAgentEvents(adapter, events)

        adapter.start("/repo")
        adapter.createSession("/repo")
        server.serverRequest(
            77,
            "item/commandExecution/requestApproval",
            mapOf(
                "itemId" to "cmd_1",
                "turnId" to "turn_1",
                "command" to "rm -rf build",
                "reason" to "clean the workspace"
            )
        )
        advanceUntilIdle()

        val request = fold(events).pendingInteraction
        assertNotNull(request)
        assertEquals("codex:req:77", request!!.id)
        assertEquals(InteractionKind.Approval, request.kind)
        assertTrue(request.options.any { it.id == "accept" })

        adapter.respondInteraction(request.id, AgentInteractionAnswer.Option("accept"))
        advanceUntilIdle()
        val response = server.responseTo(77)?.get("result").asMap()
        assertEquals("accept", response?.get("decision"))
        assertFalse(fold(events).let { it.pendingInteraction != null })
    }

    @Test
    fun user_input_request_maps_questions_and_answers() = runTest {
        val channel = FakeAgentChannel()
        val server = FakeCodexServer(channel)
        val adapter = CodexAdapter(hostId = "h", channel = channel, scope = adapterScope())
        val events = mutableListOf<AgentEvent>()
        collectAgentEvents(adapter, events)

        adapter.start("/repo")
        adapter.createSession("/repo")
        server.serverRequest(
            88,
            "item/tool/requestUserInput",
            mapOf(
                "itemId" to "call_1",
                "questions" to listOf(
                    mapOf(
                        "id" to "q1",
                        "question" to "Which branch?",
                        "options" to listOf(
                            mapOf("label" to "main", "description" to "the main line"),
                            mapOf("label" to "dev")
                        )
                    )
                )
            )
        )
        advanceUntilIdle()

        val request = fold(events).pendingInteraction
        assertNotNull(request)
        assertEquals(InteractionKind.Question, request!!.kind)
        assertEquals("Which branch?", request.questions.single().question)
        // The app-server options have no ids; the label is what is answered.
        assertEquals(listOf("main", "dev"), request.questions.single().options.map { it.id })

        adapter.respondInteraction(
            request.id,
            AgentInteractionAnswer.Answers(
                listOf(InteractionAnswerEntry("q1", listOf("dev")))
            )
        )
        advanceUntilIdle()
        val result = server.responseTo(88)?.get("result").asMap()
        val answers = result?.get("answers").asMap()
        val answer = answers?.get("q1").asMap()
        assertEquals(listOf("dev"), answer?.get("answers"))
    }

    @Test
    fun history_replay_populates_the_conversation_once() = runTest {
        val channel = FakeAgentChannel()
        val server = FakeCodexServer(channel)
        val adapter = CodexAdapter(hostId = "h", channel = channel, scope = adapterScope())
        val events = mutableListOf<AgentEvent>()
        collectAgentEvents(adapter, events)

        adapter.start("/repo")
        adapter.loadSession("thr_old")
        // Load twice: replaying history must not duplicate messages.
        adapter.loadSession("thr_old")
        advanceUntilIdle()

        val state = fold(events)
        val userMessages = state.messages.filter { it.role == AgentRole.User }
        val assistantMessages = state.messages.filter { it.role == AgentRole.Assistant }
        assertEquals(1, userMessages.size)
        assertEquals(1, assistantMessages.size)
        assertEquals("hello from history", assistantMessages.last().blocks.single().let {
            (it as ContentBlock.Text).text
        })
        assertEquals("thr_old", state.session?.remoteSessionId)
    }

    @Test
    fun list_sessions_lists_every_workspace_and_maps_the_entry_cwd() = runTest {
        val channel = FakeAgentChannel()
        val server = FakeCodexServer(channel)
        val adapter = CodexAdapter(hostId = "h", channel = channel, scope = adapterScope())
        collectAgentEvents(adapter, mutableListOf())
        adapter.start("/repo")

        val history = adapter.listSessions()
        assertEquals(1, history.size)
        assertEquals("thr_old", history.single().remoteId)
        assertEquals("Old thread", history.single().title)
        assertEquals("/repo", history.single().cwd)

        // With no workspace the CLI lists every workspace; no filter is sent.
        val global = server.requests.last { it.stringAt("method") == "thread/list" }
        assertFalse(global.mapAt("params")!!.containsKey("cwd"))

        // A chosen workspace is still forwarded as the server-side filter.
        adapter.listSessions("/repo")
        val filtered = server.requests.last { it.stringAt("method") == "thread/list" }
        assertEquals("/repo", filtered.mapAt("params")?.stringAt("cwd"))
    }

    @Test
    fun resume_targets_the_conversation_workspace() = runTest {
        val channel = FakeAgentChannel()
        val server = FakeCodexServer(channel)
        val adapter = CodexAdapter(hostId = "h", channel = channel, scope = adapterScope())
        val events = mutableListOf<AgentEvent>()
        collectAgentEvents(adapter, events)

        // A blank directory boots the CLI in the SSH login directory: no cd.
        adapter.start("")
        assertEquals("codex app-server", channel.execCommands.single())

        adapter.loadSession("thr_old", "/srv/other")
        advanceUntilIdle()

        val resume = server.requests.last { it.stringAt("method") == "thread/resume" }
        assertEquals("/srv/other", resume.mapAt("params")?.stringAt("cwd"))
        val state = fold(events)
        assertEquals("/srv/other", state.session?.cwd)
        assertEquals("thr_old", state.session?.remoteSessionId)
    }

    @Test
    fun unknown_notifications_do_not_reach_the_chat() = runTest {
        val channel = FakeAgentChannel()
        val server = FakeCodexServer(channel)
        val adapter = CodexAdapter(hostId = "h", channel = channel, scope = adapterScope())
        val events = mutableListOf<AgentEvent>()
        collectAgentEvents(adapter, events)

        adapter.start("/repo")
        adapter.createSession("/repo")
        server.notify("future/feature/happened", mapOf("detail" to "something new"))
        advanceUntilIdle()

        // Unknown protocol events are technical noise: they go to logcat, not
        // into the conversation.
        assertTrue(fold(events).notices.isEmpty())
    }

    @Test
    fun interrupt_targets_the_active_turn() = runTest {
        val channel = FakeAgentChannel()
        val server = FakeCodexServer(channel)
        val adapter = CodexAdapter(hostId = "h", channel = channel, scope = adapterScope())
        collectAgentEvents(adapter, mutableListOf())

        adapter.start("/repo")
        adapter.createSession("/repo")
        adapter.sendMessage("stop me")
        adapter.cancelTurn()
        advanceUntilIdle()

        val interrupt = server.requests.last { it["method"] == "turn/interrupt" }
        assertEquals("turn_1", interrupt.mapAt("params")?.stringAt("turnId"))
        assertEquals("thr_1", interrupt.mapAt("params")?.stringAt("threadId"))
    }

    @Test
    fun process_exit_reports_connection_lost_and_closed() = runTest {
        val channel = FakeAgentChannel()
        FakeCodexServer(channel)
        val adapter = CodexAdapter(hostId = "h", channel = channel, scope = adapterScope())
        val events = mutableListOf<AgentEvent>()
        collectAgentEvents(adapter, events)

        adapter.start("/repo")
        adapter.createSession("/repo")
        channel.emitExit(1)
        advanceUntilIdle()

        // The terminal events must survive the closed flag: they are what tells
        // the chat the session is gone.
        val state = fold(events)
        assertTrue(state.connectionLost)
        assertEquals(AgentPhase.Closed, state.phase)
        assertTrue(state.error?.contains("exited") == true)
    }
}

private fun Any?.asMap(): Map<String, Any?>? = this as? Map<String, Any?>
