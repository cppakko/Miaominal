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
import dev.akko.miaominal.nativebridge.SftpSessionPhase
import dev.akko.miaominal.nativebridge.SshSessionPhase
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * Resource-reclaim coverage: after 100 connect/close cycles the
 * descriptor count must return to a stable level. SSH and SFTP are both
 * exercised here; forwarding is covered by [NativeForwardTest].
 */
@RunWith(AndroidJUnit4::class)
class ResourceReclaimTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val host = AndroidPlatformHost(context)

    private fun newBackend() = Backend(host, NativeBackendClient.configFor(context))

    private suspend fun saveUnreachableHost(backend: Backend, id: String) {
        backend.saveHost(
            HostUpdate(
                id = id,
                name = "P7 unreachable",
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

    private fun SftpSessionPhase.isTerminal(): Boolean =
        this == SftpSessionPhase.CLOSED || this == SftpSessionPhase.FAILED

    @Test
    fun oneHundredSshConnectsAndClosesLeaveDescriptorsStable() = runBlocking {
        val backend = newBackend()
        try {
            backend.initialize()
            backend.clearLocalData()
            saveUnreachableHost(backend, "p7-fd-ssh")

            repeat(WARMUP_CYCLES) { sshCycle(backend) }
            val baseline = openFdCount()
            require(baseline > 0) { "descriptor counting requires /proc/self/fd" }
            var after = baseline
            repeat(CYCLES) {
                sshCycle(backend)
                after = openFdCount()
            }
            assertTrue(
                "SSH descriptors grew from $baseline to $after after $CYCLES cycles",
                after <= baseline + FD_TOLERANCE
            )
            assertTrue(backend.sshSessionSnapshots().isEmpty())
        } finally {
            backend.shutdown()
            backend.close()
        }
    }

    @Test
    fun oneHundredSftpConnectsAndClosesLeaveDescriptorsStable() = runBlocking {
        val backend = newBackend()
        try {
            backend.initialize()
            backend.clearLocalData()
            saveUnreachableHost(backend, "p7-fd-sftp")

            repeat(WARMUP_CYCLES) { sftpCycle(backend) }
            val baseline = openFdCount()
            require(baseline > 0) { "descriptor counting requires /proc/self/fd" }
            var after = baseline
            repeat(CYCLES) {
                sftpCycle(backend)
                after = openFdCount()
            }
            assertTrue(
                "SFTP descriptors grew from $baseline to $after after $CYCLES cycles",
                after <= baseline + FD_TOLERANCE
            )
            assertTrue(backend.sftpSessionSnapshots().isEmpty())
        } finally {
            backend.shutdown()
            backend.close()
        }
    }

    private suspend fun sshCycle(backend: Backend) {
        val session = backend.openSshSession("p7-fd-ssh", 80u, 24u)
        val deadline = System.currentTimeMillis() + 10_000
        var snapshot = session.snapshot()
        while (!snapshot.phase.isTerminal() && System.currentTimeMillis() < deadline) {
            session.awaitUpdate(snapshot.revision, 200u)
            snapshot = session.snapshot()
        }
        assertTrue("SSH session must reach a terminal phase", snapshot.phase.isTerminal())
        backend.closeSshSession(session.connectionId())
    }

    private suspend fun sftpCycle(backend: Backend) {
        val session = backend.openSftpSession("p7-fd-sftp")
        val deadline = System.currentTimeMillis() + 10_000
        var snapshot = session.snapshot()
        while (!snapshot.phase.isTerminal() && System.currentTimeMillis() < deadline) {
            kotlinx.coroutines.delay(50)
            snapshot = session.snapshot()
        }
        assertTrue("SFTP session must reach a terminal phase", snapshot.phase.isTerminal())
        backend.closeSftpSession(session.connectionId())
    }

    private fun openFdCount(): Int = File("/proc/self/fd").list()?.size ?: -1

    private companion object {
        const val CYCLES = 100
        const val WARMUP_CYCLES = 5
        const val FD_TOLERANCE = 8
    }
}
