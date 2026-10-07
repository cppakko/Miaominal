package dev.akko.miaominal

import android.os.ParcelFileDescriptor
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.AndroidPlatformHost
import dev.akko.miaominal.data.NativeBackendClient
import dev.akko.miaominal.nativebridge.AuthMethodDto
import dev.akko.miaominal.nativebridge.Backend
import dev.akko.miaominal.nativebridge.BackendException
import dev.akko.miaominal.nativebridge.CredentialAction
import dev.akko.miaominal.nativebridge.CredentialKindDto
import dev.akko.miaominal.nativebridge.HostUpdate
import dev.akko.miaominal.nativebridge.SftpSessionPhase
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * native acceptance: SFTP session lifecycle, typed failures, the
 * duplicated-descriptor registration path, and SAF error classification.
 * Remote file/directory transfers against a real server run in the Rust
 * integration tests; this suite proves the Android bridge wiring.
 */
@RunWith(AndroidJUnit4::class)
class NativeSftpTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val host = AndroidPlatformHost(context)

    private fun newBackend() = Backend(host, NativeBackendClient.configFor(context))

    private suspend fun saveUnreachableHost(backend: Backend, id: String) {
        backend.saveHost(
            HostUpdate(
                id = id,
                name = "P4 unreachable",
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

    private fun SftpSessionPhase.isTerminal(): Boolean =
        this == SftpSessionPhase.CLOSED || this == SftpSessionPhase.FAILED

    @Test
    fun unreachableHostFailsThenClosesIdempotently() = runBlocking {
        val backend = newBackend()
        try {
            backend.initialize()
            backend.clearLocalData()
            saveUnreachableHost(backend, "p4-unreachable")

            val session = backend.openSftpSession("p4-unreachable")
            val deadline = System.currentTimeMillis() + 20_000
            var snapshot = session.snapshot()
            while (!snapshot.phase.isTerminal() && System.currentTimeMillis() < deadline) {
                Thread.sleep(100)
                snapshot = session.snapshot()
            }
            assertEquals(SftpSessionPhase.FAILED, snapshot.phase)
            assertNotNull(snapshot.failure)

            val connectionId = session.connectionId()
            assertTrue(backend.sftpSessionSnapshots().any { it.connectionId == connectionId })
            assertTrue(backend.closeSftpSession(connectionId))
            assertFalse(backend.closeSftpSession(connectionId))
            assertTrue(backend.sftpSessionSnapshots().none { it.connectionId == connectionId })
        } finally {
            backend.shutdown()
        }
    }

    @Test
    fun listingBeforeReadyAndStaleAnswersAreRejected() = runBlocking {
        val backend = newBackend()
        try {
            backend.initialize()
            backend.clearLocalData()
            saveUnreachableHost(backend, "p4-unreachable")

            val session = backend.openSftpSession("p4-unreachable")
            val listingFailure = runCatching { session.listDirectory("/") }
            assertTrue(listingFailure.isFailure)
            assertTrue(listingFailure.exceptionOrNull() is BackendException.SessionNotReady)

            val answerFailure = runCatching { session.answerKeyboardInteractive(7u, listOf("x")) }
            assertTrue(answerFailure.isFailure)
            assertTrue(answerFailure.exceptionOrNull() is BackendException.RequestExpired)

            assertTrue(backend.closeSftpSession(session.connectionId()))
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
            val failure = runCatching { backend.openSftpSession("p4-missing") }
            assertTrue(failure.isFailure)
            assertTrue(backend.sftpSessionSnapshots().isEmpty())
        } finally {
            backend.shutdown()
        }
    }

    @Test
    fun registeredDescriptorsAreDuplicatedAndReleasedIdempotently() = runBlocking {
        val backend = newBackend()
        val scratch = File(context.cacheDir, "p4-fd-test")
        try {
            backend.initialize()
            backend.clearLocalData()
            scratch.mkdirs()
            val file = File(scratch, "payload.bin")
            file.writeBytes(ByteArray(4096) { 7 })

            // Warm up the descriptor counter so the listing itself is counted.
            openFdCount()

            val descriptor = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
            val before = openFdCount()
            val handle = backend.registerLocalFile(descriptor.fd, "payload.bin")
            descriptor.close()
            val whileRegistered = openFdCount()
            assertTrue(
                "registration must duplicate the descriptor (before=$before, after=$whileRegistered)",
                whileRegistered >= before
            )
            assertEquals(4096uL, handle.size())

            assertTrue(handle.release())
            assertFalse(handle.release())
            val after = openFdCount()
            assertTrue(
                "closing twice must not leak descriptors (before=$before, after=$after)",
                after <= before + 1
            )
        } finally {
            backend.shutdown()
            scratch.deleteRecursively()
        }
    }

    @Test
    fun invalidDescriptorsAreRejectedWithoutCrashing() = runBlocking {
        val backend = newBackend()
        try {
            backend.initialize()
            backend.clearLocalData()
            val failure = runCatching { backend.registerLocalFile(-1, "nope.bin") }
            assertTrue(failure.isFailure)
            val blank = runCatching { backend.registerLocalFile(0, " ") }
            assertTrue(blank.isFailure)
        } finally {
            backend.shutdown()
        }
    }

    @Test
    fun safOperationsRejectUnauthorizedTrees() {
        val failure = runCatching { host.safList("not-a-tree-uri", "") }
        assertTrue(failure.isFailure)
        assertTrue(failure.exceptionOrNull() is BackendException.InvalidArgument)

        val statFailure = runCatching { host.safStat("not-a-tree-uri", "file.txt") }
        assertTrue(statFailure.isFailure)
        val writeFailure = runCatching { host.safBeginWrite("not-a-tree-uri", "file.txt") }
        assertTrue(writeFailure.isFailure)
    }

    private fun openFdCount(): Int =
        File("/proc/self/fd").list()?.size ?: -1
}
