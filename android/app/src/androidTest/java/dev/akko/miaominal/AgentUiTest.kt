package dev.akko.miaominal

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import dev.akko.miaominal.data.agent.AgentCapabilities
import dev.akko.miaominal.data.agent.AgentCli
import dev.akko.miaominal.data.agent.AgentInteractionAnswer
import dev.akko.miaominal.data.agent.AgentMessage
import dev.akko.miaominal.data.agent.AgentPhase
import dev.akko.miaominal.data.agent.AgentProbeResult
import dev.akko.miaominal.data.agent.AgentRole
import dev.akko.miaominal.data.agent.AgentSessionInfo
import dev.akko.miaominal.data.agent.AgentSessionState
import dev.akko.miaominal.data.agent.AgentLaunchResult
import dev.akko.miaominal.data.agent.AgentHistoryEntry
import dev.akko.miaominal.data.agent.ContentBlock
import dev.akko.miaominal.data.agent.FakeAgentChatRepository
import dev.akko.miaominal.data.agent.InteractionKind
import dev.akko.miaominal.data.agent.InteractionOption
import dev.akko.miaominal.data.agent.InteractionQuestion
import dev.akko.miaominal.data.agent.InteractionRequest
import dev.akko.miaominal.data.InMemoryMiaominalRepository
import dev.akko.miaominal.data.SftpEntryKind
import dev.akko.miaominal.data.SftpEntryModel
import dev.akko.miaominal.data.SftpPhase
import dev.akko.miaominal.ui.LocalMiaominalRepository
import dev.akko.miaominal.ui.agent.AgentComposerTag
import dev.akko.miaominal.ui.agent.AgentInteractionTag
import dev.akko.miaominal.ui.agent.AgentMessageListTag
import dev.akko.miaominal.ui.agent.AgentSendTag
import dev.akko.miaominal.ui.screens.AgentChatScreen
import dev.akko.miaominal.ui.screens.AgentCwdFieldTag
import dev.akko.miaominal.ui.screens.AgentCwdPickerButtonTag
import dev.akko.miaominal.ui.screens.AgentDirectoryPickerTag
import dev.akko.miaominal.ui.screens.AgentDirectoryUseTag
import dev.akko.miaominal.ui.screens.AgentLaunchSheet
import dev.akko.miaominal.ui.screens.AgentNewConversationOpenTag
import dev.akko.miaominal.ui.screens.AgentNewConversationTag
import dev.akko.miaominal.ui.screens.AgentPrepareTag
import dev.akko.miaominal.ui.theme.MiaominalTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import java.util.concurrent.atomic.AtomicBoolean

/**
 * UI acceptance for the native AI chat: the launch panel prepares a CLI and
 * starts a conversation, the chat renders messages and sends input, and
 * interaction cards map UI answers back to protocol option ids.
 */
class AgentUiTest {

    @get:Rule
    val compose = createComposeRule()

    private val fake = FakeAgentChatRepository()

    @Test
    fun browsingConversationsDoesNotReplaceTheCurrentSession() {
        fake.setState(AgentSessionState(
            phase = AgentPhase.Ready,
            session = AgentSessionInfo(AgentCli.Codex, "host-1", "/srv/current", "running")
        ))
        fake.setDraft("unfinished prompt")
        fake.isActive = true
        compose.setContent {
            MiaominalTheme {
                AgentLaunchSheet(fake, "host-2", "Another host", onDismiss = {}, onOpened = {})
            }
        }
        compose.onNodeWithTag(AgentCwdFieldTag).assertDoesNotExist()
        compose.onNodeWithTag(AgentPrepareTag).performClick()
        // Opening the page and asking to load history must not silently close a running turn.
        compose.runOnIdle {
            assertEquals(null, fake.preparedWith)
            assertEquals("unfinished prompt", fake.draft.value)
            assertEquals("running", fake.currentSession()?.remoteSessionId)
        }
        val context = androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().targetContext
        compose.onNodeWithText(context.getString(R.string.agent_keep_current)).performClick()
        compose.runOnIdle { assertEquals(null, fake.preparedWith) }
    }

    @Test
    fun launchPanelPreparesTheCliAndStartsANewConversation() {
        val opened = AtomicBoolean(false)
        fake.prepareResult = AgentLaunchResult.Ready(
            probe = AgentProbeResult(true, "codex-test/1"),
            history = listOf(
                AgentHistoryEntry(remoteId = "thr_old", title = "之前的对话", cwd = "/srv/app")
            )
        )
        compose.setContent {
            MiaominalTheme {
                AgentLaunchSheet(
                    agentChat = fake,
                    hostId = "host-1",
                    hostLabel = "测试主机",
                    onDismiss = {},
                    onOpened = { opened.set(true) }
                )
            }
        }

        // No workspace and no manual tap: the panel connects and loads history
        // on its own, then adopts the newest workspace for a new conversation.
        compose.waitUntil(timeoutMillis = 5_000) { fake.preparedWith != null }
        assertEquals(
            Triple("host-1", AgentCli.Codex, ""),
            fake.preparedWith
        )
        compose.waitUntil(timeoutMillis = 5_000) {
            compose.onAllNodesWithText("/srv/app").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText("之前的对话").assertExists()
        compose.onNodeWithTag(AgentCwdFieldTag).assertDoesNotExist()
        compose.onNodeWithTag(AgentNewConversationOpenTag).performClick()
        compose.onNodeWithTag(AgentNewConversationTag).performScrollTo().performClick()
        compose.waitUntil(timeoutMillis = 5_000) { opened.get() }
        assertTrue(fake.startedNew)
    }

    @Test
    fun launchPanelResumesAnExistingConversation() {
        fake.prepareResult = AgentLaunchResult.Ready(
            probe = AgentProbeResult(true, "codex-test/1"),
            history = listOf(
                AgentHistoryEntry(remoteId = "thr_42", title = "历史会话", cwd = "/srv/history")
            )
        )
        var opened = false
        compose.setContent {
            MiaominalTheme {
                AgentLaunchSheet(
                    agentChat = fake,
                    hostId = "host-1",
                    hostLabel = "测试主机",
                    onDismiss = {},
                    onOpened = { opened = true }
                )
            }
        }
        // Opening the panel is enough: history is already there to resume.
        compose.waitUntil(timeoutMillis = 5_000) {
            compose.onAllNodesWithText("历史会话").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText("历史会话").performScrollTo().performClick()
        compose.waitUntil(timeoutMillis = 5_000) { opened }
        // The conversation's own workspace is used, never the process directory.
        assertEquals("thr_42", fake.resumedId)
        assertEquals("/srv/history", fake.resumedCwd)
    }

    @Test
    fun remoteDirectoryPickerFillsTheWorkingDirectory() {
        val repository = InMemoryMiaominalRepository()
        repository.seedSftpSession("host-1", SftpPhase.Ready).apply {
            seedTree(
                "/home/tester",
                listOf(
                    SftpEntryModel(
                        name = "projects",
                        path = "/home/tester/projects",
                        kind = SftpEntryKind.Directory,
                        size = null,
                        modifiedAtMs = null,
                        attributes = null,
                        owner = null
                    )
                )
            )
            seedTree("/home/tester/projects", emptyList())
        }
        compose.setContent {
            CompositionLocalProvider(LocalMiaominalRepository provides repository) {
                MiaominalTheme {
                    AgentLaunchSheet(
                        agentChat = fake,
                        hostId = "host-1",
                        hostLabel = "测试主机",
                        onDismiss = {},
                        onOpened = {}
                    )
                }
            }
        }

        compose.waitUntil(timeoutMillis = 5_000) { fake.preparedWith != null }
        compose.onNodeWithTag(AgentNewConversationOpenTag).performClick()
        compose.onNodeWithTag(AgentCwdPickerButtonTag).performClick()
        compose.onNodeWithTag(AgentDirectoryPickerTag).assertExists()
        compose.waitUntil(timeoutMillis = 5_000) {
            compose.onAllNodesWithText("projects").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText("projects").performClick()
        compose.onNodeWithTag(AgentDirectoryUseTag).performClick()
        compose.onNodeWithTag(AgentCwdFieldTag).assertTextContains("/home/tester/projects")
    }

    @Test
    fun remoteFilePickerSelectsFilesInsteadOfTheDirectory() {
        val repository = InMemoryMiaominalRepository()
        repository.seedSftpSession("host-1", SftpPhase.Ready).apply {
            seedTree("/repo", listOf(SftpEntryModel(
                name = "build.gradle.kts", path = "/repo/build.gradle.kts", kind = SftpEntryKind.File,
                size = 12, modifiedAtMs = null, attributes = null, owner = null
            )))
        }
        var selected: String? = null
        compose.setContent {
            CompositionLocalProvider(LocalMiaominalRepository provides repository) {
                MiaominalTheme {
                    dev.akko.miaominal.ui.screens.RemoteDirectoryPicker(
                        hostId = "host-1", initialPath = "/repo", onDismiss = {},
                        onSelect = { selected = it }, selectFile = true
                    )
                }
            }
        }
        compose.onNodeWithTag(AgentDirectoryUseTag).assertDoesNotExist()
        compose.waitUntil(timeoutMillis = 5_000) {
            compose.onAllNodesWithText("build.gradle.kts").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText("build.gradle.kts").performClick()
        compose.waitUntil { selected != null }
        assertEquals("/repo/build.gradle.kts", selected)
    }

    @Test
    fun chatRendersMessagesAndSendsComposerText() {
        fake.setState(
            readyState().copy(
                messages = listOf(
                    AgentMessage(
                        id = "m1",
                        role = AgentRole.User,
                        blocks = listOf(ContentBlock.Text("m1", "帮我运行测试"))
                    ),
                    AgentMessage(
                        id = "m2",
                        role = AgentRole.Assistant,
                        blocks = listOf(
                            ContentBlock.Text("m2", "好的，正在运行。"),
                            ContentBlock.ToolCall(
                                id = "t1",
                                callId = "call-1",
                                name = "command",
                                title = "cargo test",
                                status = dev.akko.miaominal.data.agent.ToolStatus.Running
                            )
                        )
                    )
                )
            )
        )
        var draft by mutableStateOf("")
        compose.setContent {
            MiaominalTheme {
                AgentChatScreen(
                    state = fake.state.value,
                    hostLabel = "测试主机",
                    draft = draft,
                    onDraftChange = { draft = it },
                    onSend = { fake.sentMessages += draft },
                    onStop = {},
                    onRespondOption = { _, _ -> },
                    onRespondAnswers = { _, _ -> },
                    onSelectModel = {},
                    onCloseSession = {}
                )
            }
        }
        compose.onNodeWithTag(AgentMessageListTag).assertExists()
        compose.onNodeWithText("帮我运行测试").assertExists()
        compose.onNodeWithText("好的，正在运行。").assertExists()
        compose.onNodeWithText("cargo test", substring = true).assertExists()

        compose.onNodeWithTag(AgentComposerTag).performTextInput("然后部署")
        compose.onNodeWithTag(AgentSendTag).performClick()
        assertEquals(listOf("然后部署"), fake.sentMessages)
    }

    @Test
    fun approvalCardMapsTheChosenOption() {
        val requestId = "codex:req:77"
        fake.setState(
            readyState().copy(
                interactions = listOf(
                    InteractionRequest(
                        id = requestId,
                        kind = InteractionKind.Approval,
                        turnId = "t1",
                        toolId = "cmd1",
                        title = "Run this command?",
                        detail = "rm -rf build",
                        options = listOf(
                            InteractionOption("accept", "Allow"),
                            InteractionOption("decline", "Deny")
                        )
                    )
                )
            )
        )
        compose.setContent {
            MiaominalTheme {
                AgentChatScreen(
                    state = fake.state.value,
                    hostLabel = "测试主机",
                    draft = "",
                    onDraftChange = {},
                    onSend = {},
                    onStop = {},
                    onRespondOption = { id, option ->
                        fake.responses += id to AgentInteractionAnswer.Option(option)
                    },
                    onRespondAnswers = { _, _ -> },
                    onSelectModel = {},
                    onCloseSession = {}
                )
            }
        }
        compose.onNodeWithTag(AgentInteractionTag).assertExists()
        compose.onNodeWithText("允许").performClick()
        assertEquals(
            listOf(requestId to AgentInteractionAnswer.Option("accept")),
            fake.responses
        )
    }

    @Test
    fun questionCardCollectsFreeformAnswers() {
        val requestId = "codex:req:88"
        fake.setState(
            readyState().copy(
                interactions = listOf(
                    InteractionRequest(
                        id = requestId,
                        kind = InteractionKind.Question,
                        turnId = "t1",
                        toolId = null,
                        title = "Codex needs your input",
                        questions = listOf(
                            InteractionQuestion(
                                id = "q1",
                                question = "Which branch?",
                                freeform = true
                            )
                        )
                    )
                )
            )
        )
        compose.setContent {
            MiaominalTheme {
                AgentChatScreen(
                    state = fake.state.value,
                    hostLabel = "测试主机",
                    draft = "",
                    onDraftChange = {},
                    onSend = {},
                    onStop = {},
                    onRespondOption = { _, _ -> },
                    onRespondAnswers = { id, answers ->
                        fake.responses += id to AgentInteractionAnswer.Answers(
                            answers.map { (questionId, values) ->
                                dev.akko.miaominal.data.agent.InteractionAnswerEntry(
                                    questionId,
                                    values
                                )
                            }
                        )
                    },
                    onSelectModel = {},
                    onCloseSession = {}
                )
            }
        }
        compose.onNodeWithTag("agent-answer-q1").performTextInput("main")
        compose.onNodeWithTag("agent-answer-submit").performScrollTo().performClick()
        assertEquals(1, fake.responses.size)
        val answer = fake.responses.single().second
        assertTrue(answer is AgentInteractionAnswer.Answers)
        assertEquals(
            listOf("main"),
            (answer as AgentInteractionAnswer.Answers).entries.single().answers
        )
    }

    private fun readyState(): AgentSessionState = AgentSessionState(
        session = AgentSessionInfo(
            cli = AgentCli.Codex,
            hostId = "host-1",
            cwd = "/srv/app",
            remoteSessionId = "thr_1",
            title = "会话"
        ),
        phase = AgentPhase.Ready,
        capabilities = AgentCapabilities()
    )
}
