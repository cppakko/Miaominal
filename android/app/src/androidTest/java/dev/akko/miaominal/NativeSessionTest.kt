package dev.akko.miaominal

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.AndroidPlatformHost
import dev.akko.miaominal.data.NativeBackendClient
import dev.akko.miaominal.nativebridge.AuthMethodDto
import dev.akko.miaominal.nativebridge.Backend
import dev.akko.miaominal.nativebridge.CredentialAction
import dev.akko.miaominal.nativebridge.CredentialKindDto
import dev.akko.miaominal.nativebridge.HostUpdate
import dev.akko.miaominal.nativebridge.SshSessionPhase
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * SSH native acceptance: a real session handle goes through the typed phases,
 * failures are reported as interrupted sessions, and closing is idempotent.
 * Terminal behavior against a real server lives in the Rust integration tests.
 */
@RunWith(AndroidJUnit4::class)
class NativeSessionTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val host = AndroidPlatformHost(context)

    private fun newBackend() = Backend(host, NativeBackendClient.configFor(context))

    private suspend fun saveUnreachableHost(backend: Backend, id: String) {
        backend.saveHost(
            HostUpdate(
                id = id,
                name = "P3 unreachable",
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

    private fun SshSessionPhase.isTerminal(): Boolean =
        this == SshSessionPhase.CLOSED || this == SshSessionPhase.FAILED

    @Test
    fun unreachableHostFailsThenClosesIdempotently() = runBlocking {
        val backend = newBackend()
        try {
            backend.initialize()
            backend.clearLocalData()
            saveUnreachableHost(backend, "p3-unreachable")

            val session = backend.openSshSession("p3-unreachable", 80u, 24u)
            val deadline = System.currentTimeMillis() + 20_000
            var snapshot = session.snapshot()
            while (!snapshot.phase.isTerminal() && System.currentTimeMillis() < deadline) {
                session.awaitUpdate(snapshot.revision, 500u)
                snapshot = session.snapshot()
            }
            assertEquals(SshSessionPhase.FAILED, snapshot.phase)
            assertNotNull(snapshot.failure)

            val connectionId = session.connectionId()
            assertTrue(backend.sshSessionSnapshots().any { it.connectionId == connectionId })
            assertTrue(backend.closeSshSession(connectionId))
            assertFalse(backend.closeSshSession(connectionId))
            assertTrue(backend.sshSessionSnapshots().none { it.connectionId == connectionId })
        } finally {
            backend.shutdown()
        }
    }

    @Test
    fun openingAMissingHostIsRejectedWithoutCreatingASession() = runBlocking {
        val backend = newBackend()
        try {
            backend.initialize()
            backend.clearLocalData()
            val failure = runCatching { backend.openSshSession("p3-missing", 80u, 24u) }
            assertTrue(failure.isFailure)
            assertTrue(backend.sshSessionSnapshots().isEmpty())
        } finally {
            backend.shutdown()
        }
    }
}
