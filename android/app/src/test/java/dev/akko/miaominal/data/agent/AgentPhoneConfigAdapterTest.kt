package dev.akko.miaominal.data.agent

import dev.akko.miaominal.data.ShellType
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.launch
import org.junit.Assert.*
import org.junit.Test

@OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
class AgentPhoneConfigAdapterTest {
    private fun config(protocol: AgentApiProtocol = AgentApiProtocol.Responses) = AgentConfigSnapshot(
        AgentConfigProfile("id", "Phone", protocol, "https://proxy.test/v1", listOf("custom", "other"), "custom", true),
        "fixture-key"
    )

    @Test fun codexUsesCustomModelsForCreateResumeAndEveryTurn() = runTest {
        val channel = FakeAgentChannel().apply { acknowledgeBootstrap = true }
        val server = FakeCodexServer(channel)
        val adapter = CodexAdapter(hostId = "host", channel = channel, scope = adapterScope(), config = config())
        val events = mutableListOf<AgentEvent>()
        collectAgentEvents(adapter, events)
        assertTrue(adapter.start("/repo").available)
        assertFalse(server.requests.any { it.stringAt("method") == "model/list" })
        val models = events.filterIsInstance<AgentEvent.ModelsLoaded>().last().models
        assertEquals(listOf("custom", "other"), models.map { it.modelId })
        assertTrue(models.all { it.reasoningOptions.isEmpty() })
        adapter.listSessions(null)
        assertEquals(emptyList<String>(), server.requests.last { it.stringAt("method") == "thread/list" }
            .mapAt("params")!!["modelProviders"])
        adapter.createSession("/repo")
        adapter.selectModel(models.last())
        adapter.loadSession("thr_old", "/repo")
        adapter.sendMessage("hello")
        for (method in listOf("thread/start", "thread/resume")) {
            val params = server.requests.last { it.stringAt("method") == method }.mapAt("params")!!
            assertEquals(AgentProcessChannel.PROVIDER, params.stringAt("modelProvider"))
            assertEquals(if (method == "thread/start") "custom" else "other", params.stringAt("model"))
        }
        assertEquals("other", server.requests.last { it.stringAt("method") == "turn/start" }.mapAt("params")!!.stringAt("model"))
        assertFalse(channel.execCommands.single().contains("fixture-key"))
    }

    private fun scriptAcp(channel: FakeAgentChannel, confirm: Boolean = true) {
        val json = JsonCodec()
        channel.onLine = { message ->
            val id = message["id"]
            val method = message.stringAt("method")
            val params = message.mapAt("params").orEmpty()
            val current = if (method == "session/set_config_option" && confirm) params.stringAt("value") else "remote/old"
            val options = listOf(mapOf("id" to "model", "category" to "model", "currentValue" to current,
                "options" to listOf(mapOf("value" to "remote/old", "name" to "Remote"))))
            val result = when (method) {
                "initialize" -> mapOf("agentCapabilities" to mapOf("loadSession" to true))
                "session/new", "session/load" -> mapOf("sessionId" to "ses_1", "configOptions" to options)
                "session/set_config_option" -> mapOf("configOptions" to options)
                "session/prompt" -> mapOf("stopReason" to "end_turn")
                else -> emptyMap()
            }
            if (id != null) channel.emitStdout(json.stringify(mapOf("jsonrpc" to "2.0", "id" to id, "result" to result)) + "\n")
        }
    }

    @Test fun codexRejectsUnconfirmedOverridesBeforeSending() = runTest {
        val channel = FakeAgentChannel().apply { acknowledgeBootstrap = true }
        val server = FakeCodexServer(channel).apply { confirmConfiguredProvider = false }
        val adapter = CodexAdapter(hostId = "h", channel = channel, scope = adapterScope(), config = config())
        assertTrue(adapter.start("/repo").available)
        assertTrue(runCatching { adapter.createSession("/repo") }.isFailure)
        assertTrue(runCatching { adapter.loadSession("old", "/repo") }.isFailure)
        assertTrue(runCatching { adapter.sendMessage("hello") }.isFailure)
        assertFalse(server.requests.any { it.stringAt("method") == "turn/start" })
    }

    @Test fun openCodeOverridesHistoricalProviderAndConfirmsModelBeforeSending() = runTest {
        for (protocol in AgentApiProtocol.entries) {
            val channel = FakeAgentChannel().apply { acknowledgeBootstrap = true }
            scriptAcp(channel)
            val adapter = OpenCodeAcpAdapter(hostId = "h", channel = channel, scope = adapterScope(), config = config(protocol))
            val events = mutableListOf<AgentEvent>()
            collectAgentEvents(adapter, events)
            assertTrue(adapter.start("/repo").available)
            adapter.createSession("/repo")
            adapter.loadSession("ses_old", "/repo")
            assertEquals("miaominal_phone/custom", channel.writtenMessages.last { it.stringAt("method") == "session/set_config_option" }
                .mapAt("params")!!.stringAt("value"))
            val model = events.filterIsInstance<AgentEvent.ModelsLoaded>().last().models.last()
            adapter.selectModel(model)
            adapter.sendMessage("hello")
            val methods = channel.writtenMessages.map { it.stringAt("method") }
            assertTrue(methods.lastIndexOf("session/set_config_option") < methods.lastIndexOf("session/prompt"))
            assertEquals(listOf("custom", "other"), events.filterIsInstance<AgentEvent.ModelsLoaded>().last().models.map { it.name })
            assertFalse(channel.execCommands.single().contains("fixture-key"))
            adapter.close()
        }
    }

    @Test fun openCodeRejectsAnUnconfirmedHistoricalModelInsteadOfUsingRemoteAuth() = runTest {
        val channel = FakeAgentChannel().apply { acknowledgeBootstrap = true }
        scriptAcp(channel, confirm = false)
        val adapter = OpenCodeAcpAdapter(hostId = "h", channel = channel, scope = adapterScope(), config = config())
        assertTrue(adapter.start("/repo").available)
        assertTrue(runCatching { adapter.loadSession("old", "/repo") }.isFailure)
        assertFalse(channel.writtenMessages.any { it.stringAt("method") == "session/prompt" })
    }

    @Test fun everyShellUsesStdinAndKeepsCredentialsOutOfItsCommand() {
        for (shell in ShellType.entries) {
            val configured = AgentProcessChannel(FakeAgentChannel(), AgentCli.Codex, shell, config())
            val cwd = if (shell == ShellType.Cmd || shell == ShellType.PowerShell) "C:\\a'b%TEMP%\\repo" else "/a'b\$x`y/repo"
            val command = configured.bootstrapCommand(cwd)
            val decoded = if ("-EncodedCommand " in command) String(java.util.Base64.getDecoder()
                .decode(command.substringAfter("-EncodedCommand ")), Charsets.UTF_16LE) else command
            assertFalse(command.contains("fixture-key"))
            assertFalse(decoded.contains("fixture-key"))
            assertFalse(decoded.contains("https://proxy.test"))
            assertTrue(command.length < 8191)
            val lines = String(configured.bootstrapInput(cwd), Charsets.UTF_8).split('\n')
            assertEquals("fixture-key", lines[0])
            assertTrue(lines[2].contains("model_provider"))
            assertFalse(decoded.contains("Set-Content"))
            assertFalse(decoded.contains("auth.json"))
        }
    }

    @Test fun openCodeProtocolSelectsTheCorrectSdkAndUsesAnEnvironmentKeyReference() {
        for (protocol in AgentApiProtocol.entries) {
            val channel = AgentProcessChannel(FakeAgentChannel(), AgentCli.OpenCode, ShellType.Posix, config(protocol))
            val json = JsonCodec().parse(channel.openCodeConfig())!!
            val provider = json.mapAt("provider")!!.mapAt("miaominal_phone")!!
            assertEquals(if (protocol == AgentApiProtocol.Responses) "@ai-sdk/openai" else "@ai-sdk/openai-compatible", provider.stringAt("npm"))
            assertEquals("{env:MIAOMINAL_AI_API_KEY}", provider.mapAt("options")!!.stringAt("apiKey"))
            assertEquals("https://proxy.test/v1", provider.mapAt("options")!!.stringAt("baseURL"))
            assertFalse(channel.openCodeConfig().contains("fixture-key"))
        }
    }

    @Test fun bootstrapFailureClosesTheChannelAndNeverStartsTheProtocol() = runTest {
        val channel = FakeAgentChannel()
        val adapter = CodexAdapter(hostId = "h", channel = channel, scope = adapterScope(), config = config(), requestTimeoutMs = 10)
        val result = adapter.start("/repo")
        assertFalse(result.available)
        assertTrue(channel.closedChannels.isNotEmpty())
        assertFalse(channel.writtenMessages.any { it.stringAt("method") == "initialize" })
    }

    @Test fun upstreamErrorFieldsAreRedactedBeforeEnteringPublicState() {
        val snapshot = config()
        val ended = snapshot.redactEvent(AgentEvent.TurnEnded("turn", AgentTurnStatus.Failed, "Bearer fixture-key"))
            as AgentEvent.TurnEnded
        assertEquals("Bearer [REDACTED]", ended.error)
        val phase = snapshot.redactEvent(AgentEvent.PhaseChanged(AgentPhase.Failed, "fixture-key")) as AgentEvent.PhaseChanged
        assertEquals("[REDACTED]", phase.error)
        val quoted = AgentConfigSnapshot(snapshot.profile, "key\"\\token")
        val error = JsonCodec().stringify(mapOf("apiKey" to "key\"\\token"))
        assertFalse(quoted.redact(error).contains("token"))
    }

    @Test fun fragmentedStderrAndProtocolBoundaryNeverExposeTheCredential() = runTest {
        val delegate = FakeAgentChannel()
        val channel = AgentProcessChannel(delegate, AgentCli.Codex, ShellType.Posix, config())
        val events = mutableListOf<AgentChannelEventModel>()
        val collector = backgroundScope.launch(UnconfinedTestDispatcher(testScheduler)) {
            channel.events().collect { events += it }
        }
        channel.launch("/repo")
        val marker = Regex("MIAOMINAL_READY_[a-fA-F0-9]+").find(delegate.execCommands.single())!!.value
        delegate.emitStdout(marker.take(12))
        delegate.emitStdout(marker.drop(12) + "\n{\"id\":1}\n")
        channel.awaitReady(100)
        assertEquals("{\"id\":1}\n", String(events.filterIsInstance<AgentChannelEventModel.Stdout>().single().data))
        delegate.emitStderr("Authorization: Bearer fixture-")
        assertTrue(events.none { it is AgentChannelEventModel.Stderr })
        delegate.emitStderr("key\nlast fixture-")
        delegate.emitStderr("key")
        delegate.emitExit(1)
        val stderr = events.filterIsInstance<AgentChannelEventModel.Stderr>().joinToString("") { String(it.data) }
        assertEquals("Authorization: Bearer [REDACTED]\nlast [REDACTED]", stderr)
        collector.cancel()
    }

    @Test fun cancellingBootstrapClosesItsExecChannel() = runTest {
        val delegate = FakeAgentChannel()
        val channel = AgentProcessChannel(delegate, AgentCli.Codex, ShellType.Posix, config())
        channel.launch("/repo")
        val pending = launch(UnconfinedTestDispatcher(testScheduler)) { channel.awaitReady(10_000) }
        pending.cancel()
        pending.join()
        assertEquals(listOf(1L), delegate.closedChannels)
    }

    @Test fun credentialsSplitAcrossProtocolDeltasAreMaskedBeforeStatePublication() {
        val snapshot = config()
        var state = AgentSessionState(messages = listOf(AgentMessage("m", AgentRole.Assistant,
            listOf(ContentBlock.Text("b", "")))))
        for (part in listOf("Authorization: fixture-", "key")) {
            val event = AgentEvent.BlockDelta("m", "b", part)
            state = snapshot.redactDeltaState(AgentReducer.reduce(state, snapshot.redactEvent(event)), event)
            assertFalse(state.toString().contains("fixture-key"))
        }
        assertEquals("Authorization: [REDACTED]", (state.messages.single().blocks.single() as ContentBlock.Text).text)
    }
}
