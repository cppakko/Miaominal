package dev.akko.miaominal

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.AndroidPlatformHost
import dev.akko.miaominal.data.NativeBackendClient
import dev.akko.miaominal.nativebridge.AuthMethodDto
import dev.akko.miaominal.nativebridge.Backend
import dev.akko.miaominal.nativebridge.BackendException
import dev.akko.miaominal.nativebridge.CredentialAction
import dev.akko.miaominal.nativebridge.CredentialKindDto
import dev.akko.miaominal.nativebridge.ForwardFailureKind
import dev.akko.miaominal.nativebridge.ForwardKindDto
import dev.akko.miaominal.nativebridge.ForwardPhase
import dev.akko.miaominal.nativebridge.ForwardRuleDto
import dev.akko.miaominal.nativebridge.HostUpdate
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * native acceptance: forwarding session lifecycle against the real bridge.
 *
 * Local/remote data round trips, port collisions, and multi-hop routes run in
 * the Rust integration suites against a real test SSH server; this suite proves
 * the Android wiring: typed terminal failures, idempotent start/stop, registry
 * snapshots, and that a restarted backend never auto-starts saved rules.
 */
@RunWith(AndroidJUnit4::class)
class NativeForwardTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val host = AndroidPlatformHost(context)

    private fun newBackend() = Backend(host, NativeBackendClient.configFor(context))

    private suspend fun prepareUnreachableHost(backend: Backend, id: String) {
        backend.saveHost(
            HostUpdate(
                id = id,
                name = "P5 unreachable",
                group = null,
                tags = null,
                kind = null,
                host = "127.0.0.1",
                port = 1.toUShort(),
                username = "tester",
                authMethod = AuthMethodDto.PASSWORD,
                privateKeyPath = null,
                managedKeyId = null,
                certificatePath = null,
                startupCommand = null,
                environmentVariables = null,
                shellType = null,
                isFavorite = null,
                lastConnectedAtMs = null
            )
        )
        backend.setHostCredential(id, CredentialKindDto.PASSWORD, CredentialAction.Set("secret"))
    }

    private suspend fun saveLocalRule(backend: Backend, profileId: String, ruleId: String) {
        backend.upsertForward(
            ForwardRuleDto(
                id = ruleId,
                profileId = profileId,
                label = "rule $ruleId",
                kind = ForwardKindDto.LOCAL,
                listenHost = "127.0.0.1",
                listenPort = 0u,
                targetHost = "127.0.0.1",
                targetPort = 9u,
                enabled = true
            )
        )
    }

    private fun ForwardPhase.isTerminal(): Boolean =
        this == ForwardPhase.STOPPED || this == ForwardPhase.FAILED

    private fun openFdCount(): Int = java.io.File("/proc/self/fd").list()?.size ?: -1

    @Test
    fun unreachableHostFailsWithATypedFailureAndClosesIdempotently() = runBlocking {
        val backend = newBackend()
        try {
            backend.initialize()
            backend.clearLocalData()
            prepareUnreachableHost(backend, "p5-unreachable")
            saveLocalRule(backend, "p5-unreachable", "rule-1")

            val session = backend.startForwardSession("p5-unreachable", "rule-1")
            assertEquals("p5-unreachable", session.profileId())
            assertEquals("rule-1", session.ruleId())

            val deadline = System.currentTimeMillis() + 20_000
            var snapshot = session.snapshot()
            while (!snapshot.phase.isTerminal() && System.currentTimeMillis() < deadline) {
                Thread.sleep(100)
                snapshot = session.snapshot()
            }
            assertEquals(ForwardPhase.FAILED, snapshot.phase)
            val failure = snapshot.failure
            assertNotNull(failure)
            assertEquals(ForwardFailureKind.TRANSPORT, failure!!.kind)
            assertTrue("the failing node must be reported", failure.nodeLabel.isNotBlank())

            val connectionId = session.connectionId()
            assertTrue(backend.forwardSessionSnapshots().any { it.connectionId == connectionId })
            assertTrue(backend.closeForwardSession("p5-unreachable", "rule-1"))
            assertFalse(backend.closeForwardSession("p5-unreachable", "rule-1"))
            assertTrue(backend.forwardSessionSnapshots().isEmpty())
        } finally {
            backend.shutdown()
        }
    }

    @Test
    fun missingRulesAreRejectedWithoutStartingASession() = runBlocking {
        val backend = newBackend()
        try {
            backend.initialize()
            backend.clearLocalData()
            prepareUnreachableHost(backend, "p5-unreachable")

            val failure = runCatching {
                backend.startForwardSession("p5-unreachable", "missing")
            }
            assertTrue(failure.isFailure)
            assertTrue(failure.exceptionOrNull() is BackendException.InvalidArgument)
            assertTrue(backend.forwardSessionSnapshots().isEmpty())
        } finally {
            backend.shutdown()
        }
    }

    @Test
    fun startingAnActiveRuleReusesTheSession() = runBlocking {
        val backend = newBackend()
        try {
            backend.initialize()
            backend.clearLocalData()
            prepareUnreachableHost(backend, "p5-unreachable")
            saveLocalRule(backend, "p5-unreachable", "rule-1")

            val first = backend.startForwardSession("p5-unreachable", "rule-1")
            val second = backend.startForwardSession("p5-unreachable", "rule-1")
            assertEquals(first.connectionId(), second.connectionId())
            assertEquals(1, backend.forwardSessionSnapshots().size)

            first.cancel()
            assertTrue(backend.closeForwardSession("p5-unreachable", "rule-1"))
        } finally {
            backend.shutdown()
        }
    }

    @Test
    fun aRestartedBackendDoesNotAutoStartSavedRules() = runBlocking {
        val first = newBackend()
        try {
            first.initialize()
            first.clearLocalData()
            prepareUnreachableHost(first, "p5-unreachable")
            saveLocalRule(first, "p5-unreachable", "rule-1")
            // The saved `enabled` flag is configuration only.
            assertTrue(first.forwardsSnapshot().any { it.enabled })
            assertTrue(first.forwardSessionSnapshots().isEmpty())
        } finally {
            first.shutdown()
        }

        val second = newBackend()
        try {
            second.initialize()
            assertTrue(
                "a fresh backend must not start anything from the saved rules",
                second.forwardSessionSnapshots().isEmpty()
            )
            assertTrue(second.forwardsSnapshot().any { it.enabled })
        } finally {
            second.shutdown()
        }
    }

    @Test
    fun shutdownStopsEveryForwardSession() = runBlocking {
        val backend = newBackend()
        try {
            backend.initialize()
            backend.clearLocalData()
            prepareUnreachableHost(backend, "p5-unreachable")
            saveLocalRule(backend, "p5-unreachable", "rule-1")
            saveLocalRule(backend, "p5-unreachable", "rule-2")

            val first = backend.startForwardSession("p5-unreachable", "rule-1")
            val second = backend.startForwardSession("p5-unreachable", "rule-2")
            assertEquals(2, backend.forwardSessionSnapshots().size)

            backend.shutdown()

            assertTrue(backend.forwardSessionSnapshots().isEmpty())
            assertTrue(first.snapshot().phase.isTerminal())
            assertTrue(second.snapshot().phase.isTerminal())
        } finally {
            backend.shutdown()
        }
    }

    @Test
    fun repeatedStartAndStopKeepsDescriptorsStable() = runBlocking {
        val backend = newBackend()
        try {
            backend.initialize()
            backend.clearLocalData()
            prepareUnreachableHost(backend, "p5-unreachable")
            saveLocalRule(backend, "p5-unreachable", "rule-1")

            // Warm up allocator/runtime state before the baseline.
            repeat(3) {
                backend.startForwardSession("p5-unreachable", "rule-1")
                backend.closeForwardSession("p5-unreachable", "rule-1")
            }
            val before = openFdCount()
            repeat(100) {
                val session = backend.startForwardSession("p5-unreachable", "rule-1")
                session.cancel()
                assertTrue(backend.closeForwardSession("p5-unreachable", "rule-1"))
            }
            val after = openFdCount()
            assertTrue("descriptor count leaked: before=$before after=$after", after <= before + 8)
            assertTrue(backend.forwardSessionSnapshots().isEmpty())
        } finally {
            backend.shutdown()
        }
    }

    @Test
    fun deletingARuleStopsItsRunningSession() = runBlocking {        val backend = newBackend()
        try {
            backend.initialize()
            backend.clearLocalData()
            prepareUnreachableHost(backend, "p5-unreachable")
            saveLocalRule(backend, "p5-unreachable", "rule-1")

            val session = backend.startForwardSession("p5-unreachable", "rule-1")
            assertTrue(backend.deleteForward("p5-unreachable", "rule-1"))
            val deadline = System.currentTimeMillis() + 8_000
            while (!session.snapshot().phase.isTerminal() && System.currentTimeMillis() < deadline) {
                Thread.sleep(50)
            }
            assertTrue(
                "deleting a rule must stop its forwarding session",
                session.snapshot().phase.isTerminal()
            )
            assertTrue(backend.forwardsSnapshot().isEmpty())
        } finally {
            backend.shutdown()
        }
    }
}
