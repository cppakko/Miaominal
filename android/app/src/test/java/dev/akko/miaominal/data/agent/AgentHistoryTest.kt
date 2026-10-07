package dev.akko.miaominal.data.agent

import org.junit.Assert.assertEquals
import org.junit.Test

class AgentHistoryTest {

    @Test
    fun workspacesAreNewestFirstAndDeduplicated() {
        val entries = listOf(
            AgentHistoryEntry("a", "older", updatedAtMs = 1_000, cwd = "/srv/one"),
            AgentHistoryEntry("b", "newest", updatedAtMs = 3_000, cwd = "/srv/two"),
            AgentHistoryEntry("c", "other", updatedAtMs = 2_000, cwd = "/srv/one"),
            AgentHistoryEntry("d", "no workspace", updatedAtMs = 4_000),
            AgentHistoryEntry("e", "blank", updatedAtMs = 5_000, cwd = " ")
        )

        assertEquals(listOf("/srv/two", "/srv/one"), historyWorkspaces(entries))
    }

    @Test
    fun entriesWithoutTimestampsKeepTheirOrder() {
        val entries = listOf(
            AgentHistoryEntry("a", "first", cwd = "/srv/one"),
            AgentHistoryEntry("b", "second", cwd = "/srv/two")
        )

        assertEquals(listOf("/srv/one", "/srv/two"), historyWorkspaces(entries))
    }
}
