package dev.akko.miaominal.data.agent

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AgentFileReferencesTest {
    @Test fun preservesSpecialPathsAndDeduplicatesReferences() {
        val paths = listOf("C:\\Work & Tools\\a.txt", "/repo/a\"b\nc.txt")
        val result = AgentFileReferences.prompt("review", paths + paths.first())
        assertTrue(result.startsWith("review\n\n"))
        assertTrue(result.endsWith(JsonCodec().stringify(paths)))
        assertEquals("hello", AgentFileReferences.prompt("hello", emptyList()))
    }

    @Test fun cachesReasoningOptionsWithoutBreakingOldSnapshots() {
        val options = listOf(AgentReasoningOption("high", "High"))
        val state = AgentSessionState(reasoningOptions = options, selectedReasoning = "high",
            selectedModel = AgentModel("openai", "test", "Test", options, "high"))
        val decoded = AgentCacheCodec.decode(AgentCacheCodec.encode(AgentCacheCodec.Snapshot(state, "draft", listOf("/repo/test.kt"))))!!
        assertEquals(listOf("/repo/test.kt"), decoded.files)
        assertEquals(options, decoded.state.reasoningOptions)
        assertEquals("high", decoded.state.selectedReasoning)
        assertEquals(state.selectedModel, decoded.state.selectedModel)
    }
}
