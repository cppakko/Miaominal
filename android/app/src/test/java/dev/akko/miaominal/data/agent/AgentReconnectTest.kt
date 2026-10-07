package dev.akko.miaominal.data.agent

import kotlinx.coroutines.test.runTest
import org.junit.Assert.*
import org.junit.Test

class AgentReconnectTest {
    private val session = AgentSessionInfo(AgentCli.Codex, "windows", "C:\\work space", "existing-thread")

    @Test fun resumesCapturedIdentityEvenWhenHistoryCannotBeListed() = runTest {
        var resumed: String? = null
        prepareAndResumeConversation(session, { host, cli, cwd ->
            assertEquals(session.hostId, host)
            assertEquals(session.cli, cli)
            assertEquals(session.cwd, cwd)
            AgentLaunchResult.Ready(AgentProbeResult(true), emptyList(), "history timeout")
        }, { resumed = it })
        assertEquals("existing-thread", resumed)
    }

    @Test fun preparationFailureDoesNotResumeOrCreateAnotherConversation() = runTest {
        var resumed = false
        val result = runCatching {
            prepareAndResumeConversation(session, { _, _, _ -> AgentLaunchResult.Failed("offline") }, { resumed = true })
        }
        assertFalse(resumed)
        assertEquals("offline", result.exceptionOrNull()?.message)
    }

    @Test fun missingRemoteIdDoesNotStartAConnection() = runTest {
        var prepared = false
        val result = runCatching {
            prepareAndResumeConversation(session.copy(remoteSessionId = null), { _, _, _ ->
                prepared = true
                AgentLaunchResult.Ready(AgentProbeResult(true), emptyList())
            }, { fail("must not resume") })
        }
        assertTrue(result.isFailure)
        assertFalse(prepared)
    }

    @Test fun failedResumeIsNotReplacedWithANewConversation() = runTest {
        val result = runCatching {
            prepareAndResumeConversation(session, { _, _, _ ->
                AgentLaunchResult.Ready(AgentProbeResult(true), emptyList())
            }, { error("conversation missing") })
        }
        assertEquals("conversation missing", result.exceptionOrNull()?.message)
    }
}
