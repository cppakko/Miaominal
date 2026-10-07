package dev.akko.miaominal.data.agent

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class AgentSessionStoreTest {
    @get:Rule val temporaryFolder = TemporaryFolder()

    @Test
    fun coldStartDropsLegacyConversationButKeepsUnsentDraft() {
        val store = AgentSessionStore(temporaryFolder.newFolder())
        val previous = AgentSessionState(
            session = AgentSessionInfo(AgentCli.Codex, "host", "/repo", "thread-1"),
            phase = AgentPhase.Ready,
            messages = listOf(
                AgentMessage("message-1", AgentRole.Assistant, listOf(ContentBlock.Text("text-1", "old reply")))
            )
        )
        store.save(AgentCacheCodec.Snapshot(previous, "unfinished prompt", listOf("/repo/file.kt")))

        assertEquals("unfinished prompt", store.retainDraftOnly())

        val restored = store.load()!!
        assertNull(restored.state.session)
        assertEquals(AgentPhase.Idle, restored.state.phase)
        assertFalse(restored.state.connectionLost)
        assertEquals(emptyList<AgentMessage>(), restored.state.messages)
        assertEquals(emptyList<String>(), restored.files)
        assertEquals("unfinished prompt", restored.draft)
    }

    @Test
    fun emptyDraftLeavesNoLocalConversationCache() {
        val store = AgentSessionStore(temporaryFolder.newFolder())
        store.saveDraft("temporary")
        store.saveDraft("")

        assertNull(store.load())
    }
}
