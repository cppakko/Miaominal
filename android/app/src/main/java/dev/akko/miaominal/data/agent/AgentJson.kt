package dev.akko.miaominal.data.agent

import com.squareup.moshi.Moshi
import com.squareup.moshi.Types
import java.io.ByteArrayOutputStream

/**
 * JSON helpers shared by the CLI adapters.
 *
 * Moshi parses into plain maps/lists so unknown protocol fields never break an
 * adapter: unrecognized data degrades into a notice instead of an exception.
 */
class JsonCodec {
    private val moshi = Moshi.Builder().build()
    private val mapType = Types.newParameterizedType(
        Map::class.java,
        String::class.java,
        Any::class.java
    )
    private val adapter = moshi.adapter<Map<String, Any?>>(mapType)
    private val anyAdapter = moshi.adapter<Any>(Any::class.java)

    fun parse(text: String): Map<String, Any?>? = try {
        adapter.fromJson(text)
    } catch (_: Throwable) {
        null
    }

    fun stringify(value: Any?): String = anyAdapter.toJson(value)
}

/**
 * Newline-delimited JSON framing over raw stdout bytes.
 *
 * Accumulates bytes (never decoded strings) so UTF-8 sequences split across
 * chunks survive; each complete line is decoded once.
 */
class LineFramer {
    private val buffer = ByteArrayOutputStream()

    fun append(bytes: ByteArray): List<String> {
        buffer.write(bytes)
        val data = buffer.toByteArray()
        val lines = mutableListOf<String>()
        var start = 0
        for (index in data.indices) {
            if (data[index] != NEWLINE) continue
            var end = index
            if (end > start && data[end - 1] == CARRIAGE_RETURN) {
                end -= 1
            }
            if (end > start) {
                lines += String(data, start, end - start, Charsets.UTF_8)
            }
            start = index + 1
        }
        buffer.reset()
        if (start < data.size) {
            buffer.write(data, start, data.size - start)
        }
        return lines
    }

    private companion object {
        const val NEWLINE = '\n'.code.toByte()
        const val CARRIAGE_RETURN = '\r'.code.toByte()
    }
}

@Suppress("UNCHECKED_CAST")
internal fun Any?.asMap(): Map<String, Any?>? = this as? Map<String, Any?>

@Suppress("UNCHECKED_CAST")
internal fun Any?.asList(): List<Any?>? = this as? List<Any?>

internal fun Map<String, Any?>.mapAt(key: String): Map<String, Any?>? = this[key].asMap()

internal fun Map<String, Any?>.listAt(key: String): List<Any?> = this[key].asList().orEmpty()

internal fun Map<String, Any?>.stringAt(key: String): String? =
    this[key] as? String

internal fun Map<String, Any?>.longAt(key: String): Long? =
    (this[key] as? Number)?.toLong()

internal fun Map<String, Any?>.doubleAt(key: String): Double? =
    (this[key] as? Number)?.toDouble()

internal fun Map<String, Any?>.booleanAt(key: String): Boolean? =
    this[key] as? Boolean

internal fun Any?.asString(): String? = this as? String

internal fun Any?.asLong(): Long? = (this as? Number)?.toLong()

internal fun Any?.asDouble(): Double? = (this as? Number)?.toDouble()

internal fun Any?.asListOrEmpty(): List<Any?> = this.asList().orEmpty()

internal fun Any?.asMapOrEmpty(): Map<String, Any?> = this.asMap().orEmpty()
