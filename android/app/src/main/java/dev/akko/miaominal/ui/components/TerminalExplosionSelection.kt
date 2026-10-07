package dev.akko.miaominal.ui.components

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.state.ToggleableState
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import java.util.BitSet
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Sensitive terminal text stays in process memory, including through rotation. */
internal class TerminalTextExplosionViewModel(private val parsingDispatcher: CoroutineDispatcher = Dispatchers.Default) : ViewModel() {
    var source by mutableStateOf<String?>(null)
        private set
    var document by mutableStateOf<TerminalExplosionSelection?>(null)
        private set
    private var parsing: Job? = null

    fun open(text: String) {
        parsing?.cancel()
        source = text
        document = null
        parsing = viewModelScope.launch {
            val parsed = withContext(parsingDispatcher) { TerminalExplosionSelection(text) }
            document = parsed
        }
    }

    fun close() {
        parsing?.cancel()
        parsing = null
        source = null
        document = null
    }
}

/** Whole-word selection stays in memory and preserves source formatting when copied. */
internal class TerminalExplosionSelection(val source: String) {
    val tokens = explosionWordTokens(source, terminalCharacterTokens(source))
    private var selection = BitSet(tokens.size)
    private val history = ArrayDeque<BitSet>()
    var version by mutableIntStateOf(0)
        private set
    val canUndo: Boolean get() { version; return history.isNotEmpty() }
    val allSelected: Boolean get() { version; return tokens.isNotEmpty() && selection.cardinality() == tokens.size }
    val isEmpty: Boolean get() { version; return selection.isEmpty }
    val selectedTokenCount: Int get() { version; return selection.cardinality() }

    fun tokenState(index: Int): ToggleableState {
        version // Establish a Compose dependency on mutable BitSet changes.
        return if (index in tokens.indices && selection[index]) ToggleableState.On else ToggleableState.Off
    }

    fun checkpoint() {
        if (history.size == 20) history.removeFirst()
        history.addLast(selection.clone() as BitSet)
    }

    fun setTokenSelected(index: Int, selected: Boolean, record: Boolean = true) {
        if (index !in tokens.indices || selection[index] == selected) return
        if (record) checkpoint()
        selection.set(index, selected)
        version++
    }

    fun selectAll() {
        if (allSelected || tokens.isEmpty()) return
        checkpoint(); selection.set(0, tokens.size); version++
    }

    fun clear() {
        if (isEmpty) return
        checkpoint(); selection.clear(); version++
    }

    fun undo() {
        selection = history.removeLastOrNull() ?: return
        version++
    }

    fun resultText(): String {
        if (allSelected) return source
        return buildString {
            var start = selection.nextSetBit(0)
            while (start >= 0) {
                val end = selection.nextClearBit(start).coerceAtMost(tokens.size)
                if (isNotEmpty()) append(' ')
                append(source, tokens[start].start, tokens[end - 1].endExclusive)
                start = selection.nextSetBit(end)
            }
        }
    }
}

/** A drag toggles each visited word once, while the entire gesture remains one undo step. */
internal class TerminalExplosionDragSelection(private val document: TerminalExplosionSelection) {
    private val visited = BitSet(document.tokens.size)
    private var lastIndex: Int? = null

    fun visit(index: Int) {
        if (index !in document.tokens.indices) return
        val previous = lastIndex ?: index
        if (lastIndex == null) document.checkpoint()
        for (word in minOf(previous, index)..maxOf(previous, index)) {
            if (!visited[word]) {
                visited.set(word)
                document.setTokenSelected(word, document.tokenState(word) != ToggleableState.On, record = false)
            }
        }
        lastIndex = index
    }
}

/** Natural-language word boundaries must not split a displayed grapheme. */
private fun explosionWordTokens(source: String, characters: List<TerminalTextToken>): List<TerminalTextToken> = buildList {
    var unit = 0
    tokenizeTerminalText(source).forEach { token ->
        var end = token.endExclusive
        while (unit < characters.size && characters[unit].endExclusive <= token.start) unit++
        while (unit < characters.size && characters[unit].start < end) {
            end = maxOf(end, characters[unit].endExclusive)
            unit++
        }
        val previous = lastOrNull()
        if (previous != null && token.start < previous.endExclusive) {
            val mergedEnd = maxOf(previous.endExclusive, end)
            removeAt(lastIndex)
            add(TerminalTextToken(source.substring(previous.start, mergedEnd), previous.start, mergedEnd))
        } else add(TerminalTextToken(source.substring(token.start, end), token.start, end))
    }
}

/** Keep surrogate pairs, combining marks, emoji modifiers/ZWJ sequences and flags together. */
internal fun terminalCharacterTokens(source: String): List<TerminalTextToken> = buildList {
    var start = 0
    while (start < source.length) {
        val first = source.codePointAt(start)
        var end = start + Character.charCount(first)
        if (Character.isWhitespace(first) || Character.isSpaceChar(first)) { start = end; continue }
        var regionalCount = if (first in 0x1F1E6..0x1F1FF) 1 else 0
        var previous = first
        while (end < source.length) {
            val next = source.codePointAt(end)
            if (Character.isWhitespace(next) || Character.isSpaceChar(next)) break
            val type = Character.getType(next)
            val mark = type == Character.NON_SPACING_MARK.toInt() ||
                type == Character.COMBINING_SPACING_MARK.toInt() || type == Character.ENCLOSING_MARK.toInt()
            val regionalPair = regionalCount == 1 && next in 0x1F1E6..0x1F1FF
            if (!(mark || next in 0xFE00..0xFE0F || next in 0xE0100..0xE01EF ||
                next in 0x1F3FB..0x1F3FF || next == 0x200D || previous == 0x200D || regionalPair)) break
            end += Character.charCount(next)
            if (regionalPair) regionalCount++
            previous = next
        }
        add(TerminalTextToken(source.substring(start, end), start, end))
        start = end
    }
}
