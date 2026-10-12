package dev.akko.miaominal

import android.net.Uri
import android.os.Environment
import android.os.ParcelFileDescriptor
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.SafDocuments
import dev.akko.miaominal.data.SharedStorageDocuments
import dev.akko.miaominal.nativebridge.BackendException
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import java.io.File
import java.util.UUID

/** Exercise real shared storage and descriptors, including cancellation and permission loss. */
class SharedStorageDocumentsTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val documents = SafDocuments(context)
    @Suppress("DEPRECATION")
    private val root = Environment.getExternalStorageDirectory().canonicalFile
    private val rootUri = Uri.fromFile(root).toString()
    private val testDirectory = "Download/Miaominal-test-${UUID.randomUUID()}"

    @Before fun setUp() {
        assertTrue("Enable all files access before running the storage tests", Environment.isExternalStorageManager())
        documents.createDirectory(rootUri, testDirectory)
    }

    @After fun tearDown() {
        documents.delete(rootUri, testDirectory)
    }

    @Test fun listsAndReadsPhoneStorageThroughTheTransferAdapter() {
        val target = "$testDirectory/notes.txt"
        write(target, "phone storage")
        assertEquals(rootUri, documents.defaultTreeUri())
        val local = documents.listLocal(rootUri, testDirectory).single()
        assertEquals("notes.txt", local.name)
        assertEquals(target, local.relativePath)
        assertFalse(local.isDirectory)
        assertEquals(13L, local.size)
        assertEquals(target, documents.list(rootUri, testDirectory).single().relativePath)
        assertEquals(13uL, documents.stat(rootUri, target).size)
        ParcelFileDescriptor.AutoCloseInputStream(ParcelFileDescriptor.adoptFd(documents.openFile(rootUri, target))).use {
            assertEquals("phone storage", it.readBytes().toString(Charsets.UTF_8))
        }
    }

    @Test fun downloadsCommitAtomicallyAndCancellationPreservesTheOriginal() {
        val target = "$testDirectory/notes.txt"
        write(target, "original")
        val cancelled = documents.beginWrite(rootUri, target)
        ParcelFileDescriptor.AutoCloseOutputStream(ParcelFileDescriptor.adoptFd(cancelled.fd)).use {
            it.write("cancelled".toByteArray())
        }
        assertEquals("original", File(root, target).readText())
        documents.finishWrite(rootUri, target, cancelled.stagingRelative, false)
        assertEquals("original", File(root, target).readText())
        assertFalse(File(root, requireNotNull(cancelled.stagingRelative)).exists())
        write(target, "replacement")
        assertEquals("replacement", File(root, target).readText())
        assertEquals(listOf("notes.txt"), documents.listLocal(rootUri, testDirectory).map { it.name })
        val freshTarget = "$testDirectory/new.txt"
        val fresh = documents.beginWrite(rootUri, freshTarget)
        ParcelFileDescriptor.AutoCloseOutputStream(ParcelFileDescriptor.adoptFd(fresh.fd)).use { it.write(42) }
        assertFalse(File(root, freshTarget).exists())
        documents.finishWrite(rootUri, freshTarget, fresh.stagingRelative, true)
        assertTrue(File(root, freshTarget).exists())
    }

    @Test fun rejectsEscapingPathsAndRevokedAccess() {
        assertThrows(BackendException.InvalidArgument::class.java) { documents.listLocal(rootUri, "../") }
        assertThrows(BackendException.InvalidArgument::class.java) { documents.beginWrite(rootUri, "/outside.txt") }
        assertThrows(BackendException.InvalidArgument::class.java) {
            documents.listLocal(Uri.fromFile(context.filesDir).toString(), "")
        }
        val revoked = SharedStorageDocuments(accessAllowed = { false })
        assertNull(revoked.rootUri())
        assertThrows(BackendException.PermissionDenied::class.java) { revoked.listDocuments(rootUri, testDirectory) }
    }

    private fun write(target: String, body: String) {
        val start = documents.beginWrite(rootUri, target)
        ParcelFileDescriptor.AutoCloseOutputStream(ParcelFileDescriptor.adoptFd(start.fd)).use { it.write(body.toByteArray()) }
        documents.finishWrite(rootUri, target, start.stagingRelative, true)
    }

}
