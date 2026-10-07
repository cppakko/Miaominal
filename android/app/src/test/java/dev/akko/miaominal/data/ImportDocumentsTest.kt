package dev.akko.miaominal.data

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.InputStream

/**
 * The import reader must stop at the limit instead of buffering an oversized
 * document, and must never keep reading after the limit is exceeded.
 */
class ImportDocumentsTest {

    @Test
    fun readsSmallDocumentsCompletely() {
        val content = "Host demo\n  HostName demo.example.com\n".toByteArray()
        val bytes = ImportDocuments.readAtMost(ByteArrayInputStream(content), 1024)
        assertArrayEquals(content, bytes)
    }

    @Test
    fun readsExactlyUpToTheLimit() {
        val content = ByteArray(64) { it.toByte() }
        val bytes = ImportDocuments.readAtMost(ByteArrayInputStream(content), 64)
        assertEquals(64, bytes.size)
    }

    @Test
    fun refusesDocumentsLargerThanTheLimitWithoutReadingTheRest() {
        var consumed = 0
        val source = object : InputStream() {
            override fun read(): Int = if (consumed < 4096) {
                consumed += 1
                0x61
            } else {
                -1
            }

            override fun read(b: ByteArray, off: Int, len: Int): Int {
                if (consumed >= 4096) return -1
                val count = minOf(len, 4096 - consumed)
                for (index in 0 until count) {
                    b[off + index] = 0x61
                }
                consumed += count
                return count
            }
        }

        assertThrows(ImportDocuments.TooLargeException::class.java) {
            ImportDocuments.readAtMost(source, 128)
        }
        assertEquals(
            "the reader must stop right after limit + 1 bytes",
            129,
            consumed
        )
    }

    @Test
    fun emptyDocumentsAreAllowed() {
        val bytes = ImportDocuments.readAtMost(ByteArrayInputStream(ByteArray(0)), 128)
        assertEquals(0, bytes.size)
    }

    @Test
    fun defaultLimitMatchesTheBackendContract() {
        assertEquals(8 * 1024 * 1024, ImportDocuments.MAX_IMPORT_BYTES)
    }
}
