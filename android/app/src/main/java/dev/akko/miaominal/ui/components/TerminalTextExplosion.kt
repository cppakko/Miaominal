package dev.akko.miaominal.ui.components

import android.app.SearchManager
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import dev.akko.miaominal.R
import dev.akko.miaominal.data.BackgroundPolicy
import java.text.BreakIterator
import java.util.Locale

internal data class TerminalTextToken(
    val text: String,
    val start: Int,
    val endExclusive: Int
)

private val terminalDoubleOperators = setOf("&&", "||", ">>", "<<", "==", "!=")
private val terminalSingleOperators = setOf('|', ';', '&', '>', '<', '=', '!', '(', ')')

internal fun tokenizeTerminalText(source: String): List<TerminalTextToken> {
    if (source.isBlank()) return emptyList()

    val tokens = mutableListOf<TerminalTextToken>()
    var cursor = 0
    while (cursor < source.length) {
        if (source[cursor].isWhitespace()) {
            cursor += 1
            continue
        }

        val start = cursor
        val quote = source[cursor].takeIf { it == '\'' || it == '"' }
        if (quote != null) {
            cursor += 1
            var escaped = false
            while (cursor < source.length) {
                val character = source[cursor]
                cursor += 1
                if (escaped) {
                    escaped = false
                } else if (character == '\\') {
                    escaped = true
                } else if (character == quote) {
                    break
                }
            }
            tokens += TerminalTextToken(source.substring(start, cursor), start, cursor)
            continue
        }

        val doubleOperator = source
            .substring(start, (start + 2).coerceAtMost(source.length))
            .takeIf(terminalDoubleOperators::contains)
        if (doubleOperator != null) {
            cursor += doubleOperator.length
            tokens += TerminalTextToken(doubleOperator, start, cursor)
            continue
        }

        if (source[cursor] in terminalSingleOperators) {
            cursor += 1
            tokens += TerminalTextToken(source.substring(start, cursor), start, cursor)
            continue
        }

        while (cursor < source.length && !source[cursor].isWhitespace()) {
            val possibleDouble = source
                .substring(cursor, (cursor + 2).coerceAtMost(source.length))
            if (possibleDouble in terminalDoubleOperators || source[cursor] in terminalSingleOperators) {
                break
            }
            cursor += 1
        }

        val chunk = source.substring(start, cursor)
        if (chunk.shouldUseNaturalWordBreaks()) {
            tokens += splitNaturalLanguageChunk(chunk, start)
        } else {
            tokens += TerminalTextToken(chunk, start, cursor)
        }
    }
    return tokens
}

internal fun selectedTerminalText(
    source: String,
    tokens: List<TerminalTextToken>,
    selectedIndices: Set<Int>
): String {
    val indices = selectedIndices
        .filter { it in tokens.indices }
        .sorted()
    if (indices.isEmpty()) return ""

    val ranges = buildList {
        var groupStart = indices.first()
        var groupEnd = groupStart
        for (index in indices.drop(1)) {
            if (index == groupEnd + 1) {
                groupEnd = index
            } else {
                add(groupStart..groupEnd)
                groupStart = index
                groupEnd = index
            }
        }
        add(groupStart..groupEnd)
    }

    return ranges.joinToString(" ") { range ->
        source.substring(tokens[range.first].start, tokens[range.last].endExclusive)
    }
}

private fun String.shouldUseNaturalWordBreaks(): Boolean {
    val containsCjk = any { character ->
        Character.UnicodeScript.of(character.code) in setOf(
            Character.UnicodeScript.HAN,
            Character.UnicodeScript.HIRAGANA,
            Character.UnicodeScript.KATAKANA,
            Character.UnicodeScript.HANGUL
        )
    }
    val looksTechnical = any { it in "/\\@:_-." }
    return containsCjk && !looksTechnical
}

private fun splitNaturalLanguageChunk(
    chunk: String,
    sourceOffset: Int
): List<TerminalTextToken> {
    val iterator = BreakIterator.getWordInstance(Locale.getDefault())
    iterator.setText(chunk)
    val tokens = mutableListOf<TerminalTextToken>()
    var start = iterator.first()
    var end = iterator.next()
    while (end != BreakIterator.DONE) {
        val text = chunk.substring(start, end)
        if (text.any { !it.isWhitespace() }) {
            tokens += TerminalTextToken(
                text = text,
                start = sourceOffset + start,
                endExclusive = sourceOffset + end
            )
        }
        start = end
        end = iterator.next()
    }
    return tokens
}

internal fun copyTerminalText(context: Context, text: String): Result<Unit> = runCatching {
    val clipboard = context.getSystemService(ClipboardManager::class.java)
        ?: error("Clipboard is unavailable")
    clipboard.setPrimaryClip(ClipData.newPlainText("terminal text", text))
}

internal fun searchTerminalText(context: Context, text: String): Result<Unit> {
    val policy = BackgroundPolicy.from(context)
    policy?.beginExternalFlow()
    val search = Intent(Intent.ACTION_WEB_SEARCH).apply { putExtra(SearchManager.QUERY, text) }
    val result = runCatching { context.startActivity(search) }.recoverCatching {
        context.startActivity(Intent(Intent.ACTION_VIEW,
            Uri.parse("https://www.google.com/search?q=${Uri.encode(text)}")))
    }
    result.onFailure { policy?.endExternalFlow() }
    return result
}

internal fun shareTerminalText(context: Context, text: String): Result<Unit> {
    val policy = BackgroundPolicy.from(context)
    policy?.beginExternalFlow()
    val result = runCatching {
        val share = Intent(Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_TEXT, text)
        }
        context.startActivity(Intent.createChooser(share, context.getString(R.string.terminal_explosion_share_chooser)))
    }
    result.onFailure { policy?.endExternalFlow() }
    return result
}
