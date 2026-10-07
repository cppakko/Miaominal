package dev.akko.miaominal

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import dev.akko.miaominal.data.CredentialUpdate
import dev.akko.miaominal.data.agent.*
import dev.akko.miaominal.ui.LocalAgentConfigRepository
import dev.akko.miaominal.ui.screens.AgentLaunchSheet
import dev.akko.miaominal.ui.theme.MiaominalTheme
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** Uses only in-memory fixtures. Compiled by default; never run on a user's AVD without approval. */
class AgentConfigUiTest {
    @get:Rule val compose = createComposeRule()
    private val configs = AgentConfigRepository(object : AgentConfigPersistence {
        var document: String? = null
        override fun read() = document
        override fun write(document: String) { this.document = document }
        override fun delete() { document = null }
    })
    private val chat = FakeAgentChatRepository()

    private fun save(id: String, name: String, protocol: AgentApiProtocol) = runBlocking {
        configs.save(AgentConfigProfile(id, name, protocol, "https://fixture.test/v1", listOf("custom"), "custom"),
            CredentialUpdate.Set("fixture-only")).getOrThrow()
    }

    private fun show() = compose.setContent {
        CompositionLocalProvider(LocalAgentConfigRepository provides configs) {
            MiaominalTheme {
                AgentLaunchSheet(chat, "host", "Fixture", onDismiss = {}, onOpened = {})
            }
        }
    }

    @Test fun initialPreparationWaitsForTheRememberedPhoneConfiguration() {
        save("response", "Phone Responses", AgentApiProtocol.Responses)
        runBlocking { configs.remember("host", AgentCli.Codex, AgentConfigSource.Local("response")).getOrThrow() }
        show()
        compose.waitUntil(5000) { chat.preparedSources.isNotEmpty() }
        assertEquals(listOf(AgentConfigSource.Local("response")), chat.preparedSources)
        compose.onNodeWithTag("agent-config-selector").assertTextContains("Phone Responses")
    }

    @Test fun codexHidesChatCompletionsConfigurationsAndActiveSessionsRequireConfirmation() {
        save("response", "Phone Responses", AgentApiProtocol.Responses)
        save("chat", "Phone Chat", AgentApiProtocol.ChatCompletions)
        chat.isActive = true
        chat.setState(AgentSessionState(phase = AgentPhase.Ready,
            session = AgentSessionInfo(AgentCli.Codex, "host", "/repo", "old")))
        show()
        compose.waitUntil(5000) {
            runCatching { compose.onNodeWithTag("agent-config-selector").assertIsEnabled() }.isSuccess
        }
        compose.onNodeWithTag("agent-config-selector").performClick()
        compose.onNodeWithText("Phone Chat").assertDoesNotExist()
        compose.onNodeWithText("Phone Responses").performClick()
        compose.waitForIdle()
        assertTrue(chat.preparedSources.isEmpty())
        assertEquals(AgentConfigSource.Remote, runBlocking { configs.selection("host", AgentCli.Codex).getOrThrow() })
    }
}
