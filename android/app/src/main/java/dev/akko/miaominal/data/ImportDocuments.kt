package dev.akko.miaominal.data

import java.io.InputStream

/**
 * Bounded document reads for configuration import.
 *
 * SAF providers can hand out arbitrarily large streams; the bytes must be
 * capped *while reading* so the process never materializes a file larger than
 * the import limit. The Rust side checks the size again as a second layer.
 */
object ImportDocuments {

    /** Mirrors the backend limit (`MAX_IMPORT_BYTES` in the bridge). */
    const val MAX_IMPORT_BYTES: Int = 8 * 1024 * 1024

    class TooLargeException(val limitBytes: Int) :
        IllegalArgumentException("the document exceeds the $limitBytes byte import limit")

    /**
     * Reads at most `limit` bytes; throws [TooLargeException] as soon as the
     * stream still has data after the limit instead of buffering the rest.
     */
    fun readAtMost(stream: InputStream, limit: Int = MAX_IMPORT_BYTES): ByteArray {
        require(limit > 0) { "limit must be positive" }
        val buffer = ByteArray(limit + 1)
        var total = 0
        while (total < buffer.size) {
            val read = stream.read(buffer, total, buffer.size - total)
            if (read <= 0) break
            total += read
        }
        if (total > limit) {
            throw TooLargeException(limit)
        }
        return buffer.copyOf(total)
    }
}
